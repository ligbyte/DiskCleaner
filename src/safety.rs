//! 安全防护层。
//!
//! 这是整个工具"敢删东西"的底气所在，所有删除动作都必须先过这一层。
//! 设计原则是 **默认拒绝**：
//!
//! 1. 只有落在明确声明的白名单根目录内的条目才允许删除；
//! 2. 命中关键系统目录 / 关键文件名的条目一律拒绝；
//! 3. 路径中不允许出现 `..`，避免目录穿越；
//! 4. 符号链接与目录联接一律不跟随、不删除；
//! 5. 白名单根目录本身永远保留，只清理其内容。
//!
//! 保护名单按平台分别维护（见 [`protected_trees`] / [`protected_user_subdirs`]），
//! 路径规范化也随平台的路径规则走（Windows 反斜杠 + 大小写不敏感，
//! macOS 斜杠 + 大小写不敏感，Linux 斜杠 + 大小写敏感）。

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, SystemTime};

// ---------------------------------------------------------------------------
// 平台路径规则
// ---------------------------------------------------------------------------

/// 该平台的主路径分隔符。
pub const SEP: char = if cfg!(windows) { '\\' } else { '/' };

/// 路径比较时是否忽略大小写（Windows 与 macOS 默认文件系统都不区分大小写）。
const CASE_INSENSITIVE: bool = cfg!(any(windows, target_os = "macos"));

/// 规范化后路径的最小长度：Windows 的 `c:\` 就是 3 个字符，Unix 的 `/` 只有 1 个。
const MIN_PATH_LEN: usize = if cfg!(windows) { 4 } else { 2 };

// ---------------------------------------------------------------------------
// 保护名单
// ---------------------------------------------------------------------------

/// 与平台无关、写死在代码里的保护目录树（规范化、小写）。
///
/// 注意：这是"兜底"防线。真正的第一道防线是"只允许白名单根目录内的路径"，
/// 因此即使某个清理目标的路径配错了，这里也能拦住最危险的情况。
const PROTECTED_TREES_STATIC: &[&str] = &[
    // --- Windows 核心（盘符与系统目录前缀会在 [`protected_trees`] 里动态补全）---
    r"\windows\system32",
    r"\windows\syswow64",
    r"\windows\winsxs",
    r"\windows\assembly",
    r"\windows\servicing",
    r"\windows\boot",
    r"\windows\systemapps",
    r"\windows\systemresources",
    r"\windows\security",
    r"\windows\ime",
    r"\windows\fonts",
    r"\windows\inf",
    r"\windows\speech",
    r"\windows\csc",
    r"\windows\system32\config",
    r"\windows\system32\drivers",
    r"\windows\system32\driverstore",
    r"\windows\system32\catroot",
    r"\windows\system32\catroot2",
    r"\windows\system32\spool",
    r"\windows\system32\wbem",
    r"\windows\system32\winevt",
    r"\windows\system32\msdtc",
    r"\windows\system32\oobe",
    r"\windows\system32\smi",
    r"\windows\system32\licensing",
    r"\windows\system32\migration",
    r"\windows\system32\recovery",
    r"\windows\system32\advancedinstallers",
    r"\windows\system32\logfiles",
    r"\windows\microsoft.net",
    r"\windows\registration",
    r"\windows\schemas",
    r"\windows\policydefinitions",
    r"\windows\globalization",
    r"\windows\provisioning",
    r"\windows\diagnostics",
    r"\windows\help",
    // 注意：这里**故意没有** `\windows` 本身 ——
    // `C:\Windows\Temp`、`C:\Windows\Logs`、`C:\Windows\Prefetch`、
    // `C:\Windows\SoftwareDistribution\Download`、`C:\Windows\Minidump`
    // 都是要清理的目标，把整棵 `C:\Windows` 拉黑会让它们一个文件都删不掉。
    // --- 已安装程序 ---
    r"\program files",
    r"\program files (x86)",
    // --- 系统维护数据 ---
    r"\$recycle.bin",
    r"\system volume information",
    r"\recovery",
    r"\programdata\package cache",
    r"\programdata\microsoft\crypto",
    r"\programdata\microsoft\windows\systemdata",
    // --- macOS 核心（`/library/caches` 与 `/var/log` 不在名单里，它们是可清理目标）---
    "/system",
    "/usr",
    "/bin",
    "/sbin",
    "/opt",
    "/etc",
    "/private/etc",
    "/var/db",
    "/private/var/db",
    "/library/application support",
    "/library/preferences",
    "/library/frameworks",
    "/library/launchagents",
    "/library/launchdaemons",
    "/library/extensions",
    "/library/privilegedhelpertools",
    "/library/keychains",
    "/library/security",
    "/library/startupitems",
    "/library/coreservices",
    "/library/receipts",
    "/library/updates",
    "/applications",
    "/cores",
    // --- Linux 核心（`/var/cache`、`/var/log`、`/var/tmp` 不在名单里）---
    "/boot",
    "/proc",
    "/sys",
    "/dev",
    "/run",
    "/root",
    "/srv",
    "/lib",
    "/lib32",
    "/lib64",
    "/libx32",
    "/var/lib",
    "/var/spool",
    "/lost+found",
];

/// 绝对不允许删除的文件名（全小写，任意位置）。
const PROTECTED_FILE_NAMES: &[&str] = &[
    // Windows
    "desktop.ini",
    "pagefile.sys",
    "hiberfil.sys",
    "swapfile.sys",
    "bootmgr",
    "bootnxt",
    "ntldr",
    "bcd",
    "ntuser.dat",
    "ntuser.ini",
    "boot.ini",
    "autoexec.bat",
    "config.sys",
    // macOS / Linux：这几类文件一旦被清理，后果是"进不去系统"或"登不进去"
    "fstab",
    "passwd",
    "shadow",
    "sudoers",
    "authorized_keys",
    "id_rsa",
    "id_ed25519",
    "known_hosts",
];

/// 用户目录下绝对不允许触碰的子目录（相对当前用户主目录，使用平台分隔符）。
#[allow(dead_code)]
const PROTECTED_USER_SUBDIRS_WINDOWS: &[&str] = &[
    "documents",
    "desktop",
    "downloads",
    "pictures",
    "videos",
    "music",
    "onedrive",
    "favorites",
    "saved games",
    "searches",
    "links",
    "contacts",
    "3d objects",
    r"appdata\roaming\microsoft\protect",
    r"appdata\roaming\microsoft\system certificates",
    r"appdata\roaming\microsoft\crypto",
    r"appdata\roaming\microsoft\signatures",
    r"appdata\local\microsoft\credentials",
    r"appdata\local\microsoft\windows\history",
    r"appdata\local\microsoft\windows\webcache",
];

/// macOS：主目录下要保护的内容。
///
/// `~/Library/Caches`、`~/Library/Logs` 与 `~/Library/Developer/Xcode/DerivedData`
/// 是**可清理目标**，所以不能把整个 `~/Library` 拉黑，只能逐个列出真正敏感的子树。
#[allow(dead_code)]
const PROTECTED_USER_SUBDIRS_MACOS: &[&str] = &[
    "desktop",
    "documents",
    "downloads",
    "pictures",
    "movies",
    "music",
    "public",
    // 应用数据与偏好设置
    "library/application support",
    "library/preferences",
    "library/keychains",
    "library/accounts",
    "library/calendars",
    "library/contacts",
    "library/mail",
    "library/messages",
    "library/safari",
    "library/cookies",
    "library/containers",
    "library/group containers",
    "library/application scripts",
    "library/mobile documents",
    "library/saved application state",
    "library/launchagents",
    "library/services",
    "library/internet plugins",
    "library/input methods",
    "library/audio",
    "library/fonts",
    "library/colorsync",
    // 开发与系统
    "library/developer/xcode/archives",
    "library/developer/xcode/userdata",
    "library/developer/coresimulator/devices",
    "library/developer/coresimulator/runtimes",
    ".ssh",
    ".gnupg",
    ".config",
    ".local",
];

/// Linux（XDG）：主目录下要保护的内容。
///
/// 注意**不包含** `~/.cache`（头号清理目标）与 `~/.local/share/Trash`（回收站目标），
/// 所以这里只列 `.local/share` 下真正敏感的那几个子目录。
#[allow(dead_code)]
const PROTECTED_USER_SUBDIRS_LINUX: &[&str] = &[
    "documents",
    "desktop",
    "downloads",
    "pictures",
    "videos",
    "music",
    "public",
    "templates",
    ".ssh",
    ".gnupg",
    ".config",
    ".mozilla",
    ".thunderbird",
    ".wine",
    ".local/share/keyrings",
    ".local/share/gnupg",
    ".local/share/kwalletd",
    ".local/share/passwords",
    ".local/share/applications",
];

/// 删除被拒绝的原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reject {
    /// 路径为空或无效
    EmptyPath,
    /// 就是文件系统的根（`C:\`、`/`）
    FsRoot,
    /// 路径中包含 `..`，疑似目录穿越
    ParentComponent,
    /// 命中保护目录树
    ProtectedTree,
    /// 命中保护文件名
    ProtectedFileName,
    /// 不在允许清理的根目录内
    OutsideRoot,
    /// 就是白名单根目录本身（只清内容，不删目录）
    RootItself,
    /// 符号链接 / 目录联接
    SymbolicLink,
}

impl Reject {
    pub fn reason(self) -> &'static str {
        match self {
            Reject::EmptyPath => "路径无效",
            Reject::FsRoot => "是文件系统根目录",
            Reject::ParentComponent => "路径包含 .. 片段",
            Reject::ProtectedTree => "位于受保护的系统目录",
            Reject::ProtectedFileName => "属于受保护的关键文件",
            Reject::OutsideRoot => "不在允许清理的目录内",
            Reject::RootItself => "是清理根目录本身",
            Reject::SymbolicLink => "是符号链接或目录联接",
        }
    }
}

// ---------------------------------------------------------------------------
// 路径规范化与包含判断
// ---------------------------------------------------------------------------

/// 把路径规范化为便于比较的字符串。
///
/// * 去掉 `\\?\` / `//?/` 长路径前缀；
/// * 统一分隔符（Windows 把 `/` 换成 `\`；Unix 保持原样，因为 `\` 在 Unix 里是
///   合法的文件名字符，强行替换反而会引入歧义）；
/// * 去掉尾部分隔符，但保留 `c:\` / `/` 这种根；
/// * Windows 与 macOS 折叠成小写（两个平台的默认文件系统都不区分大小写）。
pub fn norm(path: &Path) -> String {
    let raw = path.to_string_lossy();
    let mut s = raw.as_ref();
    // 去掉 Win32 长路径前缀
    if let Some(rest) = s.strip_prefix(r"\\?\").or_else(|| s.strip_prefix("//?/")) {
        s = rest;
    }

    let mut out: String = if cfg!(windows) {
        s.replace('/', "\\")
    } else {
        s.to_string()
    };

    // 去掉末尾分隔符，但保留根
    while out.len() > 1 && out.ends_with(SEP) && !is_fs_root(&out) {
        out.pop();
    }

    // `c:` 这种没有尾分隔符的写法补齐成 `c:\`
    let b = out.as_bytes();
    if cfg!(windows) && b.len() == 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
        out.push('\\');
    }

    if CASE_INSENSITIVE {
        out.make_ascii_lowercase();
    }
    out
}

/// 是否是文件系统根（Windows 的 `C:\`、Unix 的 `/`）。
pub fn is_fs_root(normalized: &str) -> bool {
    if cfg!(windows) {
        let b = normalized.as_bytes();
        b.len() == 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'\\'
    } else {
        normalized == "/"
    }
}

/// `candidate` 是否等于 `base`，或位于 `base` 之下。
pub fn is_within(base: &str, candidate: &str) -> bool {
    if base.is_empty() || candidate.is_empty() {
        return false;
    }
    if candidate == base {
        return true;
    }
    if base.ends_with(SEP) {
        candidate.starts_with(base)
    } else {
        candidate.len() > base.len()
            && candidate.starts_with(base)
            && candidate.as_bytes()[base.len()] == SEP as u8
    }
}

/// Windows 的系统盘前缀（`c:`）。
///
/// 静态名单里的 Windows 条目是按「相对系统盘根目录」写的（`\windows\system32`
/// 之类），而规范化后的路径总是带盘符的绝对路径，所以必须补上真实盘符才能对上。
/// 系统盘不一定是 `C:`，所以这里从 `%SystemRoot%` 反推。
#[cfg(windows)]
fn system_drive() -> Option<String> {
    let win = norm(&crate::sysutil::windows_dir());
    let bytes = win.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' {
        Some(win[..2].to_string())
    } else {
        None
    }
}

#[cfg(not(windows))]
fn system_drive() -> Option<String> {
    None
}

/// 逐个探测保护目录树，只算一次。
fn protected_trees() -> &'static [String] {
    static CELL: OnceLock<Vec<String>> = OnceLock::new();
    CELL.get_or_init(|| {
        let mut out: Vec<String> = Vec::new();
        let drive = system_drive();

        for s in PROTECTED_TREES_STATIC {
            if cfg!(windows) {
                let lowered = s.to_ascii_lowercase();
                match &drive {
                    // 补上盘符，同时也保留原样一份：万一取不到盘符还有兜底
                    Some(d) => out.push(format!("{d}{lowered}")),
                    None => out.push(lowered),
                }
            } else {
                out.push(s.to_string());
            }
        }

        #[cfg(windows)]
        {
            for key in ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"] {
                if let Some(dir) = crate::sysutil::env_path(key) {
                    let n = norm(&dir);
                    if !n.is_empty() {
                        out.push(n);
                    }
                }
            }
        }

        #[cfg(unix)]
        {
            // macOS 的 /var 与 /private/var 是同一个地方，两种写法都收进来
            out.push("/var/root".to_string());
            out.push("/private/var/root".to_string());
        }

        out.sort();
        out.dedup();
        out
    })
}

/// 用户资料目录 / 系统敏感目录：随用户真实路径动态生成，只计算一次。
fn user_protected() -> &'static [String] {
    static CELL: OnceLock<Vec<String>> = OnceLock::new();
    CELL.get_or_init(|| {
        #[cfg(windows)]
        const SUBDIRS: &[&str] = PROTECTED_USER_SUBDIRS_WINDOWS;
        #[cfg(target_os = "macos")]
        const SUBDIRS: &[&str] = PROTECTED_USER_SUBDIRS_MACOS;
        #[cfg(target_os = "linux")]
        const SUBDIRS: &[&str] = PROTECTED_USER_SUBDIRS_LINUX;
        #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
        const SUBDIRS: &[&str] = &[];

        let mut out: Vec<String> = Vec::new();
        let mut push_user = |base: PathBuf| {
            let base = norm(&base);
            if base.is_empty() {
                return;
            }
            for sub in SUBDIRS {
                let joined = if cfg!(windows) {
                    format!("{base}\\{}", sub.replace('/', "\\"))
                } else {
                    format!("{base}/{}", sub.trim_start_matches('/'))
                };
                out.push(joined);
            }
        };

        if let Some(home) = crate::sysutil::home_dir() {
            push_user(home);
            #[cfg(windows)]
            {
                // Windows 还有一份「公用」目录，内容属于所有人
                if let Some(public) = crate::sysutil::env_path("PUBLIC") {
                    push_user(public);
                }
            }
        }
        out
    })
}

/// 判断某个规范化路径是否命中保护名单。
fn hits_protected_tree(normalized: &str) -> bool {
    protected_trees()
        .iter()
        .any(|tree| is_within(tree, normalized))
}

/// 文件名是否属于"绝对不允许删除"的关键文件（廉价检查，用于扫描阶段预筛）。
pub fn is_protected_file_name(name: &str) -> bool {
    PROTECTED_FILE_NAMES.contains(&name.to_ascii_lowercase().as_str())
}

/// 给界面用的「保护目录举例」——每个平台挑几个最有代表性的。
pub fn protected_examples() -> &'static str {
    match crate::sysutil::PLATFORM {
        #[cfg(windows)]
        crate::sysutil::Platform::Windows => "System32、WinSxS、Program Files",
        #[cfg(target_os = "macos")]
        crate::sysutil::Platform::MacOs => "/System、/Library、/usr、/etc",
        #[cfg(target_os = "linux")]
        crate::sysutil::Platform::Linux => "/usr、/etc、/boot、/var/lib",
        _ => "系统关键目录",
    }
}

/// 路径是否位于受保护区域（含用户文档目录）。
pub fn is_protected(path: &Path) -> bool {
    let normalized = norm(path);
    if normalized.is_empty() || is_fs_root(&normalized) {
        return true;
    }
    hits_protected_tree(&normalized) || user_protected().iter().any(|t| is_within(t, &normalized))
}

/// 路径是否命中**系统级**保护目录树（系统目录、程序目录、回收站…）。
///
/// 与 [`is_protected`] 的区别：这里不含「文档 / 下载 / 图片」这类用户资料目录。
///
/// 之所以要分开，是因为扫描大文件时需要区分两种情况：
/// * 用户资料目录 —— **要进去统计体积**（不然「文档」会显示成 0 字节），
///   只是不把它列为可删除对象；
/// * 系统目录 —— 连进都不进，既省时间也少一处出错的可能。
pub fn is_system_protected(path: &Path) -> bool {
    let normalized = norm(path);
    if normalized.is_empty() || is_fs_root(&normalized) {
        return true;
    }
    if hits_protected_tree(&normalized) {
        return true;
    }
    // 用户主目录里的「系统级」子树（AppData/Local、Library/Caches 之外的敏感目录等）
    user_protected()
        .iter()
        .any(|t| is_within(t, &normalized) && !is_personal_tree(t))
}

/// 用户资料目录（文档 / 桌面 / 下载 / 图片…）——
/// 受保护但**不是**系统目录，扫描大文件时要进去算体积。
fn is_personal_tree(normalized: &str) -> bool {
    const PERSONAL: &[&str] = &[
        "\\documents",
        "\\desktop",
        "\\downloads",
        "\\pictures",
        "\\videos",
        "\\music",
        "/documents",
        "/desktop",
        "/downloads",
        "/pictures",
        "/videos",
        "/movies",
        "/music",
    ];
    normalised_ends_with_any(normalized, PERSONAL)
}

fn normalised_ends_with_any(haystack: &str, suffixes: &[&str]) -> bool {
    suffixes.iter().any(|s| haystack.ends_with(s))
}

// ---------------------------------------------------------------------------
// 删除前校验
// ---------------------------------------------------------------------------

/// 校验某个路径是否允许被删除：必须位于 `root` 之内，且不命中任何保护规则。
///
/// `is_symlink` 由调用方通过 `symlink_metadata` 判断后传入（本函数不做 IO）。
pub fn verify(path: &Path, root: &Path, is_symlink: bool) -> Result<(), Reject> {
    if path
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(Reject::ParentComponent);
    }

    let p = norm(path);
    if p.is_empty() {
        return Err(Reject::EmptyPath);
    }
    if is_fs_root(&p) {
        return Err(Reject::FsRoot);
    }
    if p.len() < MIN_PATH_LEN {
        return Err(Reject::EmptyPath);
    }

    let r = norm(root);
    // 清理根目录本身也不能是文件系统根
    if is_fs_root(&r) {
        return Err(Reject::FsRoot);
    }
    if p == r {
        return Err(Reject::RootItself);
    }
    if !is_within(&r, &p) {
        return Err(Reject::OutsideRoot);
    }
    if hits_protected_tree(&p) {
        return Err(Reject::ProtectedTree);
    }
    if user_protected().iter().any(|t| is_within(t, &p)) {
        return Err(Reject::ProtectedTree);
    }

    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
        if PROTECTED_FILE_NAMES.contains(&name.to_ascii_lowercase().as_str()) {
            return Err(Reject::ProtectedFileName);
        }
    }

    if is_symlink {
        return Err(Reject::SymbolicLink);
    }

    Ok(())
}

/// 清理根目录是否"看起来合法"（在扫描阶段做一次粗筛，避免配置错误导致大面积误删）。
pub fn verify_root(root: &Path) -> Result<(), Reject> {
    let r = norm(root);
    if r.is_empty() || r.len() < MIN_PATH_LEN {
        return Err(Reject::EmptyPath);
    }
    if is_fs_root(&r) {
        return Err(Reject::FsRoot);
    }
    if hits_protected_tree(&r) {
        return Err(Reject::ProtectedTree);
    }
    if user_protected().iter().any(|t| is_within(t, &r)) {
        return Err(Reject::ProtectedTree);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 年龄过滤
// ---------------------------------------------------------------------------

/// 文件是否"太新"而不该被清理。
///
/// - `min_age_days == 0` 表示不做年龄限制；
/// - 读不到修改时间时按"太新"处理（保守策略，宁可漏删不可误删）。
pub fn too_new(modified: Option<SystemTime>, min_age_days: u32) -> bool {
    if min_age_days == 0 {
        return false;
    }
    let Some(modified) = modified else {
        return true;
    };
    match SystemTime::now().duration_since(modified) {
        Ok(age) => age < Duration::from_secs(u64::from(min_age_days) * 86_400),
        // 修改时间在未来（系统时钟异常），保守跳过
        Err(_) => true,
    }
}

// ---------------------------------------------------------------------------
// 测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试用：把若干段拼成一个平台合法的路径。
    fn join(root: &str, parts: &[&str]) -> PathBuf {
        let mut path = PathBuf::from(root);
        for part in parts {
            path.push(part);
        }
        path
    }

    #[test]
    fn normalize_and_containment() {
        if cfg!(windows) {
            assert_eq!(norm(Path::new(r"C:\Windows\Temp\")), r"c:\windows\temp");
            assert_eq!(norm(Path::new(r"\\?\C:\Temp\x")), r"c:\temp\x");
            assert!(is_within(r"c:\temp", r"c:\temp\a\b"));
            assert!(!is_within(r"c:\temp", r"c:\temp2"));
            assert!(is_within(r"c:\", r"c:\anything"));
            assert!(is_fs_root(r"c:\"));
            assert!(!is_fs_root(r"c:\windows"));
        } else {
            assert_eq!(norm(Path::new("/var/log/")), "/var/log");
            assert_eq!(norm(Path::new("//tmp/x")), "/tmp/x");
            assert!(is_within("/tmp", "/tmp/a/b"));
            assert!(!is_within("/tmp", "/tmp2"));
            assert!(is_within("/", "/anything"));
            assert!(is_fs_root("/"));
            assert!(!is_fs_root("/var"));
        }
    }

    #[test]
    fn trailing_separator_does_not_change_containment() {
        // 同一个目录，带不带结尾分隔符都要能正确判断包含关系
        let (base_path, child_path) = if cfg!(windows) {
            (r"C:\Temp\", r"C:\Temp\a")
        } else {
            ("/tmp/", "/tmp/a")
        };
        assert!(is_within(
            &norm(Path::new(base_path)),
            &norm(Path::new(child_path))
        ));
    }

    #[test]
    fn protected_trees_are_rejected() {
        let (inside, outside) = if cfg!(windows) {
            (
                r"C:\Windows\System32\drivers\etc\hosts",
                r"C:\Windows\Temp\foo.tmp",
            )
        } else {
            ("/usr/lib/libc.so", "/tmp/foo.tmp")
        };
        assert!(is_protected(Path::new(inside)), "{inside} 应当受保护");
        assert!(!is_protected(Path::new(outside)), "{outside} 不应当受保护");
    }

    #[test]
    fn installed_program_dirs_are_protected() {
        let path = if cfg!(windows) {
            PathBuf::from(r"C:\Program Files\App\x.dll")
        } else {
            PathBuf::from("/usr/bin/ls")
        };
        assert!(is_protected(&path), "{} 应当受保护", path.display());
    }

    #[test]
    fn system_protection_excludes_personal_folders() {
        let home = crate::sysutil::home_dir().expect("测试环境应当有主目录");
        let personal = home.join("Documents");

        assert!(is_protected(&personal), "{} 应受保护", personal.display());
        assert!(
            !is_system_protected(&personal),
            "{} 属于个人目录，不算系统目录（扫描大文件时要进去算体积）",
            personal.display()
        );
    }

    #[test]
    fn verify_blocks_escape_and_key_files() {
        if cfg!(windows) {
            let root = Path::new(r"C:\Windows\Temp");
            assert!(verify(Path::new(r"C:\Windows\Temp\a.tmp"), root, false).is_ok());
            assert_eq!(
                verify(Path::new(r"C:\Windows\System32\x.dll"), root, false),
                Err(Reject::OutsideRoot)
            );
            assert_eq!(
                verify(Path::new(r"C:\Windows\Temp\desktop.ini"), root, false),
                Err(Reject::ProtectedFileName)
            );
            assert_eq!(
                verify(Path::new(r"C:\Windows\Temp\pagefile.sys"), root, false),
                Err(Reject::ProtectedFileName)
            );
            assert_eq!(verify(root, root, false), Err(Reject::RootItself));
            assert_eq!(
                verify(Path::new(r"C:\Windows\Temp\link"), root, true),
                Err(Reject::SymbolicLink)
            );
            assert_eq!(
                verify(Path::new(r"C:\"), Path::new(r"C:\"), false),
                Err(Reject::FsRoot)
            );
            assert_eq!(
                verify(Path::new(r"C:\Windows"), Path::new(r"C:\"), false),
                Err(Reject::FsRoot)
            );
        } else {
            let root = Path::new("/tmp/cleanc-test");
            assert!(verify(Path::new("/tmp/cleanc-test/a.tmp"), root, false).is_ok());
            assert_eq!(
                verify(Path::new("/usr/bin/ls"), root, false),
                Err(Reject::OutsideRoot)
            );
            assert_eq!(
                verify(Path::new("/tmp/cleanc-test/passwd"), root, false),
                Err(Reject::ProtectedFileName)
            );
            assert_eq!(verify(root, root, false), Err(Reject::RootItself));
            assert_eq!(
                verify(Path::new("/tmp/cleanc-test/link"), root, true),
                Err(Reject::SymbolicLink)
            );
            assert_eq!(
                verify(Path::new("/"), Path::new("/"), false),
                Err(Reject::FsRoot)
            );
            assert_eq!(
                verify(Path::new("/tmp"), Path::new("/"), false),
                Err(Reject::FsRoot)
            );
        }
    }

    #[test]
    fn parent_component_is_rejected_everywhere() {
        let root = if cfg!(windows) {
            PathBuf::from(r"C:\Windows\Temp")
        } else {
            PathBuf::from("/tmp/cleanc-test")
        };
        let bad = root.join("..").join("evil");
        assert_eq!(verify(&bad, &root, false), Err(Reject::ParentComponent));
    }

    #[test]
    fn protected_roots_are_rejected() {
        // 清理根自己落在保护名单里时必须被拦下（配置写错的第一道保险）
        let bad = if cfg!(windows) {
            join(r"C:\", &["Windows", "System32"])
        } else {
            PathBuf::from("/etc")
        };
        assert_eq!(verify_root(&bad), Err(Reject::ProtectedTree));
    }
}
