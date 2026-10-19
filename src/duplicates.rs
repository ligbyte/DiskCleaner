//! 重复文件扫描：在指定根目录里找出**内容完全相同**的文件。
//!
//! 全程**只读**；界面上对重复副本的删除走 [`crate::bigfiles::recycle_one`]
//! 同一套安全校验（路径必须在扫描根内、不许是符号链接……）。
//!
//! 判定策略分两层，先便宜后昂贵：
//!
//! 1. **按体积分组** —— 体积不同的文件必然不同，一趟遍历就把绝大多数
//!    文件排除掉了；
//! 2. **逐字节比较** —— 同体积的文件两两流式比较（64KB 一块，遇到差异
//!    立即返回），完全相等才算一组重复。
//!
//! 不引入哈希 crate：逐字节比较没有误判，对"要删文件"的场景比对哈希
//! 更稳，而同体积文件在实际目录里非常少，两两比较的开销可以接受。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Instant, SystemTime};

use crate::safety;

/// 参与查重的体积下限（字节）。太小的文件满地都是、清了也没收益，
/// 还会把比较次数抬上去。
pub const MIN_SIZE: u64 = 10 * 1024 * 1024;

/// 单个文件参与比较的体积上限。超过它的文件太罕见也太耗时，直接跳过。
const MAX_COMPARE: u64 = 512 * 1024 * 1024;

/// 一份参与查重的文件。
#[derive(Debug, Clone)]
pub struct DupFile {
    pub path: PathBuf,
    pub size: u64,
    pub modified: Option<SystemTime>,
}

/// 一组内容完全相同的文件（至少两份）。
#[derive(Debug, Clone)]
pub struct DupGroup {
    pub files: Vec<DupFile>,
}

impl DupGroup {
    /// 单份的体积（组内文件完全相同）。
    pub fn size(&self) -> u64 {
        self.files.first().map(|f| f.size).unwrap_or(0)
    }

    /// 清掉多余副本能回收的体积（保留一份）。
    pub fn reclaimable(&self) -> u64 {
        self.size() * self.files.len().saturating_sub(1) as u64
    }

    /// 界面默认保留的那一份：最早修改的（最像"原件"）。
    pub fn keep_index(&self) -> usize {
        self.files
            .iter()
            .enumerate()
            .min_by(|(i, a), (j, b)| {
                a.modified
                    .unwrap_or(SystemTime::UNIX_EPOCH)
                    .cmp(&b.modified.unwrap_or(SystemTime::UNIX_EPOCH))
                    .then(i.cmp(j))
            })
            .map(|(i, _)| i)
            .unwrap_or(0)
    }
}

pub enum DupMsg {
    Started,
    Progress {
        scanned: u64,
        current: String,
    },
    Finished {
        groups: Vec<DupGroup>,
        scanned: u64,
        elapsed_ms: u64,
        canceled: bool,
    },
}

pub struct DupHandle {
    pub rx: Receiver<DupMsg>,
    pub cancel: Arc<AtomicBool>,
}

impl DupHandle {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

pub fn spawn(roots: Vec<PathBuf>) -> DupHandle {
    let (tx, rx) = channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_worker = Arc::clone(&cancel);

    std::thread::spawn(move || {
        let started = Instant::now();
        let _ = tx.send(DupMsg::Started);

        let scanned = AtomicU64::new(0);
        let current = std::sync::Mutex::new(String::new());
        let last_emit = Instant::now();

        // ---- 一趟遍历：把所有 ≥ MIN_SIZE 的文件按体积归堆 ----
        let mut by_size: HashMap<u64, Vec<DupFile>> = HashMap::new();
        for root in &roots {
            if cancel_worker.load(Ordering::Relaxed) {
                break;
            }
            if !root.is_dir() {
                continue;
            }
            // 顶层是受保护目录就整棵跳过（和「大文件」页同一套规则）
            if safety::verify_root(root).is_err() && safety::is_protected(root) {
                continue;
            }
            walk(
                root,
                &cancel_worker,
                &scanned,
                &current,
                &last_emit,
                &tx,
                &mut by_size,
            );
        }

        // ---- 逐字节比较，把同体积堆拆成"内容相同"的簇 ----
        let mut groups: Vec<DupGroup> = Vec::new();
        let mut sizes: Vec<&u64> = by_size.keys().collect();
        sizes.sort();
        for size in sizes {
            if cancel_worker.load(Ordering::Relaxed) {
                break;
            }
            let bucket = &by_size[size];
            if bucket.len() < 2 || *size > MAX_COMPARE {
                continue;
            }
            let mut clusters: Vec<Vec<&DupFile>> = Vec::new();
            for file in bucket {
                let mut placed = false;
                for cluster in &mut clusters {
                    if files_equal(&cluster[0].path, &file.path) {
                        cluster.push(file);
                        placed = true;
                        break;
                    }
                }
                if !placed {
                    clusters.push(vec![file]);
                }
            }
            for cluster in clusters {
                if cluster.len() < 2 {
                    continue;
                }
                let mut files: Vec<DupFile> = cluster.into_iter().cloned().collect();
                files.sort_by(|a, b| a.path.cmp(&b.path));
                groups.push(DupGroup { files });
            }
        }

        // 可回收体积大的排前面
        groups.sort_by(|a, b| b.reclaimable().cmp(&a.reclaimable()));

        let _ = tx.send(DupMsg::Finished {
            groups,
            scanned: scanned.load(Ordering::Relaxed),
            elapsed_ms: started.elapsed().as_millis() as u64,
            canceled: cancel_worker.load(Ordering::Relaxed),
        });
    });

    DupHandle { rx, cancel }
}

/// 递归收集文件。符号链接不跟随，受保护路径整棵跳过。
#[allow(clippy::too_many_arguments)]
fn walk(
    dir: &Path,
    cancel: &AtomicBool,
    scanned: &AtomicU64,
    current: &std::sync::Mutex<String>,
    last_emit: &Instant,
    tx: &Sender<DupMsg>,
    by_size: &mut HashMap<u64, Vec<DupFile>>,
) {
    if cancel.load(Ordering::Relaxed) {
        return;
    }
    // 用**系统级**保护判断：用户资料目录（文档 / 下载…）本身允许进去统计，
    // 只是不把它列为可删除对象 —— 和「大文件」页的遍历规则一致。
    if safety::is_system_protected(dir) {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        let path = entry.path();
        if meta.is_symlink() {
            continue;
        }
        if meta.is_dir() {
            walk(&path, cancel, scanned, current, last_emit, tx, by_size);
            continue;
        }
        if !meta.is_file() || meta.len() < MIN_SIZE {
            continue;
        }

        let dup = DupFile {
            path: path.clone(),
            size: meta.len(),
            modified: meta.modified().ok(),
        };
        by_size.entry(meta.len()).or_default().push(dup);

        let n = scanned.fetch_add(1, Ordering::Relaxed) + 1;
        // 每 400 个文件或每 800ms 汇报一次，别把通道刷爆
        if n % 400 == 0 || last_emit.elapsed().as_millis() > 800 {
            if let Ok(mut cur) = current.lock() {
                *cur = path.to_string_lossy().to_string();
                let _ = tx.send(DupMsg::Progress {
                    scanned: n,
                    current: cur.clone(),
                });
            }
        }
    }
}

/// 流式逐字节比较两个文件。
fn files_equal(a: &Path, b: &Path) -> bool {
    use std::io::Read;
    const CHUNK: usize = 64 * 1024;

    let (mut fa, mut fb) = match (std::fs::File::open(a), std::fs::File::open(b)) {
        (Ok(fa), Ok(fb)) => (fa, fb),
        _ => return false,
    };
    let mut ba = vec![0u8; CHUNK];
    let mut bb = vec![0u8; CHUNK];
    loop {
        let na = match fa.read(&mut ba) {
            Ok(n) => n,
            Err(_) => return false,
        };
        let nb = match fb.read(&mut bb) {
            Ok(n) => n,
            Err(_) => return false,
        };
        if na != nb {
            return false;
        }
        if na == 0 {
            return true;
        }
        if ba[..na] != bb[..nb] {
            return false;
        }
    }
}
