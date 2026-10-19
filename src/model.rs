//! 全局数据模型：清理目标、扫描结果、清理报告。

use std::path::PathBuf;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// 风险等级 / 分组
// ---------------------------------------------------------------------------

/// 清理项的风险等级。等级越高，删除前越需要确认。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum RiskLevel {
    /// 公认安全的缓存 / 临时文件，删除后系统或程序会自动重建
    Safe,
    /// 需要管理员权限，或删除后有轻微副作用（例如首次启动变慢）
    Caution,
    /// 可能影响系统功能或回滚能力，默认不勾选
    Advanced,
}

impl RiskLevel {
    pub const ALL: [RiskLevel; 3] = [RiskLevel::Safe, RiskLevel::Caution, RiskLevel::Advanced];

    pub fn label(self) -> &'static str {
        match self {
            RiskLevel::Safe => "安全",
            RiskLevel::Caution => "谨慎",
            RiskLevel::Advanced => "高风险",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            RiskLevel::Safe => "删除后系统或程序会自动重建，无副作用",
            RiskLevel::Caution => "需要管理员权限，或删除后首次启动会稍慢",
            RiskLevel::Advanced => "可能影响系统功能或回滚能力，请确认后再删除",
        }
    }
}

/// 界面上的分组。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Group {
    UserFiles,
    SystemFiles,
    Browsers,
    Apps,
    Advanced,
    /// 微信的日志 / 缩略图等缓存（专项清理页）
    Wechat,
    /// QQ 的日志 / 聊天图片视频缓存（专项清理页）
    Qq,
}

impl Group {
    pub const ALL: [Group; 7] = [
        Group::UserFiles,
        Group::SystemFiles,
        Group::Browsers,
        Group::Apps,
        Group::Advanced,
        Group::Wechat,
        Group::Qq,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Group::UserFiles => "用户临时文件",
            Group::SystemFiles => "系统文件与更新",
            Group::Browsers => "浏览器缓存",
            Group::Apps => "应用 / 显卡缓存",
            Group::Advanced => "高级项（默认关闭）",
            Group::Wechat => "微信缓存",
            Group::Qq => "QQ 缓存",
        }
    }
}

// ---------------------------------------------------------------------------
// 清理目标定义
// ---------------------------------------------------------------------------

/// 缓存目录 / 文件的名称匹配规则（一律对"小写文件名"做匹配）。
///
/// 三个平台的清理白名单共用这一套词汇，因此在某个平台上总有几个变体是用不到的，
/// 这里统一关掉 dead_code，免得交叉编译时刷屏。
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NameRule {
    Exact(String),
    Prefix(String),
    /// 后缀匹配，用来挑 `*.deb` / `*.log` 这类同族文件（目前只有 Linux 白名单用得到）
    Suffix(String),
}

#[allow(dead_code)]
impl NameRule {
    pub fn exact(name: &str) -> Self {
        NameRule::Exact(name.to_ascii_lowercase())
    }

    pub fn prefix(name: &str) -> Self {
        NameRule::Prefix(name.to_ascii_lowercase())
    }

    pub fn suffix(name: &str) -> Self {
        NameRule::Suffix(name.to_ascii_lowercase())
    }

    pub fn matches(&self, name_lower: &str) -> bool {
        match self {
            NameRule::Exact(n) => name_lower == n,
            NameRule::Prefix(n) => name_lower.starts_with(n),
            NameRule::Suffix(n) => name_lower.ends_with(n),
        }
    }
}

/// 一个清理根目录的处理方式。
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum RootMode {
    /// 递归删除该目录下的所有条目（目录本身保留）
    All,
    /// 只删除该目录下的一级文件，不递归进子目录
    ShallowFiles,
    /// 精确匹配的单个文件（例如 `C:\Windows\MEMORY.DMP`）
    ExactFile,
    /// 递归查找名称命中规则的条目；命中目录则清理其内容（目录本身保留），
    /// 命中文件则按年龄规则清理
    Matched(Vec<NameRule>),
    /// 回收站：走系统 API，不逐文件枚举
    RecycleBin,
}

/// 一个清理根目录。
#[derive(Debug, Clone)]
pub struct Root {
    pub path: PathBuf,
    pub mode: RootMode,
    /// 该根下需要跳过的一级子目录名（小写比较）
    pub skip_dirs: Vec<String>,
}

#[allow(dead_code)]
impl Root {
    pub fn all(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            mode: RootMode::All,
            skip_dirs: Vec::new(),
        }
    }

    pub fn shallow_files(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            mode: RootMode::ShallowFiles,
            skip_dirs: Vec::new(),
        }
    }

    pub fn file(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            mode: RootMode::ExactFile,
            skip_dirs: Vec::new(),
        }
    }

    pub fn matched(path: impl Into<PathBuf>, rules: Vec<NameRule>) -> Self {
        Self {
            path: path.into(),
            mode: RootMode::Matched(rules),
            skip_dirs: Vec::new(),
        }
    }

    /// 回收站：走系统 API，不逐文件枚举。
    ///
    /// 路径只用于展示，真正的清空动作按平台调用各自的接口：
    /// Windows 的 `SHEmptyRecycleBin`、macOS 的 `~/.Trash`、Linux 的 XDG Trash。
    pub fn recycle_bin() -> Self {
        #[cfg(windows)]
        let path = PathBuf::from(r"C:\$Recycle.Bin");
        #[cfg(target_os = "macos")]
        let path = crate::sysutil::home_dir()
            .unwrap_or_else(|| PathBuf::from("/"))
            .join(".Trash");
        #[cfg(target_os = "linux")]
        let path = crate::sysutil::user_data_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join("Trash");
        #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
        let path = PathBuf::from("/");

        Self {
            path,
            mode: RootMode::RecycleBin,
            skip_dirs: Vec::new(),
        }
    }

    pub fn with_skip_dirs(mut self, dirs: &[&str]) -> Self {
        self.skip_dirs = dirs.iter().map(|d| d.to_ascii_lowercase()).collect();
        self
    }

    /// 该根是否被跳过（一级子目录名过滤）。
    pub fn skips_dir(&self, name: &str) -> bool {
        if self.skip_dirs.is_empty() {
            return false;
        }
        let lower = name.to_ascii_lowercase();
        self.skip_dirs.iter().any(|d| *d == lower)
    }
}

/// 浏览器缓存目录的通用匹配规则集合（目前由 Windows 白名单使用）。
#[allow(dead_code)]
pub fn browser_cache_rules() -> Vec<NameRule> {
    [
        "cache",
        "code cache",
        "gpucache",
        "shadercache",
        "grshadercache",
        "graphitedawncache",
        "dawnwebgpucache",
        "dawncache",
        "media cache",
        "cachestorage",
        "application cache",
    ]
    .iter()
    .map(|n| NameRule::exact(n))
    .collect()
}

/// 一个清理类别。
#[derive(Debug, Clone)]
pub struct CleanTarget {
    /// 稳定标识，用于配置持久化
    pub id: &'static str,
    pub name: &'static str,
    pub desc: &'static str,
    pub group: Group,
    pub risk: RiskLevel,
    /// 是否通常需要管理员权限
    pub requires_admin: bool,
    /// 默认是否勾选
    pub default_on: bool,
    /// 是否应用"最小文件年龄"过滤
    pub age_filtered: bool,
    pub roots: Vec<Root>,
    /// 界面上的补充提示
    pub note: Option<&'static str>,
}

// ---------------------------------------------------------------------------
// 扫描结果
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct FileEntry {
    pub path: PathBuf,
    pub size: u64,
    pub modified: Option<SystemTime>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TargetStatus {
    /// 正常
    #[default]
    Ok,
    /// 目录存在但没有可清理内容
    Empty,
    /// 目录不存在
    Missing,
    /// 无权限访问（通常需要管理员）
    Denied,
    /// 部分成功
    Partial,
}

impl TargetStatus {
    pub fn label(self) -> &'static str {
        match self {
            TargetStatus::Ok => "可清理",
            TargetStatus::Empty => "已干净",
            TargetStatus::Missing => "不存在",
            TargetStatus::Denied => "需要管理员",
            TargetStatus::Partial => "部分可清理",
        }
    }
}

/// 单个类别的扫描结果。
#[derive(Debug, Clone)]
pub struct TargetResult {
    pub id: &'static str,
    pub files: u64,
    pub dirs: u64,
    pub bytes: u64,
    /// 因年龄 / 权限 / 链接等原因跳过的条目数
    pub skipped: u64,
    pub status: TargetStatus,
    pub message: Option<String>,
    /// 体积最大的若干文件，用于界面预览（上限 [`PREVIEW_LIMIT`]）
    pub preview: Vec<FileEntry>,
    /// 实际存在的清理根
    pub resolved_roots: Vec<PathBuf>,
}

/// 预览列表最多保留的条目数。
pub const PREVIEW_LIMIT: usize = 200;

impl TargetResult {
    pub fn new(id: &'static str) -> Self {
        Self {
            id,
            files: 0,
            dirs: 0,
            bytes: 0,
            skipped: 0,
            status: TargetStatus::Missing,
            message: None,
            preview: Vec::new(),
            resolved_roots: Vec::new(),
        }
    }

    /// 记录一个待清理条目。
    pub fn push_entry(&mut self, path: PathBuf, size: u64, modified: Option<SystemTime>) {
        self.preview.push(FileEntry {
            path,
            size,
            modified,
        });
        if self.preview.len() > PREVIEW_LIMIT * 2 {
            self.preview.sort_by(|a, b| b.size.cmp(&a.size));
            self.preview.truncate(PREVIEW_LIMIT);
        }
    }

    pub fn finalize_preview(&mut self) {
        self.preview.sort_by(|a, b| b.size.cmp(&a.size));
        self.preview.truncate(PREVIEW_LIMIT);
    }
}

/// 一次扫描的总体结果。
#[derive(Debug, Clone, Default)]
pub struct ScanSummary {
    pub results: Vec<TargetResult>,
    pub elapsed_ms: u64,
}

impl ScanSummary {
    pub fn result_for(&self, id: &str) -> Option<&TargetResult> {
        self.results.iter().find(|r| r.id == id)
    }

    pub fn total_bytes(&self) -> u64 {
        self.results.iter().map(|r| r.bytes).sum()
    }

    pub fn total_files(&self) -> u64 {
        self.results.iter().map(|r| r.files).sum()
    }
}

// ---------------------------------------------------------------------------
// 清理报告
// ---------------------------------------------------------------------------

/// 单个类别的清理结果。
#[derive(Debug, Clone, Default)]
pub struct CleanTargetReport {
    pub id: &'static str,
    pub name: String,
    pub files: u64,
    pub dirs: u64,
    pub bytes: u64,
    /// 因占用 / 权限 / 年龄被跳过的条目
    pub skipped: u64,
    /// 被安全层拦截的条目（正常情况下应为 0，用于自证安全）
    pub blocked: u64,
    pub failures: Vec<String>,
}

/// 一次清理的总体报告。
#[derive(Debug, Clone, Default)]
pub struct CleanReport {
    pub entries: Vec<CleanTargetReport>,
    pub deleted_files: u64,
    pub deleted_dirs: u64,
    pub freed_bytes: u64,
    pub skipped: u64,
    pub blocked: u64,
    pub failures: Vec<String>,
    pub log_path: Option<PathBuf>,
    pub elapsed_ms: u64,
    /// 是否为演练模式（只统计不删除）
    pub dry_run: bool,
    pub canceled: bool,
}

impl CleanReport {
    /// 把单个目标的结果并入总报告。
    pub fn merge(&mut self, entry: CleanTargetReport) {
        self.deleted_files += entry.files;
        self.deleted_dirs += entry.dirs;
        self.freed_bytes += entry.bytes;
        self.skipped += entry.skipped;
        self.blocked += entry.blocked;
        for line in &entry.failures {
            if self.failures.len() < MAX_FAILURE_LINES {
                self.failures.push(line.clone());
            }
        }
        self.entries.push(entry);
    }
}

/// 报告中最多保留的失败明细条数。
pub const MAX_FAILURE_LINES: usize = 40;

// ---------------------------------------------------------------------------
// 磁盘信息
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct DiskInfo {
    pub mount: String,
    pub total: u64,
    pub free: u64,
    pub available: u64,
}

impl DiskInfo {
    pub fn used(&self) -> u64 {
        self.total.saturating_sub(self.free)
    }

    pub fn used_ratio(&self) -> f32 {
        if self.total == 0 {
            0.0
        } else {
            self.used() as f32 / self.total as f32
        }
    }
}

// ---------------------------------------------------------------------------
// 大文件扫描
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct BigFile {
    pub path: PathBuf,
    pub size: u64,
    pub modified: Option<SystemTime>,
    /// 是否位于受保护区域（受保护文件不可删除，只能查看）
    pub protected: bool,
}

/// 扫描出来的大目录（体积为整棵子树的递归合计）。
#[derive(Debug, Clone)]
pub struct BigFolder {
    pub path: PathBuf,
    /// 子树内所有文件的体积之和
    pub size: u64,
    /// 子树内的文件个数
    pub files: u64,
    /// 子树内最近一次修改时间，用来判断这个目录还有没有在用
    pub modified: Option<SystemTime>,
    /// 是否位于受保护区域（不可删除）
    pub protected: bool,
    /// 是否已经被列表里某个更大的目录包含
    pub nested: bool,
}

// ---------------------------------------------------------------------------
// 工具函数
// ---------------------------------------------------------------------------

/// 把字节数格式化成人类可读的形式。
pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KB", "MB", "GB", "TB", "PB"];
    if bytes == 0 {
        return "0 B".to_string();
    }
    let mut value = bytes as f64;
    let mut unit = 0usize;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else if value >= 100.0 {
        format!("{:.0} {}", value, UNITS[unit])
    } else if value >= 10.0 {
        format!("{:.1} {}", value, UNITS[unit])
    } else {
        format!("{:.2} {}", value, UNITS[unit])
    }
}

/// 逗号分隔的千分位数字。
pub fn thousands(n: u64) -> String {
    let s = n.to_string();
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 && (bytes.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(*b as char);
    }
    out
}
