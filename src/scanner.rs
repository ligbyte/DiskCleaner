//! 扫描层：按白名单目标统计可清理的体积与文件数。
//!
//! 扫描过程 **只读**，不会修改或删除任何文件；真正的删除在 [`crate::cleaner`] 中，
//! 并且删除前会再次做一遍相同的安全检查。

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Instant, SystemTime};

use rayon::prelude::*;

use crate::model::{CleanTarget, Root, RootMode, TargetResult, TargetStatus};
use crate::safety;

/// 扫描参数。
#[derive(Debug, Clone, Copy, Default)]
pub struct ScanOptions {
    /// 最小文件年龄（天）。0 表示不做年龄限制。
    pub min_age_days: u32,
}

/// 扫描线程发回的进度消息。
pub enum ScanMsg {
    Started { total: usize },
    TargetStarted { name: String },
    TargetFinished(TargetResult),
    Finished { elapsed_ms: u64 },
    Canceled,
}

/// 扫描任务的句柄。
pub struct ScanHandle {
    pub rx: Receiver<ScanMsg>,
    pub cancel: Arc<AtomicBool>,
}

impl ScanHandle {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// 启动一个后台扫描任务。
pub fn spawn(targets: Vec<CleanTarget>, opts: ScanOptions) -> ScanHandle {
    let (tx, rx) = channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_worker = Arc::clone(&cancel);

    std::thread::spawn(move || {
        let started = Instant::now();
        let total = targets.len();
        let _ = tx.send(ScanMsg::Started { total });

        // Sender 不是 Sync，跨 rayon 任务共享需要加锁
        let tx = Mutex::new(tx);

        targets.par_iter().for_each(|target| {
            if cancel_worker.load(Ordering::Relaxed) {
                return;
            }
            if let Ok(guard) = tx.lock() {
                let _ = guard.send(ScanMsg::TargetStarted {
                    name: target.name.to_string(),
                });
            }

            let result = scan_target(target, &opts, &cancel_worker);

            if let Ok(guard) = tx.lock() {
                let _ = guard.send(ScanMsg::TargetFinished(result));
            }
        });

        if let Ok(guard) = tx.lock() {
            if cancel_worker.load(Ordering::Relaxed) {
                let _ = guard.send(ScanMsg::Canceled);
            } else {
                let _ = guard.send(ScanMsg::Finished {
                    elapsed_ms: started.elapsed().as_millis() as u64,
                });
            }
        }
    });

    ScanHandle { rx, cancel }
}

// ---------------------------------------------------------------------------
// 单个目标
// ---------------------------------------------------------------------------

/// 单个根目录的统计结果（合并进 [`TargetResult`] 之前）。
#[derive(Default)]
struct RootStats {
    files: u64,
    dirs: u64,
    bytes: u64,
    skipped: u64,
    denied: bool,
    entries: Vec<(PathBuf, u64, Option<SystemTime>)>,
}

impl RootStats {
    fn push(&mut self, path: PathBuf, size: u64, modified: Option<SystemTime>) {
        self.entries.push((path, size, modified));
        if self.entries.len() > crate::model::PREVIEW_LIMIT * 2 {
            self.entries.sort_by(|a, b| b.1.cmp(&a.1));
            self.entries.truncate(crate::model::PREVIEW_LIMIT);
        }
    }
}

fn scan_target(target: &CleanTarget, opts: &ScanOptions, cancel: &AtomicBool) -> TargetResult {
    let mut result = TargetResult::new(target.id);
    let mut any_exists = false;
    let mut any_denied = false;
    let mut any_content = false;

    let age_days = if target.age_filtered {
        opts.min_age_days
    } else {
        0
    };

    for root in &target.roots {
        if cancel.load(Ordering::Relaxed) {
            break;
        }

        // 回收站走系统 API，不逐文件枚举
        if matches!(root.mode, RootMode::RecycleBin) {
            match recycle_bin_info() {
                Some((bytes, items)) => {
                    any_exists = true;
                    if items > 0 {
                        any_content = true;
                    }
                    result.bytes += bytes;
                    result.files += items;
                }
                None => {
                    any_denied = true;
                }
            }
            continue;
        }

        if !root.path.exists() {
            continue;
        }
        any_exists = true;

        // 双保险：清理根本身也要通过安全检查
        if safety::verify_root(&root.path).is_err() {
            result.skipped += 1;
            continue;
        }

        result.resolved_roots.push(root.path.clone());

        let stats = scan_root(root, age_days, cancel);
        if stats.denied {
            any_denied = true;
        }
        if stats.files > 0 {
            any_content = true;
        }

        result.files += stats.files;
        result.dirs += stats.dirs;
        result.bytes += stats.bytes;
        result.skipped += stats.skipped;
        for (path, size, modified) in stats.entries {
            result.push_entry(path, size, modified);
        }
    }

    result.finalize_preview();
    result.status = if !any_exists {
        TargetStatus::Missing
    } else if any_content {
        if any_denied {
            TargetStatus::Partial
        } else {
            TargetStatus::Ok
        }
    } else if any_denied {
        TargetStatus::Denied
    } else {
        TargetStatus::Empty
    };

    if any_denied && result.skipped > 0 {
        result.message = Some(format!(
            "有 {} 个条目因权限或占用被跳过（可尝试以管理员身份运行）",
            result.skipped
        ));
    }

    result
}

fn scan_root(root: &Root, age_days: u32, cancel: &AtomicBool) -> RootStats {
    let mut stats = RootStats::default();

    match &root.mode {
        RootMode::RecycleBin => {}
        RootMode::ExactFile => scan_exact_file(&root.path, age_days, &mut stats),
        RootMode::ShallowFiles => scan_shallow(&root.path, age_days, &mut stats, cancel),
        RootMode::All => walk(
            &root.path,
            true,
            &[],
            &root.skip_dirs,
            age_days,
            &mut stats,
            cancel,
        ),
        RootMode::Matched(rules) => walk(
            &root.path,
            false,
            rules,
            &root.skip_dirs,
            age_days,
            &mut stats,
            cancel,
        ),
    }

    stats
}

fn scan_exact_file(path: &Path, age_days: u32, stats: &mut RootStats) {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return;
    };
    if meta.file_type().is_symlink() || !meta.is_file() {
        return;
    }
    let modified = meta.modified().ok();
    if safety::too_new(modified, age_days) {
        stats.skipped += 1;
        return;
    }
    stats.files += 1;
    stats.bytes += meta.len();
    stats.push(path.to_path_buf(), meta.len(), modified);
}

fn scan_shallow(root: &Path, age_days: u32, stats: &mut RootStats, cancel: &AtomicBool) {
    let Ok(reader) = std::fs::read_dir(root) else {
        stats.denied = true;
        return;
    };
    for entry in reader.flatten() {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        let path = entry.path();
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            stats.skipped += 1;
            continue;
        };
        if meta.file_type().is_symlink() {
            stats.skipped += 1;
            continue;
        }
        if meta.is_dir() {
            continue;
        }
        let name = entry.file_name();
        if safety::is_protected_file_name(&name.to_string_lossy()) {
            stats.skipped += 1;
            continue;
        }
        let modified = meta.modified().ok();
        if safety::too_new(modified, age_days) {
            stats.skipped += 1;
            continue;
        }
        stats.files += 1;
        stats.bytes += meta.len();
        stats.push(path, meta.len(), modified);
    }
}

/// 通用递归遍历。
///
/// - `match_all == true`：清理目录下的一切（`RootMode::All`）；
/// - `match_all == false`：只清理"名字命中 `rules`"的条目（`RootMode::Matched`），
///   命中目录时改为清理其内容（目录本身保留）。
#[allow(clippy::too_many_arguments)]
fn walk(
    current: &Path,
    match_all: bool,
    rules: &[crate::model::NameRule],
    skip_dirs: &[String],
    age_days: u32,
    stats: &mut RootStats,
    cancel: &AtomicBool,
) {
    walk_inner(
        current, match_all, rules, skip_dirs, 1, age_days, stats, cancel,
    );
}

#[allow(clippy::too_many_arguments)]
fn walk_inner(
    current: &Path,
    match_all: bool,
    rules: &[crate::model::NameRule],
    skip_dirs: &[String],
    depth: usize,
    age_days: u32,
    stats: &mut RootStats,
    cancel: &AtomicBool,
) {
    if cancel.load(Ordering::Relaxed) {
        return;
    }

    let reader = match std::fs::read_dir(current) {
        Ok(r) => r,
        Err(e) => {
            if e.kind() == std::io::ErrorKind::PermissionDenied {
                stats.denied = true;
            }
            stats.skipped += 1;
            return;
        }
    };

    for entry in reader.flatten() {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        let name_lower = name_str.to_ascii_lowercase();

        // 一级子目录排除名单（例如浏览器的 "System Profile"）
        let is_dir_hint = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        if depth == 1 && is_dir_hint && skip_dirs.iter().any(|d| *d == name_lower) {
            stats.skipped += 1;
            continue;
        }

        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            stats.skipped += 1;
            continue;
        };

        // 符号链接 / 目录联接：不跟随、不删除
        if meta.file_type().is_symlink() {
            stats.skipped += 1;
            continue;
        }

        if meta.is_dir() {
            let matched = match_all || rules.iter().any(|r| r.matches(&name_lower));
            if matched {
                // 命中的缓存目录：清理其内容，目录本身保留
                stats.dirs += 1;
                walk_inner(
                    &path,
                    true,
                    rules,
                    skip_dirs,
                    depth + 1,
                    age_days,
                    stats,
                    cancel,
                );
            } else {
                walk_inner(
                    &path,
                    false,
                    rules,
                    skip_dirs,
                    depth + 1,
                    age_days,
                    stats,
                    cancel,
                );
            }
            continue;
        }

        // 文件
        let matched = match_all || rules.iter().any(|r| r.matches(&name_lower));
        if !matched {
            continue;
        }
        if safety::is_protected_file_name(&name_str) {
            stats.skipped += 1;
            continue;
        }
        let modified = meta.modified().ok();
        if safety::too_new(modified, age_days) {
            stats.skipped += 1;
            continue;
        }
        stats.files += 1;
        stats.bytes += meta.len();
        stats.push(path, meta.len(), modified);
    }
}

// ---------------------------------------------------------------------------
// 回收站
// ---------------------------------------------------------------------------

/// 查询回收站占用（字节数, 条目数）。
///
/// Windows 走 Shell 的统计接口；macOS / Linux 直接统计回收站目录本身。
#[cfg(windows)]
fn recycle_bin_info() -> Option<(u64, u64)> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::Shell::{SHQUERYRBINFO, SHQueryRecycleBinW};

    let mount = crate::sysutil::default_mount();
    let wide: Vec<u16> = std::ffi::OsStr::new(&mount)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    let mut info = SHQUERYRBINFO {
        cbSize: std::mem::size_of::<SHQUERYRBINFO>() as u32,
        i64Size: 0,
        i64NumItems: 0,
    };

    // SAFETY: `wide` 是合法的以 NUL 结尾的宽字符串，`info` 是栈上有效结构体。
    let hr = unsafe { SHQueryRecycleBinW(wide.as_ptr(), &mut info) };
    if hr < 0 {
        return None;
    }
    Some((info.i64Size.max(0) as u64, info.i64NumItems.max(0) as u64))
}

#[cfg(not(windows))]
fn recycle_bin_info() -> Option<(u64, u64)> {
    crate::recycle::trash_usage()
}
