//! 已安装软件：枚举、统计实际占用空间、调用厂商自己的卸载方式。
//!
//! # 各平台怎么"知道装了哪些软件"
//!
//! | 平台 | 数据来源 |
//! |------|----------|
//! | Windows | 注册表 `...\CurrentVersion\Uninstall`（「设置 → 应用」读的就是它） |
//! | macOS | `/Applications`、`/System/Applications`、`~/Applications` 下的 `.app` 包 |
//! | Linux | dpkg 数据库（Debian / Ubuntu）+ `snap list` + `flatpak list` |
//!
//! # 为什么坚持"把卸载交给厂商 / 系统"
//!
//! * Windows 上直接调用注册表里登记的卸载命令，比我们自己猜目录、删文件安全得多；
//! * macOS 上把 `.app` 整个移入废纸篓 —— 这就是官方推荐的卸载方式；
//! * Linux 上交给 dpkg / snap / flatpak 自己处理，避免删坏依赖关系。
//!
//! 启动前界面上会把**完整命令或完整路径**原样展示出来，用户确认后才执行。
//!
//! # 体积统计
//!
//! * Windows：按 `InstallLocation` 递归累加，兜底用注册表的 `EstimatedSize`；
//! * macOS：递归累加 `.app` 包，非常准；
//! * Linux：dpkg 的 `Installed-Size` 字段（包管理器自己算的，只能当估算）。

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::time::Instant;

// ---------------------------------------------------------------------------
// 数据模型
// ---------------------------------------------------------------------------

/// 卸载这个条目时会执行什么动作。
///
/// 三个平台各用其中一个变体，因此在本平台上总有一半变体是"用不到"的，
/// 这里统一关掉 dead_code，免得交叉编译时一半的构建都在刷告警。
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum UninstallAction {
    /// 厂商登记的卸载命令（Windows）
    Vendor(String),
    /// 把一个应用包 / 目录整体移入回收站（macOS 的 `.app`）
    TrashBundle(PathBuf),
    /// 交给系统包管理器执行（Linux），命令需要 root
    PackageManager {
        command: String,
        /// 命令作用的包名，界面上会单独列出来
        packages: Vec<String>,
    },
    /// 没有可用的卸载方式
    None,
}

impl UninstallAction {
    /// 界面上"将要执行的动作"那一栏显示的文本。
    pub fn display(&self) -> String {
        match self {
            UninstallAction::Vendor(command) => command.clone(),
            UninstallAction::TrashBundle(path) => {
                format!(
                    "把整个应用包移入{}：{}",
                    crate::recycle::trash_name(),
                    path.display()
                )
            }
            UninstallAction::PackageManager { command, packages } => {
                if packages.is_empty() {
                    command.clone()
                } else {
                    format!("{}\n（作用的软件包：{}）", command, packages.join("、"))
                }
            }
            UninstallAction::None => "（无）".to_string(),
        }
    }

    /// 确认弹窗上那个小胶囊里的短标签。三个平台的动作语义不一样，
    /// 文案也跟着分叉：Windows 是"启动厂商卸载程序"，macOS 是"整个应用包
    /// 进废纸篓"，Linux 是"交给包管理器"。
    pub fn short_label(&self) -> String {
        match self {
            UninstallAction::Vendor(_) => "调用厂商卸载程序".to_string(),
            UninstallAction::TrashBundle(_) => {
                format!("整个应用包移入{}", crate::recycle::trash_name())
            }
            UninstallAction::PackageManager { .. } => "交给系统包管理器".to_string(),
            UninstallAction::None => "没有可用的卸载方式".to_string(),
        }
    }

    /// 胶囊下面那句解释文字。
    pub fn description(&self) -> &'static str {
        match self {
            UninstallAction::Vendor(_) => {
                "下面的卸载动作会交给这款软件自带的卸载程序执行，本工具只负责把它启动起来。"
            }
            UninstallAction::TrashBundle(_) => {
                "应用包会被整体移入回收站，误删了可以随时还原；残留的偏好设置需要你自己清理。"
            }
            UninstallAction::PackageManager { .. } => {
                "下面的卸载动作会交给系统的包管理器执行，需要超级用户权限，届时会弹出授权窗口。"
            }
            UninstallAction::None => "这款软件没有提供可用的卸载方式，本工具不会对它做任何操作。",
        }
    }

    /// 这个动作是否需要提权（包管理器必然需要；厂商卸载程序看安装范围）。
    pub fn always_needs_root(&self) -> bool {
        matches!(self, UninstallAction::PackageManager { .. })
    }
}

/// 一条已安装软件记录。
#[derive(Debug, Clone)]
pub struct InstalledApp {
    pub name: String,
    pub version: String,
    pub publisher: String,
    /// 安装位置（Windows 来自注册表，macOS 是 `.app` 包路径，Linux 通常没有）
    pub install_location: Option<PathBuf>,
    /// 由来源登记的估算体积（Windows 单位 KB 已换算成字节，Linux 是 dpkg 的 Installed-Size）
    pub estimated_bytes: Option<u64>,
    /// 递归统计出来的真实体积（后台扫描逐条填充）
    pub measured_bytes: Option<u64>,
    /// 安装日期，Windows 是 `YYYYMMDD`，其它平台也统一成这个格式
    pub install_date: Option<String>,
    /// 卸载是否需要超级用户权限
    pub machine_wide: bool,
    /// 来源明确写了「不允许卸载」
    pub no_remove: bool,
    /// 卸载动作
    pub uninstall: UninstallAction,
}

impl InstalledApp {
    /// 展示用的体积：优先用实测值，没有就用来源登记的估算值。
    pub fn size_bytes(&self) -> Option<u64> {
        self.measured_bytes.or(self.estimated_bytes)
    }

    /// 展示用的体积是不是"估算"的。
    pub fn size_is_estimated(&self) -> bool {
        self.measured_bytes.is_none()
    }

    /// 安装日期的可读形式（YYYYMMDD → YYYY-MM-DD）。
    pub fn install_date_label(&self) -> Option<String> {
        let raw = self.install_date.as_deref()?;
        if raw.len() == 8 && raw.chars().all(|c| c.is_ascii_digit()) {
            Some(format!("{}-{}-{}", &raw[0..4], &raw[4..6], &raw[6..8]))
        } else {
            Some(raw.to_string())
        }
    }

    /// 卸载动作对应的原始命令（只有 Windows / Linux 有）。
    pub fn uninstall_command(&self) -> Option<&str> {
        match &self.uninstall {
            UninstallAction::Vendor(command) | UninstallAction::PackageManager { command, .. } => {
                Some(command.as_str()).filter(|s| !s.trim().is_empty())
            }
            _ => None,
        }
    }

    /// 界面上展示的"将要执行的动作"。
    pub fn uninstall_display(&self) -> String {
        self.uninstall.display()
    }

    pub fn can_uninstall(&self) -> bool {
        !self.no_remove && !matches!(self.uninstall, UninstallAction::None)
    }

    /// 卸载时是否需要提权。
    pub fn needs_elevation(&self) -> bool {
        self.machine_wide && !crate::sysutil::is_admin()
    }
}

// ---------------------------------------------------------------------------
// 后台扫描
// ---------------------------------------------------------------------------

pub enum AppMsg {
    /// 枚举完成，软件列表可以先显示了
    Found {
        apps: Vec<InstalledApp>,
    },
    /// 第 `index` 条的真实体积算出来了
    Size {
        index: usize,
        bytes: u64,
    },
    Finished {
        elapsed_ms: u64,
        canceled: bool,
    },
}

pub struct AppHandle {
    pub rx: Receiver<AppMsg>,
    cancel: Arc<AtomicBool>,
}

impl AppHandle {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// 启动后台扫描：先枚举，再并行统计体积。
pub fn spawn() -> AppHandle {
    let (tx, rx) = channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_worker = Arc::clone(&cancel);

    std::thread::spawn(move || {
        let started = Instant::now();

        let apps = enumerate();
        // 真正要统计的目录先算好，随后就能把 `apps` 的所有权交给界面
        let locations: Vec<Option<PathBuf>> = resolve_locations(&apps);
        let _ = tx.send(AppMsg::Found { apps });

        // 并行统计：每个软件一棵目录树，互不干扰
        use rayon::prelude::*;
        locations
            .par_iter()
            .enumerate()
            .for_each(|(index, location)| {
                if cancel_worker.load(Ordering::Relaxed) {
                    return;
                }
                let Some(path) = location else {
                    return;
                };
                let bytes = dir_size(path);
                let _ = tx.send(AppMsg::Size { index, bytes });
            });

        let canceled = cancel_worker.load(Ordering::Relaxed);
        let _ = tx.send(AppMsg::Finished {
            elapsed_ms: started.elapsed().as_millis() as u64,
            canceled,
        });
    });

    AppHandle { rx, cancel }
}

// ---------------------------------------------------------------------------
// 体积统计
// ---------------------------------------------------------------------------

/// 递归累加目录下所有文件的大小（实现见 [`crate::sysutil::dir_size`]）。
pub fn dir_size(root: &Path) -> u64 {
    crate::sysutil::dir_size(root)
}

/// 为每个软件选定统计目录，并处理"多个软件指向同一个目录"的情况。
///
/// 典型场景（Windows）：Chrome 的网页应用（Grok、remove.bg…）没有任何安装目录，
/// 但它们的卸载命令是 `chrome.exe --uninstall-app-id=…`，于是"卸载程序所在的目录"
/// 全都落在 `C:\Program Files\Google\Chrome\Application`。
/// 如果照单全收，每个网页应用都会显示成 512 MB，既不准又重复计算。
///
/// 处理办法：同一个目录只允许一个条目占用 ——
/// 优先给"来源明确登记了 InstallLocation"的那一条，其余置空。
fn resolve_locations(apps: &[InstalledApp]) -> Vec<Option<PathBuf>> {
    use std::collections::HashMap;

    let candidates: Vec<(Option<PathBuf>, bool)> = apps
        .iter()
        .map(|app| {
            if let Some(dir) = &app.install_location
                && is_measurable(dir)
            {
                return (Some(dir.clone()), true);
            }
            (fallback_location(app), false)
        })
        .collect();

    // 第一轮：登记表来源先占位
    let mut owner: HashMap<String, usize> = HashMap::new();
    for (index, (location, direct)) in candidates.iter().enumerate() {
        if let (Some(path), true) = (location, direct) {
            owner.entry(crate::safety::norm(path)).or_insert(index);
        }
    }

    // 第二轮：没人占的目录按出现顺序补位，重复的置空
    let mut resolved: Vec<Option<PathBuf>> = candidates.iter().map(|(p, _)| p.clone()).collect();
    for (index, location) in resolved.iter_mut().enumerate() {
        let Some(path) = location else {
            continue;
        };
        let key = crate::safety::norm(path);
        let slot = owner.entry(key).or_insert(index);
        if *slot != index {
            *location = None;
        }
    }

    resolved
}

/// 没有 InstallLocation 时，退而求其次用卸载程序所在的目录。
fn fallback_location(app: &InstalledApp) -> Option<PathBuf> {
    let command = app.uninstall_command()?;
    let (program, _) = split_command_line(command);
    let program = PathBuf::from(program);
    if !program.is_file() {
        return None;
    }
    let parent = program.parent()?;
    if is_measurable(parent) {
        Some(parent.to_path_buf())
    } else {
        None
    }
}

/// 判断一个目录是否"窄到可以代表某个软件"。
///
/// 注册表里经常出现 `C:\Program Files`、`C:\ProgramData` 这种把一整片区域
/// 当成安装目录的条目，统计它们会把别人的体积算进来，因此一律排除。
fn is_measurable(path: &Path) -> bool {
    if !path.is_dir() {
        return false;
    }

    let normalized = crate::safety::norm(path);

    // 至少要有两级：`c:\program files\foo` / `/Applications/Foo.app` 才算具体
    let depth = normalized
        .trim_end_matches(crate::safety::SEP)
        .split(crate::safety::SEP)
        .filter(|s| !s.is_empty())
        .count();
    if depth < 3 && !cfg!(unix) {
        return false;
    }
    if depth < 2 {
        return false;
    }

    let broad: &[&str] = if cfg!(windows) {
        &[
            r"c:\program files",
            r"c:\program files (x86)",
            r"c:\programdata",
            r"c:\windows",
            r"c:\windows\temp",
            r"c:\users",
            r"c:\$recycle.bin",
        ]
    } else {
        &[
            "/",
            "/applications",
            "/system",
            "/system/applications",
            "/usr",
            "/usr/share",
            "/usr/local",
            "/opt",
            "/var",
            "/etc",
            "/home",
        ]
    };
    if broad.contains(&normalized.as_str()) {
        return false;
    }

    // 用户主目录本身也不能算作某个软件的安装目录
    if let Some(home) = crate::sysutil::home_dir()
        && crate::safety::norm(&home) == normalized
    {
        return false;
    }

    true
}

// ---------------------------------------------------------------------------
// 卸载
// ---------------------------------------------------------------------------

/// 把 `"C:\Program Files\App\uninstall.exe" /S` 拆成 (程序, 参数)。
pub(crate) fn split_command_line(command: &str) -> (String, String) {
    let trimmed = command.trim();

    if let Some(rest) = trimmed.strip_prefix('"')
        && let Some(end) = rest.find('"')
    {
        return (
            rest[..end].to_string(),
            rest[end + 1..].trim_start().to_string(),
        );
    }

    // 没加引号：如果整串本身就是一个存在的文件，那就没有参数
    if Path::new(trimmed).is_file() {
        return (trimmed.to_string(), String::new());
    }

    match trimmed.split_once(' ') {
        Some((program, args)) => (program.to_string(), args.trim_start().to_string()),
        None => (trimmed.to_string(), String::new()),
    }
}

/// 执行一条卸载动作。
pub fn run_uninstall(app: &InstalledApp) -> Result<(), String> {
    match &app.uninstall {
        UninstallAction::Vendor(command) => {
            let (program, args) = split_command_line(command);
            run_vendor_uninstaller(&program, &args, app.needs_elevation())
        }
        UninstallAction::TrashBundle(path) => crate::recycle::recycle_dir(path),
        UninstallAction::PackageManager { command, .. } => run_package_manager(command),
        UninstallAction::None => Err("该软件没有可用的卸载方式".to_string()),
    }
}

/// 启动厂商自带的卸载程序。
///
/// `elevate` 为真时以"以管理员身份运行"启动（会弹 UAC）。
#[cfg(windows)]
fn run_vendor_uninstaller(program: &str, args: &str, elevate: bool) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    fn wide(s: &str) -> Vec<u16> {
        std::ffi::OsStr::new(s)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    let program_path = Path::new(program);
    if !program_path.is_file() {
        return Err(format!("找不到卸载程序：{program}"));
    }

    let verb = wide(if elevate { "runas" } else { "open" });
    let file = wide(program);
    let params = if args.is_empty() {
        None
    } else {
        Some(wide(args))
    };
    // 把工作目录设成卸载程序所在目录：不少老卸载器要靠相对路径找自己的资源
    let workdir = program_path
        .parent()
        .map(|p| wide(&p.to_string_lossy()))
        .unwrap_or_else(|| wide(""));

    // SAFETY: 所有宽字符串都以 NUL 结尾且在调用期间存活；句柄参数传空表示无父窗口。
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            file.as_ptr(),
            params.as_ref().map_or(std::ptr::null(), |v| v.as_ptr()),
            workdir.as_ptr(),
            SW_SHOWNORMAL,
        )
    };

    let code = result as isize;
    if code <= 32 {
        Err(describe_shell_error(code))
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn run_vendor_uninstaller(program: &str, args: &str, elevate: bool) -> Result<(), String> {
    let command = if args.is_empty() {
        program.to_string()
    } else {
        format!("{program} {args}")
    };
    if elevate && !crate::sysutil::is_admin() {
        run_privileged(&command)
    } else {
        spawn_shell(&command)
    }
}

#[cfg(windows)]
fn describe_shell_error(code: isize) -> String {
    let text = match code {
        0 => "系统资源不足",
        2 => "找不到指定的文件",
        3 => "找不到指定的路径",
        5 => "拒绝访问（可能是用户取消了 UAC 授权）",
        8 => "内存不足",
        11 => "程序格式无效",
        26 => "文件共享冲突",
        27 => "文件关联不完整",
        28 => "DDE 超时",
        29 => "DDE 事务失败",
        30 => "DDE 忙",
        31 => "没有关联的应用程序",
        32 => "找不到所需的 DLL",
        _ => "未知错误",
    };
    format!("启动失败：{text}（代码 {code}）")
}

/// 交给系统包管理器执行（Linux）。
///
/// 优先用 `pkexec` 弹图形化的授权框；已经是 root 就直接跑；
/// 两者都不行时把命令原样返回给用户，让他在终端里自己执行 —— 不猜、不硬来。
#[cfg(unix)]
fn run_package_manager(command: &str) -> Result<(), String> {
    if crate::sysutil::is_admin() {
        return spawn_shell(command);
    }
    if command_exists("pkexec") {
        return run_privileged(command);
    }
    Err(format!(
        "卸载系统软件包需要 root 权限。请在终端里执行：sudo {command}"
    ))
}

#[cfg(not(unix))]
fn run_package_manager(command: &str) -> Result<(), String> {
    spawn_shell(command)
}

/// 用登录 shell 执行一条命令。
#[cfg(unix)]
fn spawn_shell(command: &str) -> Result<(), String> {
    std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(command)
        .stdin(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("启动命令失败：{e}"))
}

#[cfg(not(unix))]
fn spawn_shell(_command: &str) -> Result<(), String> {
    Err("当前平台不支持 shell 卸载".to_string())
}

/// 用 `pkexec` 提权执行。
#[cfg(unix)]
fn run_privileged(command: &str) -> Result<(), String> {
    std::process::Command::new("pkexec")
        .arg("/bin/sh")
        .arg("-c")
        .arg(command)
        .stdin(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("调用 pkexec 失败：{e}（请在终端里手动执行：sudo {command}）"))
}

#[cfg(unix)]
fn command_exists(program: &str) -> bool {
    if program.contains('/') {
        return Path::new(program).is_file();
    }
    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&paths).any(|dir| dir.join(program).is_file())
}

// ---------------------------------------------------------------------------
// 枚举入口
// ---------------------------------------------------------------------------

/// 界面上说明软件清单"是从哪儿读来的"（短标签，放进句子里用）。
pub fn source_label() -> &'static str {
    match crate::sysutil::PLATFORM {
        #[cfg(windows)]
        crate::sysutil::Platform::Windows => "注册表",
        #[cfg(target_os = "macos")]
        crate::sysutil::Platform::MacOs => "Applications 目录里的 .app 包",
        #[cfg(target_os = "linux")]
        crate::sysutil::Platform::Linux => "dpkg / snap / flatpak",
        _ => "系统",
    }
}

/// 每个软件的"体积"是怎么算出来的（完整句子，界面上当脚注）。
pub fn size_note() -> String {
    match crate::sysutil::PLATFORM {
        #[cfg(windows)]
        crate::sysutil::Platform::Windows => {
            "体积按软件登记在注册表里的安装目录递归统计，只读元数据、不打开文件。".to_string()
        }
        #[cfg(target_os = "macos")]
        crate::sysutil::Platform::MacOs => {
            "体积按 .app 应用包的目录递归统计，只读元数据、不打开文件。".to_string()
        }
        #[cfg(target_os = "linux")]
        crate::sysutil::Platform::Linux => {
            "体积优先用包管理器登记的安装大小，能用目录统计的就实测一遍。".to_string()
        }
        _ => "体积按安装目录递归统计，只读元数据、不打开文件。".to_string(),
    }
}

/// 未安装任何东西时也要能正常返回空列表。
pub fn enumerate() -> Vec<InstalledApp> {
    let mut apps = platform_enumerate();
    apps.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    dedupe(apps)
}

#[cfg(windows)]
fn platform_enumerate() -> Vec<InstalledApp> {
    win::enumerate()
}

#[cfg(target_os = "macos")]
fn platform_enumerate() -> Vec<InstalledApp> {
    mac::enumerate()
}

#[cfg(target_os = "linux")]
fn platform_enumerate() -> Vec<InstalledApp> {
    linux::enumerate()
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
fn platform_enumerate() -> Vec<InstalledApp> {
    Vec::new()
}

/// 同一个软件有时会被列出两次（64/32 位视图、多个来源同时登记），
/// 这里按「名称 + 版本 + 安装位置」去重，保留信息更全的那一条。
fn dedupe(apps: Vec<InstalledApp>) -> Vec<InstalledApp> {
    let mut seen: Vec<String> = Vec::new();
    let mut out: Vec<InstalledApp> = Vec::new();

    for app in apps {
        let location = app
            .install_location
            .as_ref()
            .map(|p| crate::safety::norm(p))
            .unwrap_or_default();
        let key = format!(
            "{}\u{1}{}\u{1}{}",
            app.name.to_lowercase(),
            app.version.to_lowercase(),
            location
        );
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        out.push(app);
    }

    out
}

// ---------------------------------------------------------------------------
// macOS 实现
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
mod mac {
    use std::path::PathBuf;

    use super::{InstalledApp, UninstallAction};

    /// 扫描这些位置里的 `.app` 包。
    const APP_DIRS: &[&str] = &[
        "/Applications",
        "/Applications/Utilities",
        "/System/Applications",
        "/System/Applications/Utilities",
        "/System/Library/CoreServices",
    ];

    pub fn enumerate() -> Vec<InstalledApp> {
        let mut out = Vec::new();

        for dir in APP_DIRS {
            collect_dir(std::path::Path::new(dir), &mut out);
        }
        if let Some(home) = crate::sysutil::home_dir() {
            collect_dir(&home.join("Applications"), &mut out);
        }

        out
    }

    fn collect_dir(dir: &std::path::Path, out: &mut Vec<InstalledApp>) {
        let Ok(reader) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in reader.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("app") {
                continue;
            }
            if let Some(app) = read_bundle(&path) {
                out.push(app);
            }
        }
    }

    fn read_bundle(bundle: &std::path::Path) -> Option<InstalledApp> {
        let plist = bundle.join("Contents").join("Info.plist");

        let fallback_name = bundle.file_stem()?.to_string_lossy().to_string();
        let name = plist_string(&plist, "CFBundleDisplayName")
            .or_else(|| plist_string(&plist, "CFBundleName"))
            .unwrap_or(fallback_name);

        let version = plist_string(&plist, "CFBundleShortVersionString")
            .or_else(|| plist_string(&plist, "CFBundleVersion"))
            .unwrap_or_default();

        let publisher = plist_string(&plist, "CFBundleIdentifier").unwrap_or_default();

        // 系统自带的 App 受 SIP 保护，删不掉，直接标成不可卸载
        let system_owned = bundle.starts_with("/System");
        let in_applications = bundle.starts_with("/Applications");

        let install_date = std::fs::metadata(bundle)
            .and_then(|meta| meta.modified())
            .ok()
            .map(|time| {
                let dt: chrono::DateTime<chrono::Local> = time.into();
                dt.format("%Y%m%d").to_string()
            });

        let uninstall = if system_owned {
            UninstallAction::None
        } else {
            UninstallAction::TrashBundle(bundle.to_path_buf())
        };

        Some(InstalledApp {
            name,
            version,
            publisher,
            install_location: Some(PathBuf::from(bundle)),
            // 不填估算值：.app 包可以在后台精确统计，直接给真数
            estimated_bytes: None,
            measured_bytes: None,
            install_date,
            // /Applications 需要管理员密码才能改动，用户目录下的不用
            machine_wide: in_applications,
            no_remove: system_owned,
            uninstall,
        })
    }

    /// 从 `Info.plist` 里取一个字符串键。
    ///
    /// plist 可能是 XML 也可能是二进制；二进制的先用 `plutil` 转成 XML 再解析，
    /// 这样就不需要引入任何 plist 解析依赖。
    fn plist_string(plist: &std::path::Path, key: &str) -> Option<String> {
        let bytes = std::fs::read(plist).ok()?;
        if bytes.starts_with(b"bplist") {
            let output = std::process::Command::new("plutil")
                .args(["-convert", "xml1", "-o", "-"])
                .arg(plist)
                .output()
                .ok()?;
            if !output.status.success() {
                return None;
            }
            let text = String::from_utf8_lossy(&output.stdout).to_string();
            return find_xml_value(&text, key);
        }
        let text = String::from_utf8_lossy(&bytes).to_string();
        find_xml_value(&text, key)
    }

    /// `<key>Foo</key>` 后面紧跟着的那个 `<string>bar</string>`。
    fn find_xml_value(text: &str, key: &str) -> Option<String> {
        let needle = format!("<key>{key}</key>");
        let start = text.find(&needle)? + needle.len();
        let rest = &text[start..];
        let open = rest.find("<string>")? + "<string>".len();
        let close = rest[open..].find("</string>")?;
        let value = rest[open..open + close].trim().to_string();
        if value.is_empty() { None } else { Some(value) }
    }
}

// ---------------------------------------------------------------------------
// Linux 实现
// ---------------------------------------------------------------------------

#[cfg(target_os = "linux")]
mod linux {
    use super::{InstalledApp, UninstallAction};

    /// dpkg 数据库里这些 section 属于"库 / 调试符号 / 翻译包"，
    /// 列出来只会淹没真正想卸载的软件，直接跳过。
    const SKIPPED_SECTIONS: &[&str] = &[
        "libs",
        "oldlibs",
        "debug",
        "introspection",
        "translations",
        "localization",
        "perl",
        "ruby",
    ];

    pub fn enumerate() -> Vec<InstalledApp> {
        let mut out = Vec::new();
        out.extend(dpkg_packages());
        out.extend(snap_packages());
        out.extend(flatpak_packages());
        out
    }

    // ---- dpkg（Debian / Ubuntu）----

    fn dpkg_packages() -> Vec<InstalledApp> {
        let Ok(text) = std::fs::read_to_string("/var/lib/dpkg/status") else {
            return Vec::new();
        };

        let mut out = Vec::new();
        for block in text.split("\n\n") {
            if let Some(app) = read_dpkg_block(block) {
                out.push(app);
            }
        }
        out
    }

    fn read_dpkg_block(block: &str) -> Option<InstalledApp> {
        let mut name = None;
        let mut version = None;
        let mut maintainer = None;
        let mut section = None;
        let mut installed_size = None;
        let mut status_ok = false;
        let mut essential = false;

        for line in block.lines() {
            let Some((key, value)) = line.split_once(':') else {
                continue;
            };
            let value = value.trim();
            match key {
                "Package" => name = Some(value.to_string()),
                "Version" => version = Some(value.to_string()),
                "Maintainer" => maintainer = Some(value.to_string()),
                "Section" => section = Some(value.to_string()),
                // 单位是 KiB
                "Installed-Size" => installed_size = value.parse::<u64>().ok(),
                "Essential" => essential = value.eq_ignore_ascii_case("yes"),
                "Status" => {
                    status_ok = value.split_whitespace().last() == Some("installed")
                        && value.contains("install ok");
                }
                _ => {}
            }
        }

        let name = name?;
        if !status_ok || essential {
            return None;
        }
        if let Some(section) = &section {
            let short = section.rsplit('/').next().unwrap_or(section);
            if SKIPPED_SECTIONS.contains(&short) {
                return None;
            }
        }

        Some(InstalledApp {
            name: name.clone(),
            version: version.unwrap_or_default(),
            publisher: maintainer.unwrap_or_default(),
            // dpkg 不记录安装目录，体积只能用包管理器自己算的 Installed-Size
            install_location: None,
            estimated_bytes: installed_size.map(|kb| kb.saturating_mul(1024)),
            measured_bytes: None,
            install_date: None,
            machine_wide: true,
            no_remove: false,
            uninstall: UninstallAction::PackageManager {
                command: format!("apt-get remove --purge -y {name}"),
                packages: vec![name],
            },
        })
    }

    // ---- snap ----

    fn snap_packages() -> Vec<InstalledApp> {
        let Some(text) = run_capture("snap", &["list"]) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for line in text.lines().skip(1) {
            let columns: Vec<&str> = line.split_whitespace().collect();
            if columns.len() < 4 {
                continue;
            }
            let name = columns[0];
            let version = columns[1];
            let publisher = columns.get(4).copied().unwrap_or("");
            out.push(InstalledApp {
                name: name.to_string(),
                version: version.to_string(),
                publisher: publisher.to_string(),
                install_location: None,
                estimated_bytes: None,
                measured_bytes: None,
                install_date: None,
                machine_wide: true,
                no_remove: false,
                uninstall: UninstallAction::PackageManager {
                    command: format!("snap remove {name}"),
                    packages: vec![name.to_string()],
                },
            });
        }
        out
    }

    // ---- flatpak ----

    fn flatpak_packages() -> Vec<InstalledApp> {
        let Some(text) = run_capture(
            "flatpak",
            &["list", "--app", "--columns=application,name,version,size"],
        ) else {
            return Vec::new();
        };

        let mut out = Vec::new();
        for line in text.lines() {
            let columns: Vec<&str> = line.split('\t').collect();
            if columns.len() < 4 {
                continue;
            }
            let id = columns[0].trim();
            let display = columns[1].trim();
            let version = columns[2].trim();
            let size = parse_flatpak_size(columns[3].trim());
            out.push(InstalledApp {
                name: if display.is_empty() { id } else { display }.to_string(),
                version: version.to_string(),
                publisher: id.to_string(),
                install_location: None,
                estimated_bytes: size,
                measured_bytes: None,
                install_date: None,
                machine_wide: true,
                no_remove: false,
                uninstall: UninstallAction::PackageManager {
                    command: format!("flatpak uninstall -y {id}"),
                    packages: vec![id.to_string()],
                },
            });
        }
        out
    }

    /// flatpak 的 `size` 列形如 `123.4 MB`。
    fn parse_flatpak_size(text: &str) -> Option<u64> {
        let mut parts = text.split_whitespace();
        let value: f64 = parts.next()?.parse().ok()?;
        let unit = parts.next()?.to_ascii_lowercase();
        let factor = match unit.as_str() {
            "b" => 1.0,
            "kb" | "kib" => 1024.0,
            "mb" | "mib" => 1024.0 * 1024.0,
            "gb" | "gib" => 1024.0 * 1024.0 * 1024.0,
            _ => return None,
        };
        Some((value * factor) as u64)
    }

    fn run_capture(program: &str, args: &[&str]) -> Option<String> {
        let output = std::process::Command::new(program)
            .args(args)
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        Some(String::from_utf8_lossy(&output.stdout).to_string())
    }
}

// ---------------------------------------------------------------------------
// Windows 实现
// ---------------------------------------------------------------------------

#[cfg(windows)]
mod win {
    use std::os::windows::ffi::OsStringExt;

    use windows_sys::Win32::Foundation::{ERROR_NO_MORE_ITEMS, ERROR_SUCCESS};
    use windows_sys::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY,
        REG_DWORD, REG_EXPAND_SZ, REG_MULTI_SZ, REG_QWORD, REG_SZ, REG_VALUE_TYPE, RegCloseKey,
        RegEnumKeyExW, RegOpenKeyExW, RegQueryValueExW,
    };

    use super::{InstalledApp, UninstallAction};

    const UNINSTALL_KEY: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall";

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn open(root: HKEY, sub: &str, extra: u32) -> Option<HKEY> {
        let path = wide(sub);
        let mut key: HKEY = std::ptr::null_mut();
        // SAFETY: `path` 以 NUL 结尾，`key` 是栈上的有效输出指针。
        let rc = unsafe { RegOpenKeyExW(root, path.as_ptr(), 0, KEY_READ | extra, &mut key) };
        if rc == ERROR_SUCCESS { Some(key) } else { None }
    }

    fn subkeys(key: HKEY) -> Vec<String> {
        let mut out = Vec::new();
        let mut index = 0u32;
        let mut buf = vec![0u16; 512];

        loop {
            let mut len = buf.len() as u32;
            // SAFETY: 缓冲区长度通过 `len` 如实告知系统；其余输出参数传空即可。
            let rc = unsafe {
                RegEnumKeyExW(
                    key,
                    index,
                    buf.as_mut_ptr(),
                    &mut len,
                    std::ptr::null(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            };
            if rc == ERROR_NO_MORE_ITEMS || rc != ERROR_SUCCESS {
                break;
            }
            out.push(String::from_utf16_lossy(&buf[..len as usize]));
            index += 1;
        }

        out
    }

    fn raw_value(key: HKEY, name: &str) -> Option<(REG_VALUE_TYPE, Vec<u8>)> {
        let name_w = wide(name);
        let mut ty: REG_VALUE_TYPE = 0;
        let mut size: u32 = 0;

        // 第一次只问大小
        // SAFETY: lpData 传空是官方支持的"仅查询大小"用法。
        let rc = unsafe {
            RegQueryValueExW(
                key,
                name_w.as_ptr(),
                std::ptr::null(),
                &mut ty,
                std::ptr::null_mut(),
                &mut size,
            )
        };
        if rc != ERROR_SUCCESS || size == 0 {
            return None;
        }

        let mut data = vec![0u8; size as usize];
        // SAFETY: `data` 至少有 `size` 字节，容量由 `size` 如实告知系统。
        let rc = unsafe {
            RegQueryValueExW(
                key,
                name_w.as_ptr(),
                std::ptr::null(),
                &mut ty,
                data.as_mut_ptr(),
                &mut size,
            )
        };
        if rc != ERROR_SUCCESS {
            return None;
        }
        data.truncate(size as usize);
        Some((ty, data))
    }

    fn utf16_to_string(data: &[u8]) -> String {
        let units: Vec<u16> = data
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        let end = units.iter().position(|&u| u == 0).unwrap_or(units.len());
        std::ffi::OsString::from_wide(&units[..end])
            .to_string_lossy()
            .to_string()
    }

    fn string_value(key: HKEY, name: &str) -> Option<String> {
        let (ty, data) = raw_value(key, name)?;
        let text = match ty {
            REG_SZ | REG_EXPAND_SZ => utf16_to_string(&data),
            // 多字符串：用分号接起来，够看了
            REG_MULTI_SZ => {
                let units: Vec<u16> = data
                    .chunks_exact(2)
                    .map(|c| u16::from_le_bytes([c[0], c[1]]))
                    .collect();
                let mut parts = Vec::new();
                for chunk in units.split(|&u| u == 0) {
                    if chunk.is_empty() {
                        continue;
                    }
                    parts.push(
                        std::ffi::OsString::from_wide(chunk)
                            .to_string_lossy()
                            .to_string(),
                    );
                }
                parts.join("; ")
            }
            _ => return None,
        };

        let text = text.trim().to_string();
        if text.is_empty() {
            return None;
        }
        Some(if ty == REG_EXPAND_SZ {
            expand_env(&text)
        } else {
            text
        })
    }

    fn dword_value(key: HKEY, name: &str) -> Option<u32> {
        let (ty, data) = raw_value(key, name)?;
        if (ty == REG_DWORD || ty == REG_QWORD) && data.len() >= 4 {
            Some(u32::from_le_bytes([data[0], data[1], data[2], data[3]]))
        } else {
            None
        }
    }

    fn qword_value(key: HKEY, name: &str) -> Option<u64> {
        let (ty, data) = raw_value(key, name)?;
        if ty == REG_QWORD && data.len() >= 8 {
            let mut buf = [0u8; 8];
            buf.copy_from_slice(&data[..8]);
            Some(u64::from_le_bytes(buf))
        } else {
            None
        }
    }

    /// 展开 `%ProgramFiles%` 这类环境变量。
    fn expand_env(text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        let mut rest = text;

        while let Some(start) = rest.find('%') {
            out.push_str(&rest[..start]);
            let after = &rest[start + 1..];
            match after.find('%') {
                Some(end) => {
                    let name = &after[..end];
                    match std::env::var(name) {
                        Ok(value) => out.push_str(&value),
                        Err(_) => {
                            out.push('%');
                            out.push_str(name);
                            out.push('%');
                        }
                    }
                    rest = &after[end + 1..];
                }
                None => {
                    out.push_str(&rest[start..]);
                    return out;
                }
            }
        }

        out.push_str(rest);
        out
    }

    /// 读一条 Uninstall 记录；返回 `None` 表示这条应该被忽略。
    fn read_entry(key: HKEY, machine_wide: bool) -> Option<InstalledApp> {
        let name = string_value(key, "DisplayName")?;

        // 系统组件、补丁、附属组件都不是"装了一个软件"
        if dword_value(key, "SystemComponent").unwrap_or(0) == 1 {
            return None;
        }
        if string_value(key, "ParentKeyName").is_some()
            || string_value(key, "ParentDisplayName").is_some()
        {
            return None;
        }
        if let Some(release) = string_value(key, "ReleaseType") {
            let lower = release.to_ascii_lowercase();
            if lower.contains("update") || lower.contains("hotfix") || lower.contains("patch") {
                return None;
            }
        }

        let install_location = string_value(key, "InstallLocation")
            .map(|s| s.trim().trim_matches('"').to_string())
            .filter(|s| !s.is_empty())
            .map(std::path::PathBuf::from);

        // EstimateSize 的单位是 KB；优先用 QWORD（少数安装器会写成 64 位）
        let estimated_bytes = qword_value(key, "EstimatedSize")
            .or_else(|| dword_value(key, "EstimatedSize").map(u64::from))
            .filter(|kb| *kb > 0)
            .map(|kb| kb.saturating_mul(1024));

        let no_remove = dword_value(key, "NoRemove").unwrap_or(0) == 1;
        let command = string_value(key, "UninstallString")
            .or_else(|| string_value(key, "QuietUninstallString"))
            .filter(|s| !s.trim().is_empty());

        Some(InstalledApp {
            name,
            version: string_value(key, "DisplayVersion").unwrap_or_default(),
            publisher: string_value(key, "Publisher").unwrap_or_default(),
            install_location,
            estimated_bytes,
            measured_bytes: None,
            install_date: string_value(key, "InstallDate"),
            machine_wide,
            no_remove,
            uninstall: match command {
                Some(command) => UninstallAction::Vendor(command),
                None => UninstallAction::None,
            },
        })
    }

    fn collect(root: HKEY, extra: u32, machine_wide: bool) -> Vec<InstalledApp> {
        let Some(parent) = open(root, UNINSTALL_KEY, extra) else {
            return Vec::new();
        };

        let mut out = Vec::new();
        for sub in subkeys(parent) {
            let Some(key) = open(parent, &sub, extra) else {
                continue;
            };
            if let Some(app) = read_entry(key, machine_wide) {
                out.push(app);
            }
            // SAFETY: 句柄来自上面成功的 RegOpenKeyExW，且只关闭一次。
            unsafe { RegCloseKey(key) };
        }

        // SAFETY: 同上。
        unsafe { RegCloseKey(parent) };
        out
    }

    pub fn enumerate() -> Vec<InstalledApp> {
        let mut apps = Vec::new();
        // HKLM 六十四位视图 / 三十二位视图，以及当前用户
        apps.extend(collect(HKEY_LOCAL_MACHINE, KEY_WOW64_64KEY, true));
        apps.extend(collect(HKEY_LOCAL_MACHINE, KEY_WOW64_32KEY, true));
        apps.extend(collect(HKEY_CURRENT_USER, KEY_WOW64_64KEY, false));
        apps.extend(collect(HKEY_CURRENT_USER, KEY_WOW64_32KEY, false));
        apps
    }
}

// ---------------------------------------------------------------------------
// 测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_command_line_handles_quotes_and_args() {
        assert_eq!(
            split_command_line(r#""C:\Program Files\App\uninstall.exe" /S"#),
            (
                r"C:\Program Files\App\uninstall.exe".to_string(),
                "/S".to_string()
            )
        );
        assert_eq!(
            split_command_line(r#"MsiExec.exe /X{1234-5678}"#),
            ("MsiExec.exe".to_string(), "/X{1234-5678}".to_string())
        );
        assert_eq!(
            split_command_line("  notepad.exe  "),
            ("notepad.exe".to_string(), String::new())
        );
    }

    #[test]
    fn dir_size_sums_nested_files_only() {
        let root = std::env::temp_dir().join(format!("cleanc-size-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("a").join("b")).unwrap();
        std::fs::write(root.join("one.bin"), vec![0u8; 1000]).unwrap();
        std::fs::write(root.join("a").join("two.bin"), vec![0u8; 2000]).unwrap();
        std::fs::write(root.join("a").join("b").join("three.bin"), vec![0u8; 3000]).unwrap();

        assert_eq!(dir_size(&root), 6000);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn broad_install_locations_are_rejected() {
        let too_broad = if cfg!(windows) {
            vec![
                r"C:\",
                r"C:\Program Files",
                r"C:\Program Files (x86)",
                r"C:\ProgramData",
            ]
        } else {
            vec![
                "/",
                "/Applications",
                "/usr",
                "/usr/share",
                "/opt",
                "/System",
            ]
        };
        for path in too_broad {
            assert!(!is_measurable(Path::new(path)), "{path} 不该被当成软件目录");
        }
        // 不存在或过浅的路径也不该被当成软件目录
        let missing = if cfg!(windows) {
            r"C:\definitely-not-a-real-dir"
        } else {
            "/definitely-not-a-real-dir"
        };
        assert!(!is_measurable(Path::new(missing)));
    }

    /// 造一条记录，方便测体积与卸载动作的展示逻辑。
    fn sample(uninstall: UninstallAction) -> InstalledApp {
        InstalledApp {
            name: "x".into(),
            version: String::new(),
            publisher: String::new(),
            install_location: None,
            estimated_bytes: None,
            measured_bytes: None,
            install_date: None,
            machine_wide: true,
            no_remove: false,
            uninstall,
        }
    }

    #[test]
    fn install_date_is_formatted() {
        let mut app = sample(UninstallAction::None);
        app.install_date = Some("20260131".into());
        assert_eq!(app.install_date_label().as_deref(), Some("2026-01-31"));

        app.install_date = None;
        assert!(app.install_date_label().is_none());
    }

    #[test]
    fn size_prefers_measured_over_estimated() {
        let mut app = sample(UninstallAction::None);
        app.estimated_bytes = Some(4096);
        assert_eq!(app.size_bytes(), Some(4096));
        assert!(app.size_is_estimated());

        app.measured_bytes = Some(8192);
        assert_eq!(app.size_bytes(), Some(8192));
        assert!(!app.size_is_estimated());
    }

    #[test]
    fn uninstall_actions_are_reported_correctly() {
        let none = sample(UninstallAction::None);
        assert!(!none.can_uninstall());
        assert_eq!(none.uninstall_display(), "（无）");

        let no_remove = sample(UninstallAction::Vendor("setup.exe /remove".into()));
        assert!(no_remove.can_uninstall());

        let mut blocked = no_remove.clone();
        blocked.no_remove = true;
        assert!(!blocked.can_uninstall(), "来源明确禁止卸载时不能放开按钮");

        let bundle = sample(UninstallAction::TrashBundle(PathBuf::from(
            "/Applications/Thing.app",
        )));
        assert!(bundle.can_uninstall());
        assert!(bundle.uninstall_display().contains("Thing.app"));
        assert!(bundle.uninstall_command().is_none(), "移入回收站没有命令行");

        let package = sample(UninstallAction::PackageManager {
            command: "apt-get remove --purge -y htop".into(),
            packages: vec!["htop".into()],
        });
        assert_eq!(
            package.uninstall_command(),
            Some("apt-get remove --purge -y htop")
        );
    }
}
