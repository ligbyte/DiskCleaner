//! 应用主状态机与页面路由。

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use eframe::egui;

use crate::apps::{self, AppHandle, AppMsg, InstalledApp};
use crate::bigfiles::{self, BigFileHandle, BigFileMsg};
use crate::cleaner::{self, CleanHandle, CleanMsg, CleanOptions};
use crate::duplicates::{self, DupGroup, DupHandle, DupMsg};
use crate::model::{
    BigFile, BigFolder, CleanReport, CleanTarget, DiskInfo, Group, ScanSummary, TargetResult,
};
use crate::scanner::{self, ScanHandle, ScanMsg, ScanOptions};
use crate::settings::{Settings, ThemePref};
use crate::sysutil;
use crate::targets;
use crate::theme;
use crate::ui;

// ---------------------------------------------------------------------------
// 页面
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Overview,
    /// 垃圾文件（原「智能清理」）：全部白名单目标的扫描 + 清理
    Clean,
    Apps,
    BigFiles,
    /// 大文件的完整列表页 —— 由「大文件」页的「查看更多」跳进来，不出现在左侧导航里
    BigDetail,
    /// 重复文件：在用户目录里找内容完全相同的文件
    Duplicates,
    /// 系统设置（系统级清理）：只看系统文件与高级项两个分组
    System,
    /// 微信专项清理
    Wechat,
    /// QQ 专项清理
    Qq,
    Settings,
    Safety,
}

impl Page {
    pub fn title(self) -> &'static str {
        match self {
            Page::Overview => "概览",
            Page::Clean => "垃圾文件",
            Page::Apps => "软件卸载",
            Page::BigFiles => "大文件",
            Page::BigDetail => "全部结果",
            Page::Duplicates => "重复文件",
            Page::System => "系统",
            Page::Wechat => "微信",
            Page::Qq => "QQ",
            Page::Settings => "设置",
            Page::Safety => "安全说明",
        }
    }

    pub fn subtitle(self) -> &'static str {
        match self {
            Page::Overview => "查看磁盘空间占用，一键开始安全清理",
            Page::Clean => "只清理白名单内的缓存与临时文件，每一项都可以单独确认",
            Page::Apps => "统计每个软件占用的空间，用它自己的方式把它卸载掉",
            Page::BigFiles => "找出占用空间最大的文件，自行决定如何处理",
            Page::BigDetail => "大文件与大文件夹的完整列表，可以直接在这里处理",
            Page::Duplicates => "在常用目录里找出内容完全相同的文件，删掉多余副本",
            Page::System => "系统更新残留、日志与高级清理项，动它们之前多看一眼",
            Page::Wechat => "清理微信的日志与缩略图缓存，聊天记录与接收的文件不受影响",
            Page::Qq => "清理 QQ 的日志与聊天图片视频缓存，聊天记录不受影响",
            Page::Settings => "调整清理策略、删除方式与界面偏好",
            Page::Safety => "这个工具凭什么说自己「安全」",
        }
    }

    /// 专项清理页的扫描范围（只扫这些分组）。返回 `None` 表示不是专项页。
    pub fn scope(self) -> Option<&'static [Group]> {
        match self {
            Page::System => Some(&[Group::SystemFiles, Group::Advanced]),
            Page::Wechat => Some(&[Group::Wechat]),
            Page::Qq => Some(&[Group::Qq]),
            _ => None,
        }
    }
}

/// 已安装软件列表的排序方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppsSort {
    SizeDesc,
    NameAsc,
    DateDesc,
}

impl AppsSort {
    pub const ALL: [AppsSort; 3] = [AppsSort::SizeDesc, AppsSort::NameAsc, AppsSort::DateDesc];

    pub fn label(self) -> &'static str {
        match self {
            AppsSort::SizeDesc => "按体积",
            AppsSort::NameAsc => "按名称",
            AppsSort::DateDesc => "按安装时间",
        }
    }
}

// ---------------------------------------------------------------------------
// 运行状态
// ---------------------------------------------------------------------------

pub enum ScanState {
    Idle,
    Running {
        handle: ScanHandle,
        started: Instant,
        current: String,
        finished: usize,
        total: usize,
        live: Vec<TargetResult>,
    },
    Done(ScanSummary),
}

pub enum CleanState {
    Idle,
    Running {
        handle: CleanHandle,
        started: Instant,
        current: String,
        files: u64,
        bytes: u64,
        finished: usize,
        total: usize,
    },
    Done(CleanReport),
}

pub enum BigState {
    Idle,
    Running {
        handle: BigFileHandle,
        started: Instant,
        scanned: u64,
        current: String,
    },
    Done {
        files: Vec<BigFile>,
        folders: Vec<BigFolder>,
        /// 去重后的处置建议汇总（扫描时算一次，界面直接读）
        totals: bigfiles::AdviceTotals,
        scanned: u64,
        elapsed_ms: u64,
        canceled: bool,
        /// 本次扫描实际统计了什么 —— 结果区要显示的是"上次扫描的选项"，
        /// 而不是用户当前刚改过的勾选状态。
        want_files: bool,
        want_folders: bool,
    },
}

/// 已安装软件的扫描状态。
pub enum AppScanState {
    /// 还没扫描过（进入页面时会自动开一次）
    Idle,
    Running {
        handle: AppHandle,
        started: Instant,
        phase: String,
        measured: usize,
        total: usize,
    },
    Done {
        elapsed_ms: u64,
        canceled: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Success,
    Warn,
    Error,
}

pub struct Toast {
    pub text: String,
    pub kind: ToastKind,
    pub at: Instant,
}

pub struct CleanConfirm {
    pub targets: Vec<CleanTarget>,
    pub opts: CleanOptions,
    pub files: u64,
    pub bytes: u64,
}

pub enum Dialog {
    ConfirmClean(Box<CleanConfirm>),
    ConfirmUninstall(Box<InstalledApp>),
    /// 删除整个大目录前的确认（比删单个文件重得多，必须拦一道）
    ConfirmRecycleFolder(Box<BigFolder>),
    /// 批量把重复文件移入回收站前的确认（paths, bytes）
    ConfirmDuplicates(Vec<std::path::PathBuf>, u64),
    Report(Box<CleanReport>),
    Message {
        title: String,
        body: String,
        kind: ToastKind,
    },
}

/// 重复文件扫描的状态。
pub enum DupState {
    Idle,
    Running {
        handle: DupHandle,
        started: Instant,
        scanned: u64,
        current: String,
    },
    Done {
        groups: Vec<DupGroup>,
        scanned: u64,
        elapsed_ms: u64,
        canceled: bool,
    },
}

// ---------------------------------------------------------------------------
// App
// ---------------------------------------------------------------------------

pub struct App {
    pub settings: Settings,
    pub targets: Vec<CleanTarget>,
    pub page: Page,

    /// 已勾选的清理项 id
    pub selected: BTreeSet<String>,
    /// 展开了文件明细的清理项 id
    pub expanded: BTreeSet<String>,
    /// 分组过滤（None = 全部）
    pub group_filter: Option<Group>,
    /// 只显示已勾选的项目
    pub only_checked: bool,

    pub admin: bool,
    pub disk: Option<DiskInfo>,
    disk_at: Instant,

    pub scan: ScanState,
    pub clean: CleanState,
    pub big: BigState,

    /// 重复文件扫描状态；勾选中要移入回收站的副本路径
    pub dup: DupState,
    pub dup_selected: std::collections::BTreeSet<PathBuf>,

    /// 已安装软件（扫描填充）
    pub apps: Vec<InstalledApp>,
    pub apps_scan: AppScanState,
    pub apps_query: String,
    pub apps_sort: AppsSort,
    pub apps_only_uninstallable: bool,

    pub big_min_mb: u64,
    pub big_roots: Vec<PathBuf>,
    pub big_roots_input: String,
    /// 是否统计大文件
    pub big_want_files: bool,
    /// 是否统计大文件夹
    pub big_want_folders: bool,
    /// 大文件夹列表是否隐藏"已被上层目录包含"的条目（纯视图偏好，不持久化）
    pub big_hide_nested: bool,
    /// 「全部结果」页的路径过滤关键词（纯视图状态，不持久化）
    pub big_detail_query: String,

    pub toasts: Vec<Toast>,
    pub dialog: Option<Dialog>,
    pub last_report: Option<CleanReport>,

    /// 顶栏品牌图标（`assets/disk.png`），首次绘制时才解码上传，之后一直复用。
    pub logo_tex: Option<egui::TextureHandle>,

    /// 概览页磁盘卡片的大图标（`assets/disk1.png`），同样是懒加载后复用。
    pub disk_tex: Option<egui::TextureHandle>,

    settings_dirty: bool,
    settings_saved_at: Instant,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let settings = Settings::load();
        theme::install_fonts(&cc.egui_ctx);
        theme::install(&cc.egui_ctx, settings.theme);

        let targets = targets::build();
        let selected = match &settings.selected_targets {
            Some(list) => list
                .iter()
                .filter(|id| targets.iter().any(|t| t.id == id.as_str()))
                .cloned()
                .collect(),
            None => targets
                .iter()
                .filter(|t| t.default_on)
                .map(|t| t.id.to_string())
                .collect(),
        };

        let big_roots = settings.big_file_roots();
        let big_min_mb = settings.big_file_min_mb;
        let big_want_files = settings.big_want_files;
        let big_want_folders = settings.big_want_folders;

        Self {
            settings,
            targets,
            page: Page::Overview,
            selected,
            expanded: BTreeSet::new(),
            group_filter: None,
            only_checked: false,
            admin: sysutil::is_admin(),
            disk: sysutil::disk_info(&sysutil::default_mount()),
            disk_at: Instant::now(),
            scan: ScanState::Idle,
            clean: CleanState::Idle,
            big: BigState::Idle,
            dup: DupState::Idle,
            dup_selected: std::collections::BTreeSet::new(),
            apps: Vec::new(),
            apps_scan: AppScanState::Idle,
            apps_query: String::new(),
            apps_sort: AppsSort::SizeDesc,
            apps_only_uninstallable: false,
            big_min_mb,
            big_roots,
            big_roots_input: String::new(),
            big_want_files,
            big_want_folders,
            big_hide_nested: false,
            big_detail_query: String::new(),
            toasts: Vec::new(),
            dialog: None,
            last_report: None,
            logo_tex: None,
            disk_tex: None,
            settings_dirty: false,
            settings_saved_at: Instant::now(),
        }
    }

    // -----------------------------------------------------------------
    // 小工具
    // -----------------------------------------------------------------

    pub fn toast(&mut self, text: impl Into<String>, kind: ToastKind) {
        self.toasts.push(Toast {
            text: text.into(),
            kind,
            at: Instant::now(),
        });
        if self.toasts.len() > 4 {
            self.toasts.remove(0);
        }
    }

    pub fn touch_settings(&mut self) {
        self.settings_dirty = true;
    }

    pub fn scanning(&self) -> bool {
        matches!(self.scan, ScanState::Running { .. })
    }

    pub fn cleaning(&self) -> bool {
        matches!(self.clean, CleanState::Running { .. })
    }

    pub fn big_scanning(&self) -> bool {
        matches!(self.big, BigState::Running { .. })
    }

    pub fn dup_scanning(&self) -> bool {
        matches!(self.dup, DupState::Running { .. })
    }

    pub fn busy(&self) -> bool {
        self.scanning() || self.cleaning() || self.big_scanning() || self.dup_scanning()
    }

    /// 当前扫描结果（若已完成）。
    pub fn summary(&self) -> Option<&ScanSummary> {
        match &self.scan {
            ScanState::Done(s) => Some(s),
            _ => None,
        }
    }

    /// 运行中的实时结果。
    pub fn live_results(&self) -> Option<&[TargetResult]> {
        match &self.scan {
            ScanState::Running { live, .. } => Some(live),
            _ => None,
        }
    }

    pub fn result_for(&self, id: &str) -> Option<TargetResult> {
        if let Some(s) = self.summary() {
            return s.result_for(id).cloned();
        }
        if let Some(live) = self.live_results() {
            return live.iter().find(|r| r.id == id).cloned();
        }
        None
    }

    /// 已勾选且扫描到内容的清理项 → (目标, 文件数, 体积)
    pub fn pending_selection(&self) -> (Vec<CleanTarget>, u64, u64) {
        let mut list = Vec::new();
        let mut files = 0u64;
        let mut bytes = 0u64;
        for t in &self.targets {
            if !self.selected.contains(t.id) {
                continue;
            }
            let Some(r) = self.result_for(t.id) else {
                continue;
            };
            if r.files == 0 {
                continue;
            }
            files += r.files;
            bytes += r.bytes;
            list.push(t.clone());
        }
        (list, files, bytes)
    }

    /// [`Self::pending_selection`] 的分组过滤版：专项清理页只提交自己范围内的目标。
    pub fn pending_selection_in(&self, groups: &[Group]) -> (Vec<CleanTarget>, u64, u64) {
        let (list, files, bytes) = self.pending_selection();
        let list = list
            .into_iter()
            .filter(|t| groups.contains(&t.group))
            .collect();
        (list, files, bytes)
    }

    /// 某几个分组里已扫描出、且已勾选的可清理体积（专项页的大按钮和概览入口用）。
    pub fn scoped_bytes(&self, groups: &[Group]) -> u64 {
        let mut bytes = 0u64;
        for t in &self.targets {
            if !groups.contains(&t.group) || !self.selected.contains(t.id) {
                continue;
            }
            if let Some(r) = self.result_for(t.id) {
                bytes += r.bytes;
            }
        }
        bytes
    }

    // -----------------------------------------------------------------
    // 扫描
    // -----------------------------------------------------------------

    pub fn start_scan(&mut self) {
        if self.scanning() {
            return;
        }
        let opts = ScanOptions {
            min_age_days: self.settings.min_age_days,
        };
        let handle = scanner::spawn(self.targets.clone(), opts);
        self.scan = ScanState::Running {
            handle,
            started: Instant::now(),
            current: String::new(),
            finished: 0,
            total: self.targets.len(),
            live: Vec::new(),
        };
    }

    /// 专项清理的**范围扫描**：只扫指定分组的目标。
    ///
    /// 结果仍写进同一份 `scan` 状态 —— 各专项页读结果时按分组过滤，
    /// 而「垃圾文件」页对没扫到的目标自然显示"未扫描"。
    pub fn start_scan_scoped(&mut self, groups: &[Group]) {
        if self.scanning() {
            return;
        }
        let targets: Vec<CleanTarget> = self
            .targets
            .iter()
            .filter(|t| groups.contains(&t.group))
            .cloned()
            .collect();
        if targets.is_empty() {
            self.toast(
                "本机没有找到对应的可清理目录（未安装或数据目录被自定义到了其它位置）",
                ToastKind::Info,
            );
            return;
        }
        let opts = ScanOptions {
            min_age_days: self.settings.min_age_days,
        };
        let handle = scanner::spawn(targets.clone(), opts);
        self.scan = ScanState::Running {
            handle,
            started: Instant::now(),
            current: String::new(),
            finished: 0,
            total: targets.len(),
            live: Vec::new(),
        };
    }

    pub fn cancel_scan(&mut self) {
        if let ScanState::Running { handle, .. } = &self.scan {
            handle.cancel();
        }
    }

    fn poll_scan(&mut self) {
        let state = std::mem::replace(&mut self.scan, ScanState::Idle);
        let ScanState::Running {
            handle,
            started,
            mut current,
            mut finished,
            mut total,
            mut live,
        } = state
        else {
            self.scan = state;
            return;
        };

        let mut done: Option<u64> = None;
        let mut canceled = false;

        loop {
            match handle.rx.try_recv() {
                Ok(ScanMsg::Started { total: t }) => total = t,
                Ok(ScanMsg::TargetStarted { name }) => current = name,
                Ok(ScanMsg::TargetFinished(result)) => {
                    finished += 1;
                    live.push(result);
                }
                Ok(ScanMsg::Finished { elapsed_ms }) => {
                    done = Some(elapsed_ms);
                    break;
                }
                Ok(ScanMsg::Canceled) => {
                    canceled = true;
                    break;
                }
                Err(_) => break,
            }
        }

        if done.is_some() || canceled {
            if canceled {
                handle.cancel();
            }
            live.sort_by(|a, b| b.bytes.cmp(&a.bytes));
            self.scan = ScanState::Done(ScanSummary {
                results: live,
                elapsed_ms: done.unwrap_or_else(|| started.elapsed().as_millis() as u64),
            });
            if canceled {
                self.toast("扫描已取消，显示的是已完成部分的结果", ToastKind::Warn);
            } else {
                let bytes = match &self.scan {
                    ScanState::Done(s) => s.total_bytes(),
                    _ => 0,
                };
                self.toast(
                    format!(
                        "扫描完成，共发现 {} 可清理",
                        crate::model::human_bytes(bytes)
                    ),
                    ToastKind::Success,
                );
            }
        } else {
            self.scan = ScanState::Running {
                handle,
                started,
                current,
                finished,
                total,
                live,
            };
        }
    }

    // -----------------------------------------------------------------
    // 清理
    // -----------------------------------------------------------------

    /// 弹出确认对话框。
    pub fn request_clean(&mut self) {
        if self.cleaning() {
            return;
        }
        let (targets, files, bytes) = self.pending_selection();
        if targets.is_empty() {
            let (title, body) = if self.summary().is_none() {
                (
                    "还没有扫描结果",
                    "请先点击「开始扫描」——扫描只会读取文件信息，不会删除任何东西。扫描后你可以逐项确认再清理。",
                )
            } else {
                (
                    "还没有勾选任何项目",
                    "在下面的列表里勾选想清理的类别，或点「全选有内容的」一键选中所有扫描出内容的项目。",
                )
            };
            self.dialog = Some(Dialog::Message {
                title: title.to_string(),
                body: body.to_string(),
                kind: ToastKind::Warn,
            });
            return;
        }
        self.dialog = Some(Dialog::ConfirmClean(Box::new(CleanConfirm {
            targets,
            opts: CleanOptions {
                mode: self.settings.clean_mode,
                min_age_days: self.settings.min_age_days,
                dry_run: self.settings.dry_run,
            },
            files,
            bytes,
        })));
    }

    /// 专项清理页的确认弹窗：只提交该页范围内的目标。
    pub fn request_clean_scoped(&mut self, groups: &[Group]) {
        if self.cleaning() {
            return;
        }
        let (targets, files, bytes) = self.pending_selection_in(groups);
        if targets.is_empty() {
            let (title, body) = if self.summary().is_none() {
                (
                    "还没有扫描结果",
                    "请先点击「开始扫描」——扫描只会读取文件信息，不会删除任何东西。",
                )
            } else {
                ("还没有勾选任何项目", "在下面的列表里勾选想清理的类别。")
            };
            self.dialog = Some(Dialog::Message {
                title: title.to_string(),
                body: body.to_string(),
                kind: ToastKind::Warn,
            });
            return;
        }
        self.dialog = Some(Dialog::ConfirmClean(Box::new(CleanConfirm {
            targets,
            opts: CleanOptions {
                mode: self.settings.clean_mode,
                min_age_days: self.settings.min_age_days,
                dry_run: self.settings.dry_run,
            },
            files,
            bytes,
        })));
    }

    pub fn start_clean(&mut self, targets: Vec<CleanTarget>, opts: CleanOptions) {
        if self.cleaning() {
            return;
        }
        let total = targets.len();
        let handle = cleaner::spawn(targets, opts);
        self.clean = CleanState::Running {
            handle,
            started: Instant::now(),
            current: String::new(),
            files: 0,
            bytes: 0,
            finished: 0,
            total,
        };
    }

    pub fn cancel_clean(&mut self) {
        if let CleanState::Running { handle, .. } = &self.clean {
            handle.cancel();
        }
    }

    fn poll_clean(&mut self) {
        let state = std::mem::replace(&mut self.clean, CleanState::Idle);
        let CleanState::Running {
            handle,
            started,
            mut current,
            mut files,
            mut bytes,
            mut finished,
            mut total,
        } = state
        else {
            self.clean = state;
            return;
        };

        let mut report: Option<CleanReport> = None;

        loop {
            match handle.rx.try_recv() {
                Ok(CleanMsg::Started { total: t }) => total = t,
                Ok(CleanMsg::TargetStarted { name }) => current = name,
                Ok(CleanMsg::TargetProgress { files: f, bytes: b }) => {
                    files = f;
                    bytes = b;
                }
                Ok(CleanMsg::TargetFinished) => finished += 1,
                Ok(CleanMsg::Finished(r)) | Ok(CleanMsg::Canceled(r)) => {
                    report = Some(r);
                    break;
                }
                Err(_) => break,
            }
        }

        match report {
            Some(report) => {
                let freed = report.freed_bytes;
                let blocked = report.blocked;
                if report.canceled {
                    self.toast("清理已中断，已完成的部分已生效", ToastKind::Warn);
                } else if blocked > 0 {
                    self.toast(
                        format!(
                            "清理完成，释放 {}；有 {blocked} 项被安全层拦截（已记录到日志）",
                            crate::model::human_bytes(freed)
                        ),
                        ToastKind::Warn,
                    );
                } else {
                    self.toast(
                        if report.dry_run {
                            format!("演练完成：预计可释放 {}", crate::model::human_bytes(freed))
                        } else {
                            format!("清理完成，释放 {}", crate::model::human_bytes(freed))
                        },
                        ToastKind::Success,
                    );
                }
                self.refresh_disk(true);
                self.last_report = Some(report.clone());
                self.dialog = Some(Dialog::Report(Box::new(report.clone())));
                // 清理后扫描结果已过期，需要重新扫描
                self.scan = ScanState::Idle;
                self.clean = CleanState::Done(report);
            }
            None => {
                self.clean = CleanState::Running {
                    handle,
                    started,
                    current,
                    files,
                    bytes,
                    finished,
                    total,
                };
            }
        }
    }

    // -----------------------------------------------------------------
    // 大文件
    // -----------------------------------------------------------------

    pub fn start_big_scan(&mut self) {
        if self.big_scanning() {
            return;
        }
        if self.big_roots.is_empty() {
            self.toast("请先添加至少一个扫描目录", ToastKind::Warn);
            return;
        }
        if !self.big_want_files && !self.big_want_folders {
            self.toast("请至少勾选「文件」或「文件夹」中的一项", ToastKind::Warn);
            return;
        }
        let opts = bigfiles::BigFileOptions {
            min_size: self.big_min_mb.max(1) * 1024 * 1024,
            roots: self.big_roots.clone(),
            want_files: self.big_want_files,
            want_folders: self.big_want_folders,
        };
        let handle = bigfiles::spawn(opts);
        self.big = BigState::Running {
            handle,
            started: Instant::now(),
            scanned: 0,
            current: String::new(),
        };
    }

    pub fn cancel_big_scan(&mut self) {
        if let BigState::Running { handle, .. } = &self.big {
            handle.cancel();
        }
    }

    fn poll_big(&mut self) {
        let state = std::mem::replace(&mut self.big, BigState::Idle);
        let BigState::Running {
            handle,
            started,
            mut scanned,
            mut current,
        } = state
        else {
            self.big = state;
            return;
        };

        let mut outcome = None;
        loop {
            match handle.rx.try_recv() {
                Ok(BigFileMsg::Started) => {}
                Ok(BigFileMsg::Progress {
                    scanned: s,
                    current: c,
                }) => {
                    scanned = s;
                    current = c;
                }
                Ok(BigFileMsg::Finished {
                    files,
                    folders,
                    totals,
                    scanned: s,
                    elapsed_ms,
                    canceled,
                }) => {
                    outcome = Some((files, folders, totals, s, elapsed_ms, canceled));
                    break;
                }
                Err(_) => break,
            }
        }

        match outcome {
            Some((files, folders, totals, scanned, elapsed_ms, canceled)) => {
                let file_count = files.len();
                let folder_count = folders.len();
                self.big = BigState::Done {
                    files,
                    folders,
                    totals,
                    scanned,
                    elapsed_ms,
                    canceled,
                    want_files: self.big_want_files,
                    want_folders: self.big_want_folders,
                };
                if canceled {
                    self.toast("大文件扫描已取消", ToastKind::Warn);
                } else {
                    self.toast(
                        format!("扫描完成：{file_count} 个大文件、{folder_count} 个大文件夹"),
                        ToastKind::Success,
                    );
                }
            }
            None => {
                self.big = BigState::Running {
                    handle,
                    started,
                    scanned,
                    current,
                };
            }
        }
    }

    /// 删除单个大文件（移入回收站）。
    pub fn recycle_big_file(&mut self, path: PathBuf) {
        match bigfiles::recycle_one(&path, &self.big_roots) {
            Ok(()) => {
                if let BigState::Done { files, .. } = &mut self.big {
                    files.retain(|f| f.path != path);
                }
                self.toast("已移入回收站", ToastKind::Success);
                self.refresh_disk(true);
            }
            Err(msg) => self.toast(format!("未删除：{msg}"), ToastKind::Error),
        }
    }

    /// 弹出「删除整个目录」的确认框。
    pub fn request_recycle_folder(&mut self, path: &Path) {
        let BigState::Done { folders, .. } = &self.big else {
            return;
        };
        let Some(folder) = folders.iter().find(|f| f.path == path).cloned() else {
            return;
        };

        if folder.protected {
            self.dialog = Some(Dialog::Message {
                title: "无法删除".to_string(),
                body: format!(
                    "「{}」位于本工具的保护范围内，不会提供删除操作。\n\n\
                     如果确实需要清理，请手动在系统自带的文件管理器里处理。",
                    folder.path.display()
                ),
                kind: ToastKind::Warn,
            });
            return;
        }

        self.dialog = Some(Dialog::ConfirmRecycleFolder(Box::new(folder)));
    }

    /// 真正把一个目录移入回收站。
    pub fn recycle_big_folder(&mut self, path: PathBuf) {
        match bigfiles::recycle_folder(&path, &self.big_roots) {
            Ok(()) => {
                if let BigState::Done { folders, .. } = &mut self.big {
                    folders.retain(|f| f.path != path);
                }
                self.toast("整个目录已移入回收站", ToastKind::Success);
                self.toast(
                    "上层目录的体积已经变了，需要准确数字请重新扫描",
                    ToastKind::Info,
                );
                self.refresh_disk(true);
            }
            Err(msg) => self.toast(format!("未删除：{msg}"), ToastKind::Error),
        }
    }

    // -----------------------------------------------------------------
    // 重复文件
    // -----------------------------------------------------------------

    /// 开始重复文件扫描：根目录复用「大文件」页的扫描范围（用户自己的目录）。
    pub fn start_dup_scan(&mut self) {
        if self.dup_scanning() || self.big_scanning() {
            return;
        }
        if self.big_roots.is_empty() {
            self.toast("请先在「设置」里添加至少一个扫描目录", ToastKind::Warn);
            return;
        }
        let handle = duplicates::spawn(self.big_roots.clone());
        self.dup = DupState::Running {
            handle,
            started: Instant::now(),
            scanned: 0,
            current: String::new(),
        };
    }

    pub fn cancel_dup_scan(&mut self) {
        if let DupState::Running { handle, .. } = &self.dup {
            handle.cancel();
        }
    }

    fn poll_dup(&mut self) {
        let state = std::mem::replace(&mut self.dup, DupState::Idle);
        let DupState::Running {
            handle,
            started,
            mut scanned,
            mut current,
        } = state
        else {
            self.dup = state;
            return;
        };

        let mut outcome = None;
        loop {
            match handle.rx.try_recv() {
                Ok(DupMsg::Started) => {}
                Ok(DupMsg::Progress {
                    scanned: s,
                    current: c,
                }) => {
                    scanned = s;
                    current = c;
                }
                Ok(DupMsg::Finished {
                    groups,
                    scanned: s,
                    elapsed_ms,
                    canceled,
                }) => {
                    outcome = Some((groups, s, elapsed_ms, canceled));
                    break;
                }
                Err(_) => break,
            }
        }

        match outcome {
            Some((groups, scanned, elapsed_ms, canceled)) => {
                let group_count = groups.len();
                let reclaimable: u64 = groups.iter().map(|g| g.reclaimable()).sum();
                // 默认勾选：每组保留最早修改的一份，其余副本全部选中
                self.dup_selected = groups
                    .iter()
                    .flat_map(|g| {
                        let keep = g.keep_index();
                        g.files
                            .iter()
                            .enumerate()
                            .filter(|(i, _)| *i != keep)
                            .map(|(_, f)| f.path.clone())
                            .collect::<Vec<_>>()
                    })
                    .collect();
                self.dup = DupState::Done {
                    groups,
                    scanned,
                    elapsed_ms,
                    canceled,
                };
                if canceled {
                    self.toast("重复文件扫描已取消", ToastKind::Warn);
                } else if group_count == 0 {
                    self.toast("扫描完成，没有发现重复文件", ToastKind::Success);
                } else {
                    self.toast(
                        format!(
                            "发现 {group_count} 组重复文件，最多可释放 {}",
                            crate::model::human_bytes(reclaimable)
                        ),
                        ToastKind::Success,
                    );
                }
            }
            None => {
                self.dup = DupState::Running {
                    handle,
                    started,
                    scanned,
                    current,
                };
            }
        }
    }

    /// 勾选 / 取消一个重复副本。
    pub fn toggle_dup(&mut self, path: &Path) {
        if self.dup_selected.contains(path) {
            self.dup_selected.remove(path);
        } else {
            self.dup_selected.insert(path.to_path_buf());
        }
    }

    /// 把整个重复文件组勾 / 取消（除保留那份之外的所有副本）。
    pub fn toggle_dup_group(&mut self, group: &DupGroup, checked: bool) {
        let keep = group.keep_index();
        for (i, f) in group.files.iter().enumerate() {
            if i == keep {
                continue;
            }
            if checked {
                self.dup_selected.insert(f.path.clone());
            } else {
                self.dup_selected.remove(&f.path);
            }
        }
    }

    /// 弹出批量移入回收站的确认框。
    pub fn request_recycle_duplicates(&mut self) {
        if self.dup_selected.is_empty() {
            self.toast("请先勾选要清理的重复副本", ToastKind::Warn);
            return;
        }
        let bytes: u64 = self
            .dup_selected
            .iter()
            .filter_map(|p| std::fs::metadata(p).ok())
            .map(|m| m.len())
            .sum();
        let paths: Vec<PathBuf> = self.dup_selected.iter().cloned().collect();
        self.dialog = Some(Dialog::ConfirmDuplicates(paths, bytes));
    }

    /// 真正把勾选的重复副本移入回收站。
    pub fn recycle_duplicates_confirmed(&mut self, paths: Vec<PathBuf>) {
        let mut freed = 0u64;
        let mut done = 0usize;
        let mut blocked = 0usize;
        for path in &paths {
            let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
            match bigfiles::recycle_one(path, &self.big_roots) {
                Ok(()) => {
                    freed += size;
                    done += 1;
                    self.dup_selected.remove(path);
                }
                Err(_) => blocked += 1,
            }
        }
        // 从结果里移除已经清掉的副本；组里剩不到两份就整个删掉
        if let DupState::Done { groups, .. } = &mut self.dup {
            groups.retain_mut(|g| {
                g.files.retain(|f| !paths.contains(&f.path));
                g.files.len() >= 2
            });
        }
        if blocked > 0 {
            self.toast(
                format!(
                    "已移入回收站 {done} 个文件、释放 {}；{blocked} 个被安全层拦截",
                    crate::model::human_bytes(freed)
                ),
                ToastKind::Warn,
            );
        } else {
            self.toast(
                format!(
                    "已移入回收站 {done} 个重复副本，释放 {}",
                    crate::model::human_bytes(freed)
                ),
                ToastKind::Success,
            );
        }
        self.refresh_disk(true);
    }

    // -----------------------------------------------------------------
    // 已安装软件
    // -----------------------------------------------------------------

    pub fn apps_scanning(&self) -> bool {
        matches!(self.apps_scan, AppScanState::Running { .. })
    }

    pub fn start_apps_scan(&mut self) {
        if self.apps_scanning() {
            return;
        }
        let handle = apps::spawn();
        self.apps_scan = AppScanState::Running {
            handle,
            started: Instant::now(),
            phase: "正在读取已安装软件清单…".to_string(),
            measured: 0,
            total: 0,
        };
    }

    pub fn cancel_apps_scan(&mut self) {
        if let AppScanState::Running { handle, .. } = &self.apps_scan {
            handle.cancel();
        }
    }

    /// 进入页面时如果还没扫过，就自动扫一次。
    pub fn ensure_apps_scanned(&mut self) {
        if matches!(self.apps_scan, AppScanState::Idle) {
            self.start_apps_scan();
        }
    }

    fn poll_apps(&mut self) {
        let state = std::mem::replace(&mut self.apps_scan, AppScanState::Idle);
        let AppScanState::Running {
            handle,
            started,
            mut phase,
            mut measured,
            mut total,
        } = state
        else {
            self.apps_scan = state;
            return;
        };

        let mut outcome: Option<(u64, bool)> = None;

        loop {
            match handle.rx.try_recv() {
                Ok(AppMsg::Found { apps }) => {
                    total = apps.len();
                    phase = "正在统计占用空间…".to_string();
                    self.apps = apps;
                }
                Ok(AppMsg::Size { index, bytes }) => {
                    if let Some(app) = self.apps.get_mut(index) {
                        app.measured_bytes = Some(bytes);
                    }
                    measured += 1;
                }
                Ok(AppMsg::Finished {
                    elapsed_ms,
                    canceled,
                }) => {
                    outcome = Some((elapsed_ms, canceled));
                    break;
                }
                Err(_) => break,
            }
        }

        match outcome {
            Some((elapsed_ms, canceled)) => {
                self.apps_scan = AppScanState::Done {
                    elapsed_ms,
                    canceled,
                };
                if canceled {
                    self.toast("已停止统计，当前显示的是已完成的部分", ToastKind::Warn);
                } else {
                    let (count, bytes) = self.apps_summary();
                    self.toast(
                        format!(
                            "找到 {count} 款软件，合计占用 {}",
                            crate::model::human_bytes(bytes)
                        ),
                        ToastKind::Success,
                    );
                }
            }
            None => {
                self.apps_scan = AppScanState::Running {
                    handle,
                    started,
                    phase,
                    measured,
                    total,
                };
            }
        }
    }

    /// 当前筛选条件下的 (软件数, 合计体积)。
    pub fn apps_summary(&self) -> (u64, u64) {
        let view = self.apps_view();
        let bytes = view
            .iter()
            .filter_map(|i| self.apps.get(*i))
            .filter_map(|a| a.size_bytes())
            .sum::<u64>();
        (view.len() as u64, bytes)
    }

    /// 按搜索词与排序规则，算出要显示的条目下标。
    pub fn apps_view(&self) -> Vec<usize> {
        let query = self.apps_query.trim().to_lowercase();

        let mut indices: Vec<usize> = self
            .apps
            .iter()
            .enumerate()
            .filter(|(_, app)| {
                if self.apps_only_uninstallable && !app.can_uninstall() {
                    return false;
                }
                if query.is_empty() {
                    return true;
                }
                app.name.to_lowercase().contains(&query)
                    || app.publisher.to_lowercase().contains(&query)
            })
            .map(|(index, _)| index)
            .collect();

        match self.apps_sort {
            AppsSort::SizeDesc => {
                indices.sort_by(|a, b| {
                    let sa = self.apps[*a].size_bytes().unwrap_or(0);
                    let sb = self.apps[*b].size_bytes().unwrap_or(0);
                    sb.cmp(&sa).then_with(|| {
                        self.apps[*a]
                            .name
                            .to_lowercase()
                            .cmp(&self.apps[*b].name.to_lowercase())
                    })
                });
            }
            AppsSort::NameAsc => {
                indices.sort_by(|a, b| {
                    self.apps[*a]
                        .name
                        .to_lowercase()
                        .cmp(&self.apps[*b].name.to_lowercase())
                });
            }
            AppsSort::DateDesc => {
                indices.sort_by(|a, b| self.apps[*b].install_date.cmp(&self.apps[*a].install_date));
            }
        }

        indices
    }

    /// 弹出卸载确认框。
    pub fn request_uninstall(&mut self, index: usize) {
        let Some(app) = self.apps.get(index).cloned() else {
            return;
        };
        if !app.can_uninstall() {
            self.dialog = Some(Dialog::Message {
                title: "无法卸载".to_string(),
                body: format!(
                    "「{}」没有提供可用的卸载方式{}。\n\n\
                     这类软件通常需要用它自己的安装包来卸载。",
                    app.name,
                    if app.no_remove {
                        "（来源明确标记了不允许卸载）"
                    } else {
                        ""
                    }
                ),
                kind: ToastKind::Warn,
            });
            return;
        }
        self.dialog = Some(Dialog::ConfirmUninstall(Box::new(app)));
    }

    /// 真正执行卸载动作。
    pub fn run_uninstall(&mut self, app: InstalledApp) {
        if !app.can_uninstall() {
            self.toast("该软件没有可用的卸载方式", ToastKind::Error);
            return;
        }
        let elevate = app.needs_elevation();

        match apps::run_uninstall(&app) {
            Ok(()) => {
                let hint = if elevate {
                    crate::sysutil::elevation_hint()
                } else {
                    "请在弹出的窗口里完成后续步骤"
                };
                self.toast(
                    format!("已启动「{}」的卸载流程，{hint}", app.name),
                    ToastKind::Info,
                );
                self.toast("卸载完成后，点「重新扫描」刷新列表", ToastKind::Info);
            }
            Err(msg) => self.toast(format!("卸载「{}」失败：{msg}", app.name), ToastKind::Error),
        }
    }

    // -----------------------------------------------------------------
    // 其他
    // -----------------------------------------------------------------

    pub fn refresh_disk(&mut self, force: bool) {
        if !force && self.disk_at.elapsed() < Duration::from_secs(5) {
            return;
        }
        self.disk = sysutil::disk_info(&sysutil::default_mount());
        self.disk_at = Instant::now();
    }

    fn poll_toasts(&mut self) {
        self.toasts
            .retain(|t| t.at.elapsed() < Duration::from_secs(5));
    }

    fn maybe_save_settings(&mut self) {
        if !self.settings_dirty {
            return;
        }
        if self.settings_saved_at.elapsed() < Duration::from_millis(700) {
            return;
        }
        self.settings.selected_targets = Some(self.selected.iter().cloned().collect::<Vec<_>>());
        self.settings.big_file_min_mb = self.big_min_mb;
        self.settings.big_want_files = self.big_want_files;
        self.settings.big_want_folders = self.big_want_folders;
        self.settings.big_file_roots = self
            .big_roots
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect();
        if let Err(e) = self.settings.save() {
            self.toast(format!("设置保存失败：{e}"), ToastKind::Error);
        }
        self.settings_dirty = false;
        self.settings_saved_at = Instant::now();
    }

    pub fn apply_theme(&mut self, pref: ThemePref, ctx: &egui::Context) {
        self.settings.theme = pref;
        theme::install(ctx, pref);
        self.touch_settings();
    }

    fn apply_selection_for_group(&mut self, group: Group, checked: bool) {
        let ids: Vec<String> = self
            .targets
            .iter()
            .filter(|t| t.group == group)
            .map(|t| t.id.to_string())
            .collect();
        for id in ids {
            if checked {
                self.selected.insert(id);
            } else {
                self.selected.remove(&id);
            }
        }
        self.touch_settings();
    }

    pub fn set_group_selection(&mut self, group: Group, checked: bool) {
        self.apply_selection_for_group(group, checked);
    }

    pub fn toggle_target(&mut self, id: &str) {
        let key = id.to_string();
        if self.selected.contains(&key) {
            self.selected.remove(&key);
        } else {
            self.selected.insert(key);
        }
        self.touch_settings();
    }

    /// 按当前扫描结果选择"所有有内容的项"。
    pub fn select_all_with_content(&mut self) {
        let ids: Vec<String> = self
            .targets
            .iter()
            .filter(|t| self.result_for(t.id).map(|r| r.files > 0).unwrap_or(false))
            .map(|t| t.id.to_string())
            .collect();
        self.selected = ids.into_iter().collect();
        self.touch_settings();
    }

    /// 卡片行右上角那颗勾选框用的一步到位勾选：把四张卡覆盖的项全部勾上 / 全不勾。
    ///
    /// 参考图上没有"全选"按钮 —— 选择就是四张卡上的四个勾选框。Shift 点任一卡片
    /// 等同于"全选有内容的"，这样常用的那一步不用点四次。
    pub fn select_card_row(&mut self, checked: bool) {
        if checked {
            self.select_all_with_content();
        } else {
            self.selected.clear();
            self.touch_settings();
        }
    }
}

// ---------------------------------------------------------------------------
// eframe::App
// ---------------------------------------------------------------------------

/// 中央内容区右侧给悬浮滚动条留的过道宽度（见 `App::ui` 里的说明）。
///
/// 取 12px：滚动条休眠时 2px、悬停时展开到 10px，留 12 保证两种状态都在空白里。
const SCROLL_GUTTER: f32 = 12.0;

impl eframe::App for App {
    /// 窗口清屏色，避免缩放时露出底色。
    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        let p = if visuals.dark_mode {
            theme::Palette::DARK
        } else {
            theme::Palette::LIGHT
        };
        p.bg.to_normalized_gamma_f32()
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        self.poll_scan();
        self.poll_clean();
        self.poll_big();
        self.poll_apps();
        self.poll_dup();
        self.poll_toasts();
        self.refresh_disk(false);
        self.maybe_save_settings();

        if self.page == Page::Apps {
            self.ensure_apps_scanned();
        }

        if self.busy() || self.apps_scanning() {
            ctx.request_repaint_after(Duration::from_millis(50));
        } else if !self.toasts.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(120));
        }

        // 面板顺序 = 布局顺序。标题栏在最上面，占满整个窗口宽度；侧栏紧跟其后
        // 声明，于是占满剩下的整个高度（连顶栏和状态栏的左边也不让出），
        // 这样才能做出参考图里那种「侧栏通高」的样子。
        ui::chrome::title_bar(ui);
        ui::chrome::side_nav(self, ui);
        ui::chrome::status_bar(self, ui);
        ui::chrome::top_bar(self, ui);

        let bg = theme::Palette::of(ui).bg;
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(bg).inner_margin(egui::Margin {
                left: 22,
                // 内容自身仍然停在「离窗口右边缘 22px」处，只是把这 22px 拆成
                // 「10px 内边距 + 12px 滚动条过道」，见下面的 SCROLL_GUTTER。
                right: 10,
                top: 16,
                bottom: 16,
            }))
            .show(ui, |ui| {
                // 滚动条过道：**自己**留，不用 egui 的 `floating_allocated_width`。
                //
                // 起因：egui 默认的 `ScrollStyle::floating()` 让滚动条不占布局宽度，
                // 那条 2px（悬停展开到 10px）的灰条直接浮在内容最右边的像素上，
                // 一滚动就压在按钮圆角上。而 `floating_allocated_width` 虽然能留过道，
                // 它的生效条件是「当前页**需要**滚动」—— 页面矮一点、不用滚动时过道
                // 就没了，于是同一个窗口里，能滚的页内容右边界在 1098、不能滚的页在
                // 1110，翻页时右边距会跳 12px（智能清理页改短之后就踩到了）。
                //
                // 所以这里把过道做成**无条件**的：先 `set_max_width` 把内容收窄 12px，
                // 滚动条就永远落在那 12px 空白里；内容右边界恒为 1120 − 10 − 12 = 1098，
                // 和滚动与否无关。
                let w = (ui.available_width() - SCROLL_GUTTER).max(160.0);
                ui.set_max_width(w);
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| match self.page {
                        Page::Overview => ui::overview::show(self, ui),
                        Page::Clean => ui::clean_page::show(self, ui),
                        Page::Apps => ui::apps_page::show(self, ui),
                        Page::BigFiles => ui::bigfiles_page::show(self, ui),
                        Page::BigDetail => ui::bigfiles_page::show_all(self, ui),
                        Page::Duplicates => ui::duplicates_page::show(self, ui),
                        // 系统设置 / 微信 / QQ：同一套专项清理页，只是扫描范围不同
                        Page::System | Page::Wechat | Page::Qq => ui::scope_page::show(self, ui),
                        Page::Settings => ui::settings_page::show(self, ui),
                        Page::Safety => ui::safety_page::show(self, ui),
                    });
            });

        ui::dialogs::show(self, &ctx);
        ui::chrome::toasts(self, &ctx);
        // 无边框窗口靠这些手柄恢复"拖边改大小"
        ui::chrome::resize_handles(&ctx);
    }
}
