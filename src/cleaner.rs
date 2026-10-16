//! 清理层：真正执行删除。
//!
//! 安全约定：
//! * 每个待删除条目都会 **再次** 通过 [`crate::safety::verify`]，不信任扫描阶段的结果；
//! * 目录是"先清空内容，再在确认为空时删除"，永远不会递归删除一个非空目录；
//! * 命中的浏览器缓存目录只清内容，目录本身保留（避免应用找不到缓存路径）；
//! * 被占用 / 无权限的条目只会被跳过并计数，不会中断整个流程；
//! * 默认把文件移入回收站；永久删除需要在界面上显式切换并二次确认；
//! * 支持"演练模式"，只统计不删除，方便先看清楚会发生什么。

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Instant, SystemTime};

use serde::{Deserialize, Serialize};

use crate::model::{
    CleanReport, CleanTarget, CleanTargetReport, NameRule, Root, RootMode, human_bytes,
};
use crate::recycle;
use crate::safety;
use crate::sysutil;

// ---------------------------------------------------------------------------
// 参数
// ---------------------------------------------------------------------------

/// 删除方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CleanMode {
    /// 移入回收站（可恢复，更安全，但大量文件时较慢）
    RecycleBin,
    /// 永久删除（更快，不可恢复）
    Permanent,
}

impl CleanMode {
    pub fn label(self) -> &'static str {
        match self {
            CleanMode::RecycleBin => recycle::action_label(),
            CleanMode::Permanent => "永久删除",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            CleanMode::RecycleBin => "文件仍可在回收站里找回，最安全",
            CleanMode::Permanent => "直接释放磁盘空间，删除后无法恢复",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct CleanOptions {
    pub mode: CleanMode,
    pub min_age_days: u32,
    /// 演练模式：只统计体积，不真正删除任何文件
    pub dry_run: bool,
}

impl Default for CleanOptions {
    fn default() -> Self {
        Self {
            mode: CleanMode::RecycleBin,
            min_age_days: 7,
            dry_run: false,
        }
    }
}

// ---------------------------------------------------------------------------
// 消息与句柄
// ---------------------------------------------------------------------------

pub enum CleanMsg {
    Started { total: usize },
    TargetStarted { name: String },
    TargetProgress { files: u64, bytes: u64 },
    TargetFinished,
    Finished(CleanReport),
    Canceled(CleanReport),
}

pub struct CleanHandle {
    pub rx: Receiver<CleanMsg>,
    pub cancel: Arc<AtomicBool>,
}

impl CleanHandle {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// 启动后台清理任务。
pub fn spawn(targets: Vec<CleanTarget>, opts: CleanOptions) -> CleanHandle {
    let (tx, rx) = channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_worker = Arc::clone(&cancel);

    std::thread::spawn(move || {
        let started = Instant::now();
        let total = targets.len();
        let _ = tx.send(CleanMsg::Started { total });

        let mut report = CleanReport {
            dry_run: opts.dry_run,
            ..Default::default()
        };
        let mut log_lines: Vec<String> = Vec::new();

        for target in &targets {
            if cancel_worker.load(Ordering::Relaxed) {
                break;
            }
            let _ = tx.send(CleanMsg::TargetStarted {
                name: target.name.to_string(),
            });

            let entry = clean_target(target, &opts, &cancel_worker, &tx, &mut log_lines);
            report.merge(entry.clone());

            // 防止日志缓冲区无限增长
            if log_lines.len() > MAX_LOG_LINES {
                log_lines.truncate(MAX_LOG_LINES);
            }

            let _ = tx.send(CleanMsg::TargetFinished);
        }

        report.elapsed_ms = started.elapsed().as_millis() as u64;
        report.canceled = cancel_worker.load(Ordering::Relaxed);
        report.log_path = write_log(&report, &log_lines, opts).ok();

        if report.canceled {
            let _ = tx.send(CleanMsg::Canceled(report));
        } else {
            let _ = tx.send(CleanMsg::Finished(report));
        }
    });

    CleanHandle { rx, cancel }
}

// ---------------------------------------------------------------------------
// 单个目标
// ---------------------------------------------------------------------------

fn clean_target(
    target: &CleanTarget,
    opts: &CleanOptions,
    cancel: &AtomicBool,
    tx: &Sender<CleanMsg>,
    log_lines: &mut Vec<String>,
) -> CleanTargetReport {
    let mut report = CleanTargetReport {
        id: target.id,
        name: target.name.to_string(),
        ..Default::default()
    };

    let age_days = if target.age_filtered {
        opts.min_age_days
    } else {
        0
    };

    let mut progress = ProgressThrottle::new(tx);
    log_lines.push(format!("[目标] {} ({})", target.name, target.id));

    for root in &target.roots {
        if cancel.load(Ordering::Relaxed) {
            break;
        }

        if matches!(root.mode, RootMode::RecycleBin) {
            clean_recycle_bin(opts, &mut report, log_lines);
            continue;
        }

        if !root.path.exists() {
            continue;
        }

        match &root.mode {
            RootMode::RecycleBin => {}

            // 单文件：以父目录作为包含性校验的根
            RootMode::ExactFile => {
                let Some(parent) = root.path.parent() else {
                    report.blocked += 1;
                    continue;
                };
                if safety::verify_root(parent).is_err() {
                    report.blocked += 1;
                    continue;
                }
                let mut queue = DeleteQueue::new(parent);
                queue_file(
                    &root.path,
                    parent,
                    age_days,
                    opts,
                    &mut queue,
                    &mut report,
                    &mut progress,
                    log_lines,
                );
                queue.finish(opts, &mut report, &mut progress, log_lines);
            }

            RootMode::ShallowFiles => {
                if safety::verify_root(&root.path).is_err() {
                    report.blocked += 1;
                    continue;
                }
                let mut queue = DeleteQueue::new(&root.path);
                clean_shallow(
                    root,
                    age_days,
                    opts,
                    cancel,
                    &mut queue,
                    &mut report,
                    &mut progress,
                    log_lines,
                );
                queue.finish(opts, &mut report, &mut progress, log_lines);
            }

            RootMode::All | RootMode::Matched(_) => {
                if safety::verify_root(&root.path).is_err() {
                    report.blocked += 1;
                    log_lines.push(format!(
                        "  [拦截] 清理根目录未通过安全检查: {}",
                        root.path.display()
                    ));
                    continue;
                }
                let pass = match &root.mode {
                    RootMode::All => Pass::Everything,
                    RootMode::Matched(rules) => Pass::MatchOnly(rules.as_slice()),
                    _ => continue,
                };
                let mut queue = DeleteQueue::new(&root.path);
                clean_dir(
                    &root.path,
                    &root.path,
                    pass,
                    &root.skip_dirs,
                    1,
                    age_days,
                    opts,
                    cancel,
                    &mut queue,
                    &mut report,
                    &mut progress,
                    log_lines,
                );
                queue.finish(opts, &mut report, &mut progress, log_lines);
            }
        }
    }

    log_lines.push(format!(
        "  └─ 小结: {} 个文件 / 目录 {} / {} / 跳过 {} / 拦截 {}",
        report.files,
        report.dirs,
        human_bytes(report.bytes),
        report.skipped,
        report.blocked
    ));

    report
}

// ---------------------------------------------------------------------------
// 遍历的"放行策略"
// ---------------------------------------------------------------------------

/// 决定当前目录层级的删除范围。
#[derive(Clone, Copy)]
enum Pass<'a> {
    /// 全部删除（`RootMode::All`，或已命中缓存目录的子树）
    Everything,
    /// 只删除名字命中规则的条目（`RootMode::Matched` 的目录扫描阶段）
    MatchOnly(&'a [NameRule]),
}

impl Pass<'_> {
    fn matches(&self, name_lower: &str) -> bool {
        match self {
            Pass::Everything => true,
            Pass::MatchOnly(rules) => rules.iter().any(|r| r.matches(name_lower)),
        }
    }

    fn is_everything(&self) -> bool {
        matches!(self, Pass::Everything)
    }
}

// ---------------------------------------------------------------------------
// 目录遍历 + 删除
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn clean_dir(
    walk_root: &Path,
    current: &Path,
    pass: Pass<'_>,
    skip_dirs: &[String],
    depth: usize,
    age_days: u32,
    opts: &CleanOptions,
    cancel: &AtomicBool,
    queue: &mut DeleteQueue,
    report: &mut CleanTargetReport,
    progress: &mut ProgressThrottle,
    log_lines: &mut Vec<String>,
) {
    if cancel.load(Ordering::Relaxed) {
        return;
    }

    // 先把目录项读进内存，避免"边遍历边删除"导致漏项
    let mut children: Vec<(PathBuf, bool, bool)> = Vec::new();
    match std::fs::read_dir(current) {
        Ok(reader) => {
            for entry in reader.flatten() {
                let ftype = entry.file_type().ok();
                let is_symlink = ftype.map(|t| t.is_symlink()).unwrap_or(false);
                let is_dir = ftype.map(|t| t.is_dir()).unwrap_or(false);
                children.push((entry.path(), is_dir, is_symlink));
            }
        }
        Err(e) => {
            report.skipped += 1;
            push_detail(
                log_lines,
                &format!("  读取目录失败 {} ({e})", current.display()),
            );
            return;
        }
    }

    for (path, is_dir_hint, is_symlink) in children {
        if cancel.load(Ordering::Relaxed) {
            return;
        }

        // 符号链接 / 目录联接：不跟随、不删除
        if is_symlink {
            report.skipped += 1;
            continue;
        }

        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let name_lower = name.to_ascii_lowercase();

        if depth == 1 && is_dir_hint && skip_dirs.iter().any(|d| *d == name_lower) {
            report.skipped += 1;
            continue;
        }

        if is_dir_hint {
            let hit = pass.matches(&name_lower);
            // 命中缓存目录 → 之后整棵子树全部清理
            let child_pass = if pass.is_everything() || hit {
                Pass::Everything
            } else {
                pass
            };
            let preserve_self = !pass.is_everything() && hit;

            clean_dir(
                walk_root,
                &path,
                child_pass,
                skip_dirs,
                depth + 1,
                age_days,
                opts,
                cancel,
                queue,
                report,
                progress,
                log_lines,
            );

            // 空目录顺手删除；命中的缓存目录本身保留。
            // 这里只登记，等文件都删完再统一处理（见 DeleteQueue::finish）。
            if !opts.dry_run && !preserve_self {
                queue.push_dir(&path);
            }
            continue;
        }

        if !pass.matches(&name_lower) {
            continue;
        }
        queue_file(
            &path, walk_root, age_days, opts, queue, report, progress, log_lines,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn clean_shallow(
    root: &Root,
    age_days: u32,
    opts: &CleanOptions,
    cancel: &AtomicBool,
    queue: &mut DeleteQueue,
    report: &mut CleanTargetReport,
    progress: &mut ProgressThrottle,
    log_lines: &mut Vec<String>,
) {
    let Ok(reader) = std::fs::read_dir(&root.path) else {
        report.skipped += 1;
        return;
    };
    let mut files: Vec<PathBuf> = Vec::new();
    for entry in reader.flatten() {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        let ftype = entry.file_type().ok();
        if ftype.map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        if ftype.map(|t| t.is_symlink()).unwrap_or(false) {
            report.skipped += 1;
            continue;
        }
        let name = entry.file_name();
        if root.skips_dir(&name.to_string_lossy()) {
            continue;
        }
        files.push(entry.path());
    }

    for path in files {
        queue_file(
            &path, &root.path, age_days, opts, queue, report, progress, log_lines,
        );
    }
}

/// 删除队列。
///
/// 「移入回收站」这条通道底层是 `SHFileOperationW`：一次调用可以携带几百个路径。
/// 如果每个文件都单独调一次，清理上万个缓存文件会慢得离谱（每个文件都要走一遍
/// Shell 的目录解析与回收站改名）。所以这里先把通过全部安全检查的候选文件攒起来，
/// 攒够一批再整批提交。
///
/// 「永久删除」不需要排队，但沿用同一个队列只是为了让上层调用点保持统一。
struct DeleteQueue {
    /// 包含性校验的根（所有入队文件都必须位于它之下）
    root: PathBuf,
    pending: Vec<(PathBuf, u64)>,
    /// 清空后可能变空的目录，按「由深到浅」的顺序记录，最后统一处理
    dirs: Vec<PathBuf>,
}

/// 攒够这么多文件就提交一批。
const QUEUE_FLUSH: usize = 400;

impl DeleteQueue {
    fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
            pending: Vec::new(),
            dirs: Vec::new(),
        }
    }

    fn push(&mut self, path: &Path, size: u64) {
        self.pending.push((path.to_path_buf(), size));
    }

    fn is_full(&self) -> bool {
        self.pending.len() >= QUEUE_FLUSH
    }

    /// 记下一个「内容清空后应当被删掉」的目录。
    ///
    /// 必须延后处理：文件是分批提交的，遍历到某个目录时，它下面往往还有
    /// 排队等着删除的文件，此刻 `remove_dir` 一定失败。
    fn push_dir(&mut self, path: &Path) {
        self.dirs.push(path.to_path_buf());
    }

    /// 收尾：先把剩下的文件全部提交，再按由深到浅的顺序清理空目录。
    fn finish(
        &mut self,
        opts: &CleanOptions,
        report: &mut CleanTargetReport,
        progress: &mut ProgressThrottle,
        log_lines: &mut Vec<String>,
    ) {
        self.flush(opts.mode, report, progress, log_lines);

        let dirs = std::mem::take(&mut self.dirs);
        if opts.dry_run {
            return;
        }

        for dir in dirs {
            if let Err(reject) = safety::verify(&dir, &self.root, false) {
                report.blocked += 1;
                push_detail(
                    log_lines,
                    &format!("  [拦截] 目录 {} —— {}", dir.display(), reject.reason()),
                );
                continue;
            }
            // remove_dir 只对空目录生效；非空目录会自然失败，绝不递归删除。
            // 顺序是「由深到浅」，所以子目录先被移除，父目录随后才可能变空。
            if std::fs::remove_dir(&dir).is_ok() {
                report.dirs += 1;
                push_detail(log_lines, &format!("  - 空目录 {}", dir.display()));
            }
        }
    }

    /// 提交队列中的删除请求，并把逐条结果计入报告。
    fn flush(
        &mut self,
        mode: CleanMode,
        report: &mut CleanTargetReport,
        progress: &mut ProgressThrottle,
        log_lines: &mut Vec<String>,
    ) {
        if self.pending.is_empty() {
            return;
        }
        let items = std::mem::take(&mut self.pending);

        // 真正动手之前再走一遍安全检查：路径必须仍位于授权根之下、且不在保护名单里
        let mut verified: Vec<(PathBuf, u64)> = Vec::with_capacity(items.len());
        for (path, size) in items {
            match safety::verify(&path, &self.root, false) {
                Ok(()) => verified.push((path, size)),
                Err(reject) => {
                    report.blocked += 1;
                    push_detail(
                        log_lines,
                        &format!("  [拦截] {} —— {}", path.display(), reject.reason()),
                    );
                }
            }
        }

        match mode {
            // 永久删除：逐个删除，失败只跳过、不中断整个流程
            CleanMode::Permanent => {
                for (path, size) in verified {
                    match std::fs::remove_file(&path) {
                        Ok(()) => {
                            report.files += 1;
                            report.bytes += size;
                            push_detail(
                                log_lines,
                                &format!("  - {} ({})", path.display(), human_bytes(size)),
                            );
                            progress.tick(1, size);
                        }
                        Err(_) => {
                            report.skipped += 1;
                            push_detail(
                                log_lines,
                                &format!("  [跳过] {} —— 文件被占用或无权限", path.display()),
                            );
                        }
                    }
                }
            }

            // 移入回收站：按批提交，一次系统调用处理几百个文件
            CleanMode::RecycleBin => {
                for chunk in verified.chunks(QUEUE_FLUSH) {
                    let paths: Vec<PathBuf> = chunk.iter().map(|(p, _)| p.clone()).collect();
                    let outcome = recycle::recycle_batch(&paths);
                    for (offset, ok) in outcome.into_iter().enumerate() {
                        let (path, size) = &chunk[offset];
                        if ok {
                            report.files += 1;
                            report.bytes += *size;
                            push_detail(
                                log_lines,
                                &format!("  - {} ({})", path.display(), human_bytes(*size)),
                            );
                            progress.tick(1, *size);
                        } else {
                            report.skipped += 1;
                            push_detail(
                                log_lines,
                                &format!(
                                    "  [跳过] {} —— 文件被占用、无权限或回收站不可用",
                                    path.display()
                                ),
                            );
                        }
                    }
                }
            }
        }
    }
}

/// 校验单个文件并把它放进删除队列（含全部安全检查与年龄过滤）。
///
/// 注意：安全检查在**入队前**完成，队列提交时不再重复；两者之间只隔几百个文件，
/// 时间窗口极短。
#[allow(clippy::too_many_arguments)]
fn queue_file(
    path: &Path,
    root: &Path,
    age_days: u32,
    opts: &CleanOptions,
    queue: &mut DeleteQueue,
    report: &mut CleanTargetReport,
    progress: &mut ProgressThrottle,
    log_lines: &mut Vec<String>,
) {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        report.skipped += 1;
        return;
    };
    if meta.file_type().is_symlink() || !meta.is_file() {
        report.skipped += 1;
        return;
    }

    // 年龄过滤：太新的文件一律不动
    let modified = meta.modified().ok();
    if safety::too_new(modified, age_days) {
        report.skipped += 1;
        return;
    }

    let size = meta.len();

    // 删除前的最后一道保险
    if let Err(reject) = safety::verify(path, root, false) {
        report.blocked += 1;
        push_detail(
            log_lines,
            &format!("  [拦截] {} —— {}", path.display(), reject.reason()),
        );
        return;
    }

    // 演练模式：只统计，不入队
    if opts.dry_run {
        report.files += 1;
        report.bytes += size;
        progress.tick(1, size);
        return;
    }

    queue.push(path, size);
    if queue.is_full() {
        queue.flush(opts.mode, report, progress, log_lines);
    }
}

/// 清空回收站。
fn clean_recycle_bin(
    opts: &CleanOptions,
    report: &mut CleanTargetReport,
    log_lines: &mut Vec<String>,
) {
    let (bytes, items) = recycle_bin_size().unwrap_or((0, 0));
    if opts.dry_run {
        report.files += items;
        report.bytes += bytes;
        return;
    }
    match empty_recycle_bin() {
        Ok(()) => {
            report.files += items;
            report.bytes += bytes;
            push_detail(
                log_lines,
                &format!("  - 清空回收站: {} 个条目 / {}", items, human_bytes(bytes)),
            );
        }
        Err(msg) => {
            report.skipped += 1;
            push_detail(log_lines, &format!("  [失败] 清空回收站: {msg}"));
        }
    }
}

// ---------------------------------------------------------------------------
// 进度节流与日志缓冲
// ---------------------------------------------------------------------------

const MAX_LOG_LINES: usize = 4_000;
const MAX_LOG_DETAIL_LINES: usize = 1_500;

fn push_detail(log_lines: &mut Vec<String>, line: &str) {
    if log_lines.len() < MAX_LOG_DETAIL_LINES {
        log_lines.push(line.to_string());
    }
}

struct ProgressThrottle<'a> {
    tx: &'a Sender<CleanMsg>,
    files: u64,
    bytes: u64,
    last_emit: Instant,
}

impl<'a> ProgressThrottle<'a> {
    fn new(tx: &'a Sender<CleanMsg>) -> Self {
        Self {
            tx,
            files: 0,
            bytes: 0,
            last_emit: Instant::now(),
        }
    }

    fn tick(&mut self, files: u64, bytes: u64) {
        self.files += files;
        self.bytes += bytes;
        if self.last_emit.elapsed().as_millis() >= 120 {
            self.last_emit = Instant::now();
            let _ = self.tx.send(CleanMsg::TargetProgress {
                files: self.files,
                bytes: self.bytes,
            });
        }
    }
}

// ---------------------------------------------------------------------------
// 日志
// ---------------------------------------------------------------------------

fn write_log(
    report: &CleanReport,
    detail: &[String],
    opts: CleanOptions,
) -> std::io::Result<PathBuf> {
    let dir = sysutil::logs_dir();
    sysutil::ensure_dir(&dir);
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let path = dir.join(format!("clean-{stamp}.log"));

    let mut out = String::with_capacity(64 * 1024);
    out.push_str("==================================================\n");
    out.push_str(" 磁盘清理工具 · 清理日志\n");
    out.push_str(&format!(
        " 时间     : {}\n",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
    ));
    out.push_str(&format!(
        " 运行平台 : {}\n",
        crate::sysutil::PLATFORM.label()
    ));
    out.push_str(&format!(" 删除方式 : {}\n", opts.mode.label()));
    out.push_str(&format!(" 最小年龄 : {} 天\n", opts.min_age_days));
    out.push_str(&format!(
        " 演练模式 : {}\n",
        if opts.dry_run {
            "是（未真正删除）"
        } else {
            "否"
        }
    ));
    out.push_str("==================================================\n\n");

    for line in detail {
        out.push_str(line);
        out.push('\n');
    }

    out.push_str("\n---------------- 汇总 ----------------\n");
    out.push_str(&format!(
        " 共处理文件 : {}\n 删除目录   : {}\n 处理体积   : {}\n 跳过       : {}\n 安全拦截   : {}\n 耗时       : {} ms\n",
        report.deleted_files,
        report.deleted_dirs,
        human_bytes(report.freed_bytes),
        report.skipped,
        report.blocked,
        report.elapsed_ms
    ));
    if !report.failures.is_empty() {
        out.push_str("\n---------------- 失败明细 ----------------\n");
        for line in &report.failures {
            out.push_str(line);
            out.push('\n');
        }
    }
    if report.canceled {
        out.push_str("\n(本次操作被用户中断)\n");
    }

    std::fs::write(&path, out)?;
    Ok(path)
}

// ---------------------------------------------------------------------------
// 回收站
// ---------------------------------------------------------------------------

/// 回收站当前占用的（字节数, 条目数）。
#[cfg(windows)]
fn recycle_bin_size() -> Option<(u64, u64)> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::Shell::{SHQUERYRBINFO, SHQueryRecycleBinW};

    let mount = sysutil::default_mount();
    let wide: Vec<u16> = std::ffi::OsStr::new(&mount)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut info = SHQUERYRBINFO {
        cbSize: std::mem::size_of::<SHQUERYRBINFO>() as u32,
        i64Size: 0,
        i64NumItems: 0,
    };
    // SAFETY: 宽字符串以 NUL 结尾，结构体在栈上有效。
    let hr = unsafe { SHQueryRecycleBinW(wide.as_ptr(), &mut info) };
    if hr < 0 {
        None
    } else {
        Some((info.i64Size.max(0) as u64, info.i64NumItems.max(0) as u64))
    }
}

#[cfg(not(windows))]
fn recycle_bin_size() -> Option<(u64, u64)> {
    recycle::trash_usage()
}

#[cfg(windows)]
fn empty_recycle_bin() -> Result<(), String> {
    use windows_sys::Win32::UI::Shell::SHEmptyRecycleBinW;

    // SHERB_NOCONFIRMATION | SHERB_NOPROGRESSUI | SHERB_NOSOUND
    const FLAGS: u32 = 0x0000_0001 | 0x0000_0002 | 0x0000_0004;

    // SAFETY: hwnd 传空、root path 传空表示"所有驱动器"。
    let hr = unsafe { SHEmptyRecycleBinW(std::ptr::null_mut(), std::ptr::null(), FLAGS) };
    if hr < 0 {
        Err(format!("系统返回错误码 0x{:08X}", hr as u32))
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn empty_recycle_bin() -> Result<(), String> {
    recycle::empty_trash()
}

/// 把时间戳转成可读字符串。
pub fn format_time(time: Option<SystemTime>) -> String {
    match time {
        Some(t) => {
            let dt: chrono::DateTime<chrono::Local> = t.into();
            dt.format("%Y-%m-%d %H:%M").to_string()
        }
        None => "—".to_string(),
    }
}

// ---------------------------------------------------------------------------
// 单元测试
// ---------------------------------------------------------------------------
//
// 全部只在系统临时目录里自建 / 自删文件，不会碰任何已有数据。
#[cfg(test)]
mod tests {
    use super::*;

    /// 建一个本次测试专用的临时目录，名字里带进程号与序号避免并发冲突。
    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cleanc-test-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("创建测试目录失败");
        dir
    }

    fn write_file(dir: &Path, name: &str, size: usize) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, vec![b'x'; size]).expect("写入测试文件失败");
        path
    }

    fn report() -> CleanTargetReport {
        CleanTargetReport {
            id: "test",
            name: "测试".to_string(),
            ..Default::default()
        }
    }

    /// 永久删除模式下，队列应真的删掉文件并如实统计。
    #[test]
    fn queue_flush_permanently_deletes_and_reports() {
        let dir = scratch("perm");
        let a = write_file(&dir, "a.tmp", 1200);
        let b = write_file(&dir, "b.tmp", 3400);

        let opts = CleanOptions {
            mode: CleanMode::Permanent,
            min_age_days: 0,
            dry_run: false,
        };
        let (tx, _rx) = channel();
        let mut progress = ProgressThrottle::new(&tx);
        let mut report = report();
        let mut logs = Vec::new();
        let mut queue = DeleteQueue::new(&dir);

        queue_file(
            &a,
            &dir,
            0,
            &opts,
            &mut queue,
            &mut report,
            &mut progress,
            &mut logs,
        );
        queue_file(
            &b,
            &dir,
            0,
            &opts,
            &mut queue,
            &mut report,
            &mut progress,
            &mut logs,
        );
        queue.flush(opts.mode, &mut report, &mut progress, &mut logs);

        assert_eq!(report.files, 2, "两个文件都应被删除");
        assert_eq!(report.bytes, 4600);
        assert_eq!(report.skipped, 0);
        assert_eq!(report.blocked, 0);
        assert!(!a.exists() && !b.exists(), "文件应已从磁盘上消失");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 演练模式一个字节都不许动。
    #[test]
    fn dry_run_never_touches_disk() {
        let dir = scratch("dry");
        let a = write_file(&dir, "a.tmp", 512);
        let b = write_file(&dir, "b.tmp", 512);

        let opts = CleanOptions {
            mode: CleanMode::Permanent,
            min_age_days: 0,
            dry_run: true,
        };
        let (tx, _rx) = channel();
        let mut progress = ProgressThrottle::new(&tx);
        let mut report = report();
        let mut logs = Vec::new();
        let mut queue = DeleteQueue::new(&dir);

        queue_file(
            &a,
            &dir,
            0,
            &opts,
            &mut queue,
            &mut report,
            &mut progress,
            &mut logs,
        );
        queue_file(
            &b,
            &dir,
            0,
            &opts,
            &mut queue,
            &mut report,
            &mut progress,
            &mut logs,
        );
        queue.flush(opts.mode, &mut report, &mut progress, &mut logs);

        assert_eq!(report.files, 2, "演练模式仍应统计数量");
        assert_eq!(report.bytes, 1024);
        assert!(a.exists() && b.exists(), "演练模式不允许删除任何文件");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 年龄过滤：太新的文件必须被跳过。
    #[test]
    fn fresh_files_are_skipped_by_age_filter() {
        let dir = scratch("age");
        let a = write_file(&dir, "fresh.tmp", 900);
        let b = write_file(&dir, "old.tmp", 900);

        // 把 b 的修改时间往回拨 40 天
        let old = SystemTime::now() - std::time::Duration::from_secs(40 * 24 * 3600);
        filetime_set(&b, old);

        let opts = CleanOptions {
            mode: CleanMode::Permanent,
            min_age_days: 7,
            dry_run: false,
        };
        let (tx, _rx) = channel();
        let mut progress = ProgressThrottle::new(&tx);
        let mut report = report();
        let mut logs = Vec::new();
        let mut queue = DeleteQueue::new(&dir);

        queue_file(
            &a,
            &dir,
            7,
            &opts,
            &mut queue,
            &mut report,
            &mut progress,
            &mut logs,
        );
        queue_file(
            &b,
            &dir,
            7,
            &opts,
            &mut queue,
            &mut report,
            &mut progress,
            &mut logs,
        );
        queue.flush(opts.mode, &mut report, &mut progress, &mut logs);

        assert_eq!(report.files, 1, "只有旧文件应被处理");
        assert_eq!(report.skipped, 1, "新文件应被年龄过滤跳过");
        assert!(a.exists(), "新文件必须还在");
        assert!(!b.exists(), "旧文件应被删除");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 提交前的二次校验：越界路径必须被拦截，哪怕它被直接塞进队列。
    #[test]
    fn flush_blocks_paths_outside_root() {
        let root = scratch("inside");
        let outside = scratch("outside");
        let inside = write_file(&root, "ok.tmp", 100);
        let escaping = write_file(&outside, "nope.tmp", 100);

        let (tx, _rx) = channel();
        let mut progress = ProgressThrottle::new(&tx);
        let mut report = report();
        let mut logs = Vec::new();
        let mut queue = DeleteQueue::new(&root);

        queue.push(&inside, 100);
        queue.push(&escaping, 100);
        queue.flush(CleanMode::Permanent, &mut report, &mut progress, &mut logs);

        assert_eq!(report.files, 1);
        assert_eq!(report.blocked, 1, "越界路径应被安全层拦截");
        assert!(escaping.exists(), "越界文件绝不能被删除");
        assert!(!inside.exists(), "根目录内的文件应被正常删除");

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&outside);
    }

    /// 空目录要在文件删完之后才被清掉：子目录先移除，父目录随后才可能变空。
    #[test]
    fn directory_walk_removes_files_then_empty_dirs() {
        let dir = scratch("tree");
        std::fs::create_dir_all(dir.join("sub1").join("deep")).unwrap();
        std::fs::create_dir_all(dir.join("sub2")).unwrap();
        write_file(&dir, "loose.tmp", 256);
        write_file(&dir.join("sub1"), "a.tmp", 256);
        write_file(&dir.join("sub1").join("deep"), "b.tmp", 256);

        let opts = CleanOptions {
            mode: CleanMode::Permanent,
            min_age_days: 0,
            dry_run: false,
        };
        let (tx, _rx) = channel();
        let mut progress = ProgressThrottle::new(&tx);
        let mut report = report();
        let mut logs = Vec::new();
        let cancel = AtomicBool::new(false);
        let mut queue = DeleteQueue::new(&dir);

        clean_dir(
            &dir,
            &dir,
            Pass::Everything,
            &[],
            1,
            0,
            &opts,
            &cancel,
            &mut queue,
            &mut report,
            &mut progress,
            &mut logs,
        );
        queue.finish(&opts, &mut report, &mut progress, &mut logs);

        assert_eq!(report.files, 3, "三个文件都应被删除");
        assert_eq!(report.bytes, 768);
        assert_eq!(report.blocked, 0, "不应有任何安全拦截");
        assert_eq!(report.dirs, 3, "sub1/deep、sub1、sub2 都应被清掉");
        assert!(dir.exists(), "清理根目录本身必须保留");
        assert!(!dir.join("sub1").exists());
        assert!(!dir.join("sub2").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 端到端：走完整的 `clean_target` 流程（根校验 → 遍历 → 队列 → 空目录清理）。
    ///
    /// 这条用例同时把「Matched 模式」的语义钉死：
    /// * 只有名字命中规则的**目录**会被放行；
    /// * 一旦命中，该目录的整棵子树都会清空（缓存目录就是这么工作的）；
    /// * 命中的目录本身保留（应用还需要这个路径）；
    /// * 没命中规则的东西一律不动。
    #[test]
    fn clean_target_end_to_end() {
        let dir = scratch("target");
        std::fs::create_dir_all(dir.join("cache").join("inner")).unwrap();
        std::fs::create_dir_all(dir.join("other")).unwrap();
        write_file(&dir, "top.tmp", 100);
        write_file(&dir.join("cache"), "c1.tmp", 200);
        write_file(&dir.join("cache").join("inner"), "c2.tmp", 300);
        write_file(&dir.join("cache"), "stale.dat", 400);
        write_file(&dir.join("other"), "untouched.dat", 500);

        let target = CleanTarget {
            id: "test-target",
            name: "测试类别",
            desc: "",
            group: crate::model::Group::UserFiles,
            risk: crate::model::RiskLevel::Safe,
            requires_admin: false,
            default_on: true,
            age_filtered: false,
            roots: vec![
                Root::matched(dir.clone(), vec![NameRule::exact("cache")]).with_skip_dirs(&[]),
            ],
            note: None,
        };

        let opts = CleanOptions {
            mode: CleanMode::Permanent,
            min_age_days: 0,
            dry_run: false,
        };
        let (tx, _rx) = channel();
        let mut logs = Vec::new();
        let entry = clean_target(&target, &opts, &AtomicBool::new(false), &tx, &mut logs);

        assert_eq!(entry.blocked, 0, "不应有安全拦截");
        assert_eq!(entry.files, 3, "命中目录的整棵子树都应被清空");
        assert_eq!(entry.bytes, 900);
        assert_eq!(entry.dirs, 1, "inner 应作为空目录被移除");

        assert!(dir.join("top.tmp").exists(), "未命中规则的文件必须保留");
        assert!(
            dir.join("other").join("untouched.dat").exists(),
            "未命中规则的目录必须原样保留"
        );
        assert!(dir.join("cache").exists(), "命中的缓存目录本身要保留");
        assert!(
            !dir.join("cache").join("inner").exists(),
            "空子目录应被清理"
        );
        assert_eq!(
            std::fs::read_dir(dir.join("cache")).unwrap().count(),
            0,
            "命中目录应被清空"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 给文件设置修改时间。
    ///
    /// Windows 用 `SetFileTime`，macOS / Linux 用 `utimensat` —— 两个平台都要能
    /// 造出"昨天 / 上个月的文件"，否则年龄过滤的测试就没法跑。
    fn filetime_set(path: &Path, time: SystemTime) {
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            use windows_sys::Win32::Foundation::{FILETIME, HANDLE};
            use windows_sys::Win32::Storage::FileSystem::SetFileTime;

            let file = std::fs::OpenOptions::new()
                .write(true)
                .custom_flags(0x0200_0000) // FILE_FLAG_BACKUP_SEMANTICS
                .open(path)
                .expect("打开文件失败");

            let since_epoch = time
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default();
            // Unix 纪元 → FILETIME：100ns 单位，偏移 11644473600 秒
            let ticks = (since_epoch.as_secs() + 11_644_473_600) * 10_000_000
                + u64::from(since_epoch.subsec_nanos()) / 100;
            let ft = FILETIME {
                dwLowDateTime: (ticks & 0xFFFF_FFFF) as u32,
                dwHighDateTime: (ticks >> 32) as u32,
            };

            use std::os::windows::io::AsRawHandle;
            let handle = file.as_raw_handle() as HANDLE;
            // SAFETY: 句柄来自仍然存活的文件对象，`ft` 在栈上有效。
            let ok = unsafe { SetFileTime(handle, std::ptr::null(), std::ptr::null(), &ft) };
            assert_ne!(ok, 0, "设置文件时间失败");
            drop(file);
        }

        #[cfg(unix)]
        {
            let since_epoch = time
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default();
            let stamp = libc::timespec {
                tv_sec: since_epoch.as_secs() as libc::time_t,
                tv_nsec: i64::from(since_epoch.subsec_nanos()),
            };
            let times = [stamp, stamp];
            let c_path = std::ffi::CString::new(path.to_string_lossy().as_bytes())
                .expect("路径里不该有 NUL");
            // SAFETY: 两个指针都指向调用期间有效的内存（C 字符串 + 栈上数组）。
            let rc = unsafe { libc::utimensat(libc::AT_FDCWD, c_path.as_ptr(), times.as_ptr(), 0) };
            assert_eq!(rc, 0, "设置文件时间失败");
        }
    }
}
