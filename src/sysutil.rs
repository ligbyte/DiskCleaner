//! 系统环境相关的工具函数：路径解析、权限检测、磁盘容量、文件管理器定位。
//!
//! 这一层是整个程序里唯一直接依赖操作系统的模块。上层（清理目标、扫描、清理、
//! 回收站、软件列表）都只调用这里暴露的抽象，因此新增一个平台时只需要：
//!
//! 1. 在 [`crate::targets`] 里加一份该平台的清理目标白名单；
//! 2. 在本文件里补上该平台的路径 / 容量 / 权限 / 打开文件管理器实现；
//! 3. 在 [`crate::recycle`] 里补上该平台的"移入回收站"实现。
//!
//! 目前支持 Windows、macOS 与 Linux（XDG）。

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::model::DiskInfo;

// ---------------------------------------------------------------------------
// 平台标识
// ---------------------------------------------------------------------------

/// 目标平台（编译期确定，界面与日志都要用）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Windows,
    MacOs,
    Linux,
    Other,
}

pub const PLATFORM: Platform = if cfg!(windows) {
    Platform::Windows
} else if cfg!(target_os = "macos") {
    Platform::MacOs
} else if cfg!(target_os = "linux") {
    Platform::Linux
} else {
    Platform::Other
};

impl Platform {
    /// 界面 / 日志里显示的短名。
    pub fn label(self) -> &'static str {
        match self {
            Platform::Windows => "Windows",
            Platform::MacOs => "macOS",
            Platform::Linux => "Linux",
            Platform::Other => "未知系统",
        }
    }

    /// 这个平台是否支持"移入回收站"。
    pub fn has_recycle_bin(self) -> bool {
        matches!(self, Platform::Windows | Platform::MacOs | Platform::Linux)
    }
}

// ---------------------------------------------------------------------------
// 环境变量与目录
// ---------------------------------------------------------------------------

/// 读取环境变量并转成路径。
pub fn env_path(key: &str) -> Option<PathBuf> {
    let value = std::env::var_os(key)?;
    if value.is_empty() {
        None
    } else {
        Some(PathBuf::from(value))
    }
}

/// 当前用户的主目录。
///
/// * Windows：`USERPROFILE`
/// * macOS / Linux：`HOME`
///
/// 主目录是「大文件」页的默认扫描目录，也是安全层的用户资料目录基准点，
/// 所以取不到时宁可返回 `None` 让上层明确处理，也不要瞎猜一个路径。
pub fn home_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        env_path("USERPROFILE").or_else(|| {
            // 少数精简系统没有 USERPROFILE，用 HOMEDRIVE + HOMEPATH 拼
            let drive = env_path("HOMEDRIVE")?;
            let path = env_path("HOMEPATH")?;
            Some(
                drive.join(
                    path.to_string_lossy()
                        .trim_start_matches('\\')
                        .trim_start_matches('/'),
                ),
            )
        })
    }
    #[cfg(unix)]
    {
        env_path("HOME")
    }
}

/// 临时目录（优先使用环境变量，避免与 `fs::canonicalize` 的长路径前缀打架）。
pub fn temp_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        env_path("TEMP").or_else(|| env_path("TMP"))
    }
    #[cfg(unix)]
    {
        // TMPDIR 是 POSIX 约定，取不到时回落到系统的 /tmp
        env_path("TMPDIR").or_else(|| Some(std::env::temp_dir()))
    }
}

#[cfg(windows)]
pub fn windows_dir() -> PathBuf {
    env_path("SystemRoot")
        .or_else(|| env_path("windir"))
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
}

#[cfg(windows)]
pub fn program_data() -> PathBuf {
    env_path("ProgramData").unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"))
}

#[cfg(windows)]
pub fn local_app_data() -> Option<PathBuf> {
    env_path("LOCALAPPDATA")
}

#[cfg(windows)]
pub fn roaming_app_data() -> Option<PathBuf> {
    env_path("APPDATA")
}

/// macOS 的 `~/Library`。
#[cfg(target_os = "macos")]
pub fn mac_library() -> Option<PathBuf> {
    home_dir().map(|home| home.join("Library"))
}

/// macOS 的 `~/Library/Application Support`。
#[cfg(target_os = "macos")]
pub fn application_support() -> Option<PathBuf> {
    mac_library().map(|lib| lib.join("Application Support"))
}

/// macOS 的 `~/Library/Caches`。
#[cfg(target_os = "macos")]
pub fn user_cache_dir() -> Option<PathBuf> {
    mac_library().map(|lib| lib.join("Caches"))
}

/// Linux 的 `$XDG_CACHE_HOME`（缺省 `~/.cache`）。
#[cfg(target_os = "linux")]
pub fn user_cache_dir() -> Option<PathBuf> {
    env_path("XDG_CACHE_HOME").or_else(|| home_dir().map(|home| home.join(".cache")))
}

/// Linux 的 `$XDG_CONFIG_HOME`（缺省 `~/.config`）。
#[cfg(target_os = "linux")]
pub fn user_config_dir() -> Option<PathBuf> {
    env_path("XDG_CONFIG_HOME").or_else(|| home_dir().map(|home| home.join(".config")))
}

/// Linux 的 `$XDG_DATA_HOME`（缺省 `~/.local/share`）。
#[cfg(target_os = "linux")]
pub fn user_data_dir() -> Option<PathBuf> {
    env_path("XDG_DATA_HOME").or_else(|| home_dir().map(|home| home.join(".local").join("share")))
}

/// 用户的「文档」目录（QQ / 微信的数据目录默认就在它下面）。
///
/// Windows 上「文档」可以被重定向到其它盘，严格做法要读 shell 元数据；
/// 这里按惯例取 `USERPROFILE\Documents`，绝大多数机器上是准的，取不到
/// 返回 `None`，让上层跳过对应目标即可。
pub fn documents_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        env_path("USERPROFILE").map(|home| home.join("Documents"))
    }
    #[cfg(not(windows))]
    {
        env_path("XDG_DOCUMENTS_DIR").or_else(|| home_dir().map(|home| home.join("Documents")))
    }
}

/// 应用自己的数据目录。
///
/// * Windows：`%LOCALAPPDATA%\CleanC`
/// * macOS：`~/Library/Application Support/CleanC`
/// * Linux：`$XDG_DATA_HOME/cleanc`（缺省 `~/.local/share/cleanc`）
///
/// 全都取不到时退回到系统临时目录，保证程序永远能启动。
pub fn app_data_dir() -> PathBuf {
    #[cfg(windows)]
    let base = local_app_data();
    #[cfg(target_os = "macos")]
    let base = application_support();
    #[cfg(target_os = "linux")]
    let base = user_data_dir();
    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
    let base: Option<PathBuf> = None;

    base.unwrap_or_else(std::env::temp_dir).join("CleanC")
}

/// 设置文件的存放位置（界面「设置 → 数据与日志」会原样展示）。
pub fn settings_path() -> PathBuf {
    app_data_dir().join("settings.json")
}

pub fn logs_dir() -> PathBuf {
    app_data_dir().join("logs")
}

/// 系统根目录所在位置：Windows 是系统盘盘符，Unix 是 `/`。
///
/// 「概览」页与状态栏显示的磁盘容量就取这个位置。
pub fn default_mount() -> String {
    #[cfg(windows)]
    {
        // 系统盘不一定叫 C:，从 SystemRoot 里取盘符更稳妥
        let root = windows_dir();
        if let Some(first) = root.components().next() {
            let text = first.as_os_str().to_string_lossy().to_string();
            if !text.is_empty() {
                return format!("{text}\\");
            }
        }
        r"C:\".to_string()
    }
    #[cfg(not(windows))]
    {
        "/".to_string()
    }
}

/// 展示磁盘时用的友好名字（`C:\` → `C 盘`，`/` → `根分区`）。
pub fn mount_label(mount: &str) -> String {
    let trimmed = mount.trim_end_matches(['\\', '/']);
    let bytes = trimmed.as_bytes();
    if bytes.len() == 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return format!("{} 盘", (bytes[0] as char).to_ascii_uppercase());
    }
    if trimmed.is_empty() {
        return "根分区".to_string();
    }
    trimmed.to_string()
}

/// 读取卷标（Windows 里 `C:` 盘可能叫「系统」「Windows」）。
///
/// 只有 Windows 有这个概念；macOS / Linux 返回 `None`，调用方退回 [`mount_label`]。
#[cfg(windows)]
pub fn volume_label(mount: &str) -> Option<String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetVolumeInformationW;

    // 只认盘符根（`C:\`）。这块守卫不能省：把 `/` 或 `C:\Users` 这类路径传进去，
    // API 不会报错，而是把**当前卷**的卷标返回来（实测传 "/" 会拿到本机 C 盘的
    // 「Samsung」），调用方根本无从察觉。
    let trimmed = mount.trim_end_matches(['\\', '/']);
    let bytes = trimmed.as_bytes();
    if !(bytes.len() == 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':') {
        return None;
    }

    let wide: Vec<u16> = std::ffi::OsStr::new(mount)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    // 卷标上限是 MAX_PATH + 1
    let mut name = [0u16; 261];
    // SAFETY: `wide` 是以 NUL 结尾的合法宽字符串；`name` 是栈上的可写缓冲区，
    // 长度按**元素个数**传入（不是字节数）；其余出参不需要，传空指针即可。
    let ok = unsafe {
        GetVolumeInformationW(
            wide.as_ptr(),
            name.as_mut_ptr(),
            name.len() as u32,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            0,
        )
    };
    if ok == 0 {
        return None;
    }

    let len = name.iter().position(|&c| c == 0).unwrap_or(0);
    let text = String::from_utf16_lossy(&name[..len]);
    let text = text.trim().to_string();
    if text.is_empty() { None } else { Some(text) }
}

/// 非 Windows 平台没有「卷标」这个概念。
///
/// 留在跨平台编译面里（而不是加 `cfg` 到调用点），是为了让 [`disk_title`]
/// 不用写两套分支。
#[cfg(not(windows))]
pub fn volume_label(_mount: &str) -> Option<String> {
    None
}

/// 磁盘卡片的标题，参考系统资源管理器的写法：
///
/// * Windows：`系统 (C:)` / `本地磁盘 (D:)`；
/// * macOS / Linux：`根分区 (/)`。
///
/// 括号里是挂载点本身 —— 用户能拿它跟系统里的盘符对上号。
pub fn disk_title(mount: &str) -> String {
    let trimmed = mount.trim_end_matches(['\\', '/']);
    let bytes = trimmed.as_bytes();
    // 盘符统一大写，跟资源管理器一致（`d:\` → `D:`）
    let short = if bytes.len() == 2 && bytes[1] == b':' {
        trimmed.to_ascii_uppercase()
    } else if trimmed.is_empty() {
        mount.to_string()
    } else {
        trimmed.to_string()
    };
    let name = volume_label(mount).unwrap_or_else(|| {
        if cfg!(windows) {
            "本地磁盘".to_string()
        } else {
            mount_label(mount)
        }
    });
    format!("{name} ({short})")
}

/// 确保目录存在，失败时返回 `false`（不 panic）。
pub fn ensure_dir(path: &Path) -> bool {
    if path.is_dir() {
        return true;
    }
    std::fs::create_dir_all(path).is_ok()
}

// ---------------------------------------------------------------------------
// 权限
// ---------------------------------------------------------------------------

/// 当前进程是否拥有管理员 / root 权限。
///
/// * Windows：试探往 `%SystemRoot%\Temp` 写一个临时文件，能写成功说明具备提权能力。
/// * Unix：直接读有效用户 id，`0` 就是 root。
pub fn is_admin() -> bool {
    #[cfg(windows)]
    {
        let probe = windows_dir()
            .join("Temp")
            .join(format!(".cleanc_admin_probe_{}", std::process::id()));
        match std::fs::File::create(&probe) {
            Ok(_) => {
                let _ = std::fs::remove_file(&probe);
                true
            }
            Err(_) => false,
        }
    }
    #[cfg(unix)]
    {
        // SAFETY: geteuid 无参数、无副作用，任何线程任何时刻调用都安全。
        unsafe { libc::geteuid() == 0 }
    }
}

/// 界面上"管理员权限"提示对应的权限名词。
pub fn privilege_label() -> &'static str {
    #[cfg(windows)]
    {
        "管理员"
    }
    #[cfg(not(windows))]
    {
        "root"
    }
}

/// 普通权限时提示用户该怎么提权。
pub fn elevation_hint() -> &'static str {
    match PLATFORM {
        Platform::Windows => "以管理员身份运行可以清理系统级目录（系统更新缓存、系统临时文件等）",
        Platform::MacOs => "系统级目录（/Library/Caches、/var/log 等）需要 sudo 才能清理",
        Platform::Linux => "系统级目录（/var/cache、/var/log 等）需要 sudo 才能清理",
        Platform::Other => "部分系统目录需要更高的权限才能清理",
    }
}

// ---------------------------------------------------------------------------
// 磁盘容量
// ---------------------------------------------------------------------------

/// 查询指定挂载点的容量信息。
#[cfg(windows)]
pub fn disk_info(mount: &str) -> Option<DiskInfo> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

    let wide: Vec<u16> = std::ffi::OsStr::new(mount)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    let mut available_to_caller: u64 = 0;
    let mut total: u64 = 0;
    let mut free: u64 = 0;

    // SAFETY: 传入的都是栈上有效指针，`wide` 是合法的以 NUL 结尾的宽字符串。
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut available_to_caller,
            &mut total,
            &mut free,
        )
    };

    if ok == 0 || total == 0 {
        return None;
    }

    Some(DiskInfo {
        mount: mount.to_string(),
        total,
        free,
        available: available_to_caller,
    })
}

/// 查询指定挂载点的容量信息（`statvfs`，macOS 与 Linux 通用）。
#[cfg(unix)]
pub fn disk_info(mount: &str) -> Option<DiskInfo> {
    let path = std::ffi::CString::new(mount).ok()?;
    // SAFETY: `statvfs` 只写入我们提供的这块栈内存；`path` 是合法的 C 字符串。
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: 同上，指针与结构体生命周期都覆盖整个调用。
    let rc = unsafe { libc::statvfs(path.as_ptr(), &mut st) };
    if rc != 0 {
        return None;
    }

    // f_frsize 是"基本块大小"，优先用它；个别文件系统会给 0，退回 f_bsize
    let unit = if st.f_frsize > 0 {
        st.f_frsize as u64
    } else {
        st.f_bsize as u64
    };
    let total = (st.f_blocks as u64).saturating_mul(unit);
    if total == 0 {
        return None;
    }
    let free = (st.f_bfree as u64).saturating_mul(unit);
    let available = (st.f_bavail as u64).saturating_mul(unit);

    Some(DiskInfo {
        mount: mount.to_string(),
        total,
        free,
        available,
    })
}

#[cfg(not(any(windows, unix)))]
pub fn disk_info(mount: &str) -> Option<DiskInfo> {
    let _ = mount;
    None
}

// ---------------------------------------------------------------------------
// 文件管理器
// ---------------------------------------------------------------------------

/// 在系统的文件管理器里定位（选中）某个文件或目录。
pub fn reveal_in_file_manager(path: &Path) {
    #[cfg(windows)]
    {
        if path.is_dir() {
            let _ = Command::new("explorer.exe").arg(path).spawn();
            return;
        }
        // `/select,` 后的路径不能加引号，否则 explorer 会解析失败
        let mut arg = String::from("/select,");
        arg.push_str(&path.to_string_lossy());
        let _ = Command::new("explorer.exe").arg(arg).spawn();
    }

    #[cfg(target_os = "macos")]
    {
        // `open -R` 就是 Finder 的「在访达中显示」
        let _ = Command::new("open").arg("-R").arg(path).spawn();
    }

    #[cfg(target_os = "linux")]
    {
        // 首选 FileManager1 D-Bus 接口（能真正高亮条目）；
        // 没有就退回到直接打开所在目录，保证总是有反馈。
        let parent = if path.is_dir() {
            path
        } else {
            path.parent().unwrap_or(path)
        };
        let target = format!("file://{}", path.to_string_lossy());
        let ok = Command::new("dbus-send")
            .args([
                "--session",
                "--dest=org.freedesktop.FileManager1",
                "--type=method_call",
                "/org/freedesktop/FileManager1",
                "org.freedesktop.FileManager1.ShowItems",
                &format!("array:string:{target}"),
                "string:",
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !ok {
            let _ = Command::new("xdg-open").arg(parent).spawn();
        }
    }

    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
    {
        let _ = path;
    }
}

/// 用系统默认方式打开路径。
pub fn open_path(path: &Path) {
    #[cfg(windows)]
    {
        let _ = Command::new("explorer.exe").arg(path).spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = Command::new("open").arg(path).spawn();
    }
    #[cfg(target_os = "linux")]
    {
        let _ = Command::new("xdg-open").arg(path).spawn();
    }
    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
    {
        let _ = path;
    }
}

/// 用系统默认方式打开一个网址（保留给"查看日志/文档"这类入口）。
pub fn open_url(url: &str) {
    #[cfg(windows)]
    {
        // explorer.exe 对 URL 的处理比 `cmd /C start` 干净
        let _ = Command::new("explorer.exe").arg(url).spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = Command::new("open").arg(url).spawn();
    }
    #[cfg(target_os = "linux")]
    {
        let _ = Command::new("xdg-open").arg(url).spawn();
    }
    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
    {
        let _ = url;
    }
}

/// 单次统计允许访问的最大条目数，防止某个异常目录把界面卡死。
const MAX_WALK_ENTRIES: usize = 400_000;

/// 递归累加目录下所有文件的大小。
///
/// * 不跟随符号链接 / 目录联接，避免走进环或者把别的盘算进来；
/// * 只做元数据读取，不打开文件内容，因此速度很快。
pub fn dir_size(root: &Path) -> u64 {
    let mut total = 0u64;
    let mut visited = 0usize;
    let mut stack: Vec<PathBuf> = vec![root.to_path_buf()];

    while let Some(dir) = stack.pop() {
        let Ok(reader) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in reader.flatten() {
            visited += 1;
            if visited > MAX_WALK_ENTRIES {
                return total;
            }
            let Ok(ftype) = entry.file_type() else {
                continue;
            };
            if ftype.is_symlink() {
                continue;
            }
            if ftype.is_dir() {
                stack.push(entry.path());
                continue;
            }
            // 文件：直接用目录项自带的元数据，省一次系统调用
            if let Ok(meta) = entry.metadata() {
                total += meta.len();
            }
        }
    }

    total
}

/// 统计目录下的**一级**条目数量（用于"回收站里有多少个东西"）。
pub fn count_top_level(dir: &Path) -> u64 {
    let Ok(reader) = std::fs::read_dir(dir) else {
        return 0;
    };
    reader.flatten().count() as u64
}

// ---------------------------------------------------------------------------
// 杂项
// ---------------------------------------------------------------------------

/// 是否应该跳过这个文件名 —— 回收站自身的元数据文件。
///
/// Windows 的 `$Recycle.Bin` 里，`$I*`（元数据）与 `$R*`（真实内容）成对出现；
/// 扫描时把 `$I*` 当成可清理对象没有意义。macOS / Linux 的回收站把元数据放在
/// 独立的 `info/` 或 `.DS_Store` 之类的文件里，不靠文件名前缀区分，返回 `false`。
pub fn is_recycle_metadata(name: &str) -> bool {
    #[cfg(windows)]
    {
        let lower = name.to_ascii_lowercase();
        lower.starts_with("$i") || lower.starts_with("$r")
    }
    #[cfg(not(windows))]
    {
        let _ = name;
        false
    }
}

/// 回收站 / 垃圾桶在界面上的叫法。
pub fn trash_label() -> &'static str {
    match PLATFORM {
        Platform::MacOs => "废纸篓",
        _ => "回收站",
    }
}

// ---------------------------------------------------------------------------
// 测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_dir_is_absolute_when_present() {
        if let Some(home) = home_dir() {
            assert!(home.is_absolute(), "{} 应当是绝对路径", home.display());
        }
    }

    #[test]
    fn app_data_dir_is_absolute() {
        assert!(app_data_dir().is_absolute());
    }

    #[test]
    fn default_mount_is_recognizable() {
        let mount = default_mount();
        assert!(!mount.is_empty());
        if cfg!(windows) {
            assert!(mount.ends_with('\\'), "{mount}");
        } else {
            assert_eq!(mount, "/");
        }
    }

    #[test]
    fn mount_label_is_human_readable() {
        assert_eq!(mount_label(r"C:\"), "C 盘");
        assert_eq!(mount_label(r"d:\"), "D 盘");
        assert_eq!(mount_label("/"), "根分区");
    }

    #[test]
    fn disk_title_carries_the_mount_point() {
        // 括号里那份一定是挂载点本身，用户拿它跟系统里的盘符对号
        assert!(
            disk_title(r"C:\").ends_with("(C:)"),
            "{}",
            disk_title(r"C:\")
        );
        assert!(
            disk_title(r"d:\").ends_with("(D:)"),
            "{}",
            disk_title(r"d:\")
        );
        assert!(disk_title("/").ends_with("(/)"), "{}", disk_title("/"));

        // 名字那半截：unix 没有卷标概念，固定用挂载点的叫法
        if !cfg!(windows) {
            assert_eq!(disk_title("/"), "根分区 (/)");
        }

        // 真机上的系统盘也要能组出合理标题
        let real = disk_title(&default_mount());
        assert!(real.ends_with(')'), "{real}");
    }

    #[test]
    fn volume_label_only_answers_for_drive_roots() {
        // 非盘符根一律不查卷标：Windows 的 GetVolumeInformationW 传进 `/`
        // 不会报错，而是把**当前卷**的卷标返回来，是个很容易误用的坑
        assert_eq!(volume_label("/"), None);
        assert_eq!(volume_label(r"C:\Users"), None);
    }

    #[test]
    fn disk_info_reads_something_real() {
        // 系统盘总能读出容量；读不出来说明平台实现有问题
        let info = disk_info(&default_mount()).expect("系统盘容量应当可读");
        assert!(info.total > 0);
        assert!(info.used() <= info.total);
    }
}
