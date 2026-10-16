//! 大文件 / 大目录的归类与处置建议。
//!
//! 扫描层只回答「哪些东西大」，这里回答「大的东西能不能删」。
//!
//! 全模块都是**纯函数、不做任何 IO**（只读环境变量构造保护名单），
//! 所以可以放心地在大列表逐行渲染时调用，也便于单测覆盖。

use std::path::Path;

use crate::safety;

/// 条目所属的类别。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// 缓存、临时文件、构建产物：删掉之后程序会自动重建。
    Cache,
    /// 安装包、压缩包、绿色软件：装好 / 解压完通常就没用了。
    Installer,
    /// 大体积媒体与磁盘镜像：删了不好找回，得先确认内容。
    Media,
    /// 系统或已安装程序的一部分：直接删可能让程序起不来。
    System,
    /// 普通文件：本工具判断不了它的价值，交给用户。
    Personal,
}

impl Kind {
    /// 全部类别，顺序与 [`Kind::index`] 一致。
    pub const ALL: [Kind; 5] = [
        Kind::Cache,
        Kind::Installer,
        Kind::Media,
        Kind::System,
        Kind::Personal,
    ];

    /// 稳定的下标，可以直接拿来索引 `[T; 5]`。
    pub fn index(self) -> usize {
        match self {
            Kind::Cache => 0,
            Kind::Installer => 1,
            Kind::Media => 2,
            Kind::System => 3,
            Kind::Personal => 4,
        }
    }

    /// 分类名（用于详情展示）。
    pub fn label(self) -> &'static str {
        match self {
            Kind::Cache => "缓存 / 构建产物",
            Kind::Installer => "安装包 / 压缩包",
            Kind::Media => "大体积媒体",
            Kind::System => "系统 / 程序文件",
            Kind::Personal => "普通文件",
        }
    }

    /// 胶囊上的短标签。
    pub fn short(self) -> &'static str {
        match self {
            Kind::Cache => "可清理",
            Kind::Installer => "可清理",
            Kind::Media => "先确认",
            Kind::System => "别直接删",
            Kind::Personal => "自行确认",
        }
    }

    /// 一句话处置建议。
    ///
    /// 刻意不复述 [`Kind::label`] 的内容 —— 界面上两者会拼在一起显示成
    /// 「缓存 / 构建产物 · 删掉后相关程序会按需重新生成…」。
    pub fn advice(self) -> &'static str {
        match self {
            Kind::Cache => {
                "删掉后相关程序会按需重新生成。确认当前没有正在编译或下载的任务，就可以放心清理。"
            }
            Kind::Installer => {
                "装好、解压完之后原始文件通常就没用了。确认软件已经正常安装，再移入回收站。"
            }
            Kind::Media => "这类内容删掉后不易找回，建议先打开目录确认内容，再决定是否移入回收站。",
            Kind::System => {
                "不要直接删除，否则可能让程序无法启动。需要腾空间请走程序自带的卸载或清理功能。"
            }
            Kind::Personal => {
                "本工具无法判断它对你是否还有用。建议先打开目录看一眼，再决定是否移入回收站。"
            }
        }
    }
}

/// 受保护条目的建议文本（优先级高于 [`Kind`]）。
pub const PROTECTED_ADVICE: &str = "位于本工具的保护范围内（系统关键目录或你的个人资料目录），不会提供删除操作。如需处理请手动在系统自带的文件管理器里进行。";

/// 路径里带这些片段的目录，通常是能重建的缓存 / 构建产物。
///
/// 标记同时收录 Windows（反斜杠）与 Unix（斜杠）两套写法：这样同一份规则
/// 在任何平台上都能读懂"看起来像缓存"的路径，交叉编译时也不需要 `cfg` 分叉。
/// 比较前会先把路径规范化、折叠成小写、目录场景下补一个结尾分隔符。
const CACHE_MARKERS: &[&str] = &[
    // ---- 临时目录 ----
    r"\temp\",
    r"\tmp\",
    "/tmp/",
    "/var/tmp/",
    // ---- 各种叫 cache 的目录 ----
    r"\cache\",
    r"\caches\",
    "/cache/",
    "/caches/",
    "/var/cache/",
    // 各种「以 -cache 结尾」的包管理器缓存：npm-cache、yarn-cache、pip-cache…
    r"-cache\",
    "-cache/",
    // ---- 构建产物 ----
    r"\node_modules\",
    "/node_modules/",
    r"\target\",
    "/target/",
    r"\__pycache__\",
    "/__pycache__/",
    r"\.gradle\",
    "/.gradle/",
    r"\.npm\",
    "/.npm/",
    r"\.nuget\",
    "/.nuget/",
    r"\.m2\",
    "/.m2/",
    r"\.pnpm-store\",
    "/.pnpm-store/",
    r"\.cargo\registry\",
    "/.cargo/registry/",
    r"\.venv\",
    "/.venv/",
    r"\.cache\",
    "/.cache/",
    r"\.pytest_cache\",
    "/.pytest_cache/",
    r"\.mypy_cache\",
    "/.mypy_cache/",
    // ---- 应用自建的缓存 / 日志 / 崩溃转储 ----
    r"\gpucache\",
    "/gpucache/",
    r"\shadercache\",
    "/shadercache/",
    r"\code cache\",
    "/code cache/",
    r"\crashdumps\",
    "/crashdumps/",
    r"\logs\",
    "/logs/",
    r"\downloaded installations\",
    "/downloaded installations/",
];

/// 路径里带这些片段的条目属于系统或已安装程序。
const SYSTEM_MARKERS: &[&str] = &[
    // Windows
    r"\windows\",
    "/windows/",
    r"\program files\",
    "/program files/",
    r"\program files (x86)\",
    "/program files (x86)/",
    r"\programdata\",
    "/programdata/",
    r"\systemapps\",
    "/systemapps/",
    r"\$recycle.bin\",
    "/$recycle.bin/",
    r"\system volume information\",
    "/system volume information/",
    // macOS / Linux
    "/usr/",
    "/bin/",
    "/sbin/",
    "/lib/",
    "/lib64/",
    "/etc/",
    "/boot/",
    "/opt/",
    "/var/lib/",
    "/var/db/",
    "/system/",
    "/library/",
    "/applications/",
    "/cores/",
    "/private/",
];

/// 判定一个**文件**该怎么处理。
pub fn for_file(path: &Path) -> Kind {
    let key = safety::norm(path).to_ascii_lowercase();

    if contains_any(&key, CACHE_MARKERS) {
        return Kind::Cache;
    }
    if contains_any(&key, SYSTEM_MARKERS) {
        return Kind::System;
    }

    let ext = extension_lower(path);
    // 可执行文件与动态库要单独判断：落在 AppData 里的 `GitHubDesktop.exe`、
    // `chrome.dll` 是**已安装程序的一部分**，按"安装包"处理会诱导用户把软件弄坏。
    match ext.as_str() {
        "exe" => {
            if looks_like_installer(path) {
                Kind::Installer
            } else {
                Kind::System
            }
        }
        "dll" | "sys" | "ocx" | "node" => Kind::System,
        // Unix 的库与内核模块同理：它们几乎总是某个程序的一部分
        "so" | "dylib" | "ko" | "a" => Kind::System,
        _ => by_extension(&ext).unwrap_or(Kind::Personal),
    }
}

/// 文件名是否像"安装包"：`setup.exe`、`xxx-installer.exe`、`app_install.exe`…
///
/// 只认得出这一类才敢说"装完就没用了"，其余可执行文件一律按程序本体对待。
fn looks_like_installer(path: &Path) -> bool {
    const HINTS: &[&str] = &[
        "setup",
        "install",
        "uninst",
        "bootstrapper",
        "webinstall",
        "-full",
        "portable",
    ];
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    let name = name.to_ascii_lowercase();
    HINTS.iter().any(|hint| name.contains(hint))
}

/// 判定一个**目录**该怎么处理。
///
/// 只看路径特征，不做内容分析 —— 这样一次扫描下来不需要为每个文件
/// 都算一遍分类，代价可以忽略。
pub fn for_dir(path: &Path) -> Kind {
    let key = dir_key(path);

    if contains_any(&key, CACHE_MARKERS) {
        return Kind::Cache;
    }
    if contains_any(&key, SYSTEM_MARKERS) {
        return Kind::System;
    }
    Kind::Personal
}

/// 按扩展名归类（目录不适用）。返回 `None` 表示没有明确倾向。
///
/// 这里刻意**只收"看一眼就知道是什么"的类型**：
/// * 可执行文件与动态库由 [`for_file`] 单独判断，不进这张表；
/// * `.zip` / `.rar` / `.bak` 这类"可能是资料也可能是垃圾"的格式也一律不收，
///   留给 [`Kind::Personal`]，让用户自己看一眼 —— 宁可少建议，也不要误导。
fn by_extension(ext: &str) -> Option<Kind> {
    let kind = match ext {
        // 磁盘镜像
        "iso" | "img" | "vhd" | "vhdx" | "vmdk" | "qcow2" | "wim" | "esd" => Kind::Media,
        // 影音
        "mp4" | "mkv" | "avi" | "mov" | "flv" | "wmv" | "webm" | "m4v" | "mpg" | "mpeg" | "vob"
        | "rmvb" => Kind::Media,
        // 明确的安装包格式（Windows / macOS / Linux 都收）
        "msi" | "msix" | "appx" | "msu" | "apk" | "deb" | "rpm" | "dmg" | "pkg" | "appimage"
        | "flatpak" | "snap" | "run" => Kind::Installer,
        // 一眼就能看出是中间产物
        "tmp" | "temp" | "log" | "dmp" | "dump" | "cache" | "chk" | "etl" => Kind::Cache,
        _ => return None,
    };
    Some(kind)
}

/// 规范化后的目录路径，补一个结尾分隔符，这样 `\temp\` 这类片段才匹配得上。
///
/// 分隔符按平台取（Windows `\`、Unix `/`），同时统一折成小写 ——
/// Linux 的路径本来区分大小写，但这里只是启发式分类，放宽一点更不容易漏判。
fn dir_key(path: &Path) -> String {
    let mut key = safety::norm(path).to_ascii_lowercase();
    if !key.ends_with(safety::SEP) {
        key.push(safety::SEP);
    }
    key
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

/// 小写扩展名（不含点）。只有结果行会调用，所以这里的分配可以忽略不计。
fn extension_lower(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// 测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// 测试用的路径构造器。
    ///
    /// 断言里统一写成 Windows 风格（`C:\x\Temp`），在 Unix 上会先把反斜杠换成
    /// 斜杠 —— 这样同一批用例在两个平台上都真的跑得起来，而不是被 `cfg` 裁掉。
    #[cfg(windows)]
    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }
    #[cfg(unix)]
    fn p(s: &str) -> PathBuf {
        PathBuf::from(s.replace('\\', "/"))
    }

    #[test]
    fn cache_paths_win_over_everything_else() {
        // 即使落在 Windows 目录里，只要命中缓存特征就按缓存处理
        assert_eq!(for_file(&p(r"C:\Windows\Temp\a.log")), Kind::Cache);
        assert_eq!(for_file(&p(r"C:\Windows\Temp\big.bin")), Kind::Cache);
        assert_eq!(for_dir(&p(r"C:\Users\me\AppData\Local\Temp")), Kind::Cache);
    }

    #[test]
    fn build_artifacts_are_cache() {
        assert_eq!(for_dir(&p(r"D:\p\proj\node_modules")), Kind::Cache);
        assert_eq!(for_dir(&p(r"F:\rust\clean_c\target")), Kind::Cache);
        assert_eq!(for_dir(&p(r"C:\Users\me\.cache")), Kind::Cache);
        // 以 -cache 结尾的包管理器缓存
        assert_eq!(
            for_dir(&p(r"C:\Users\me\AppData\Local\npm-cache")),
            Kind::Cache
        );
        assert_eq!(for_dir(&p(r"C:\Users\me\.pnpm-store")), Kind::Cache);
    }

    #[test]
    fn system_trees_are_flagged() {
        assert_eq!(for_file(&p(r"C:\Program Files\App\app.dll")), Kind::System);
        assert_eq!(for_dir(&p(r"C:\Windows\assembly")), Kind::System);
        assert_eq!(for_dir(&p(r"C:\ProgramData\Microsoft")), Kind::System);
    }

    #[test]
    fn extensions_are_classified() {
        assert_eq!(for_file(&p(r"D:\a\ubuntu.iso")), Kind::Media);
        assert_eq!(for_file(&p(r"D:\a\movie.mkv")), Kind::Media);
        assert_eq!(for_file(&p(r"D:\a\office.msi")), Kind::Installer);
        assert_eq!(for_file(&p(r"D:\a\huge.dmp")), Kind::Cache);
        assert_eq!(for_file(&p(r"D:\a\notes.txt")), Kind::Personal);
        assert_eq!(for_file(&p(r"D:\a\noextension")), Kind::Personal);
    }

    #[test]
    fn installed_program_binaries_are_never_called_installers() {
        // 这些是已安装程序的本体，说成"装完就没用了"会直接诱导用户弄坏软件
        assert_eq!(
            for_file(&p(
                r"C:\Users\me\AppData\Local\GitHubDesktop\app-3.6.4\GitHubDesktop.exe"
            )),
            Kind::System
        );
        assert_eq!(
            for_file(&p(
                r"C:\Users\me\AppData\Local\ms-playwright\chromium-1234\chrome-headless-shell.exe"
            )),
            Kind::System
        );
        assert_eq!(
            for_file(&p(r"C:\Users\me\AppData\Local\JxBrowser\8.9.0\chrome.dll")),
            Kind::System
        );

        // 但货真价实的安装包仍然要被认出来
        assert_eq!(
            for_file(&p(r"D:\downloads\VSCodeUserSetup-x64.exe")),
            Kind::Installer
        );
        assert_eq!(
            for_file(&p(r"D:\下载\WeChatInstaller.exe")),
            Kind::Installer
        );
    }

    #[test]
    fn ambiguous_formats_stay_personal() {
        // 压缩包和备份可能是重要资料，不该被贴上"可清理"
        for name in [
            r"D:\a\src.zip",
            r"D:\a\data.7z",
            r"D:\a\conf.bak",
            r"D:\a\old.old",
        ] {
            assert_eq!(for_file(&p(name)), Kind::Personal, "{name}");
        }
    }

    #[test]
    fn extension_match_is_case_insensitive() {
        assert_eq!(for_file(&p(r"D:\a\UBUNTU.ISO")), Kind::Media);
        assert_eq!(for_file(&p(r"D:\a\Office.MSI")), Kind::Installer);
        assert_eq!(for_file(&p(r"D:\a\VerySetup.EXE")), Kind::Installer);
    }

    #[test]
    fn plain_personal_folder_stays_personal() {
        assert_eq!(for_dir(&p(r"C:\Users\me\Projects\demo")), Kind::Personal);
        assert_eq!(for_dir(&p(r"C:\Users\me\Documents")), Kind::Personal);
    }

    #[test]
    fn kind_indices_match_all() {
        for (position, kind) in Kind::ALL.iter().enumerate() {
            assert_eq!(kind.index(), position);
        }
    }

    #[test]
    fn dir_matching_does_not_need_a_real_trailing_slash() {
        // 同一个目录，带不带结尾斜杠都应该命中
        assert_eq!(for_dir(&p(r"C:\x\Temp\")), Kind::Cache);
        assert_eq!(for_dir(&p(r"C:\x\Temp")), Kind::Cache);
    }

    /// Unix 路径（macOS / Linux）也要能正确归类。
    #[cfg(unix)]
    #[test]
    fn unix_paths_are_classified() {
        use std::path::Path;
        // 缓存
        assert_eq!(for_dir(Path::new("/home/me/.cache")), Kind::Cache);
        assert_eq!(
            for_file(Path::new("/var/cache/apt/archives/a.deb")),
            Kind::Cache
        );
        assert_eq!(for_dir(Path::new("/var/tmp/build")), Kind::Cache);
        assert_eq!(for_dir(Path::new("/Users/me/Library/Caches")), Kind::Cache);
        assert_eq!(for_dir(Path::new("/home/me/proj/target")), Kind::Cache);
        assert_eq!(for_file(Path::new("/tmp/x.o")), Kind::Cache);
        // 系统
        assert_eq!(
            for_file(Path::new("/usr/lib/x86_64-linux-gnu/libz.so.1")),
            Kind::System
        );
        assert_eq!(for_file(Path::new("/usr/bin/ls")), Kind::System);
        assert_eq!(for_dir(Path::new("/var/lib/dpkg")), Kind::System);
        assert_eq!(for_dir(Path::new("/Applications/Safari.app")), Kind::System);
        // 安装包
        assert_eq!(
            for_file(Path::new("/home/me/Downloads/foo.deb")),
            Kind::Installer
        );
        assert_eq!(
            for_file(Path::new("/home/me/Downloads/Bar.AppImage")),
            Kind::Installer
        );
        // 大体积媒体
        assert_eq!(for_file(Path::new("/home/me/videos/a.mp4")), Kind::Media);
        // 普通文件
        assert_eq!(
            for_file(Path::new("/home/me/Documents/notes.txt")),
            Kind::Personal
        );
        assert_eq!(for_dir(Path::new("/home/me/projects/demo")), Kind::Personal);
    }
}
