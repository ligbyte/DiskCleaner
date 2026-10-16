//! 大文件 / 大目录扫描：在指定根目录下找出体积最大的文件和目录。
//!
//! 该模块 **只读**，不删除任何东西；界面上对单个文件 / 目录的删除仍会走
//! [`crate::cleaner`] 同款的安全校验流程。
//!
//! 一趟遍历同时产出两份结果：
//!
//! * **大文件** —— 单个文件体积超过阈值；
//! * **大目录** —— 整棵子树的体积合计超过阈值。
//!
//! 两者共用同一次递归：向上返回时顺带把子树的体积、文件数、最新修改时间
//! 汇总给父目录，因此算目录体积没有额外的一次遍历开销。

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Instant, SystemTime};

use crate::advice::{self, Kind};
use crate::model::{BigFile, BigFolder};
use crate::safety;

/// 最多返回的大文件条数。
const MAX_FILES: usize = 400;
/// 最多返回的大目录条数。
const MAX_FOLDERS: usize = 200;

/// 处置建议的分档汇总。
///
/// 大目录之间存在包含关系（`AppData` 囊括 `AppData\Local`），直接把体积相加
/// 会算出远超磁盘容量的数字。所以这里做了一次去重：
///
/// * 目录只统计**没有被其它目录包住**的那些；
/// * 已经算在某个列出的目录里的文件，不再重复计入。
#[derive(Debug, Default, Clone, Copy)]
pub struct AdviceTotals {
    /// 按 [`Kind::index`] 顺序排列的 `(体积, 条目数)`
    pub kinds: [(u64, usize); 5],
    /// 受保护条目单独统计（它们一律归入「建议保留」）
    pub protected: (u64, usize),
}

impl AdviceTotals {
    pub fn of(&self, kind: Kind) -> (u64, usize) {
        self.kinds[kind.index()]
    }
}

/// 把（可能互相包含的）结果整理成"每个字节只算一次"的汇总。
pub fn summarize(files: &[BigFile], folders: &[BigFolder]) -> AdviceTotals {
    let mut out = AdviceTotals::default();

    let top: Vec<&BigFolder> = folders.iter().filter(|f| !f.nested).collect();
    for folder in &top {
        if folder.protected {
            out.protected.0 += folder.size;
            out.protected.1 += 1;
        } else {
            let slot = advice::for_dir(&folder.path).index();
            out.kinds[slot].0 += folder.size;
            out.kinds[slot].1 += 1;
        }
    }

    let keys: Vec<String> = top.iter().map(|f| safety::norm(&f.path)).collect();
    for file in files {
        if file.protected {
            out.protected.0 += file.size;
            out.protected.1 += 1;
            continue;
        }
        // 已经算在上面某个目录里的文件，就不要再算一遍
        let key = safety::norm(&file.path);
        if keys.iter().any(|k| safety::is_within(k, &key)) {
            continue;
        }
        let slot = advice::for_file(&file.path).index();
        out.kinds[slot].0 += file.size;
        out.kinds[slot].1 += 1;
    }

    out
}

#[derive(Debug, Clone)]
pub struct BigFileOptions {
    /// 体积下限（字节），文件与目录共用这一个阈值
    pub min_size: u64,
    pub roots: Vec<PathBuf>,
    /// 是否把大文件列进结果
    pub want_files: bool,
    /// 是否把大目录列进结果
    pub want_folders: bool,
}

pub enum BigFileMsg {
    Started,
    Progress {
        scanned: u64,
        current: String,
    },
    Finished {
        files: Vec<BigFile>,
        folders: Vec<BigFolder>,
        /// 去重后的处置建议汇总
        totals: AdviceTotals,
        scanned: u64,
        elapsed_ms: u64,
        canceled: bool,
    },
}

pub struct BigFileHandle {
    pub rx: Receiver<BigFileMsg>,
    pub cancel: Arc<AtomicBool>,
}

impl BigFileHandle {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

pub fn spawn(opts: BigFileOptions) -> BigFileHandle {
    let (tx, rx) = channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_worker = Arc::clone(&cancel);

    std::thread::spawn(move || {
        let started = Instant::now();
        let _ = tx.send(BigFileMsg::Started);

        let scanned = AtomicU64::new(0);
        let mut walker = Walker {
            opts: &opts,
            cancel: &cancel_worker,
            scanned: &scanned,
            out: Output::default(),
            last_emit: Instant::now(),
            tx: &tx,
        };

        for root in &opts.roots {
            if cancel_worker.load(Ordering::Relaxed) {
                break;
            }
            if !root.is_dir() {
                continue;
            }
            // 顶层若本身就是受保护目录，整棵跳过（例如误把 C:\Windows 加进来）
            if safety::verify_root(root).is_err() && safety::is_protected(root) {
                continue;
            }
            // 根目录自己不进结果列表：它必然最大，列出来也没有可操作性
            walker.run(root);
        }

        let mut out = walker.out;
        out.files.sort_by(|a, b| b.size.cmp(&a.size));
        out.files.truncate(MAX_FILES);
        out.folders.sort_by(|a, b| b.size.cmp(&a.size));
        out.folders.truncate(MAX_FOLDERS);
        mark_nested(&mut out.folders);

        let totals = summarize(&out.files, &out.folders);

        let _ = tx.send(BigFileMsg::Finished {
            files: out.files,
            folders: out.folders,
            totals,
            scanned: scanned.load(Ordering::Relaxed),
            elapsed_ms: started.elapsed().as_millis() as u64,
            canceled: cancel_worker.load(Ordering::Relaxed),
        });
    });

    BigFileHandle { rx, cancel }
}

// ---------------------------------------------------------------------------
// 遍历
// ---------------------------------------------------------------------------

/// 一棵子树的合计数据。
#[derive(Debug, Default, Clone, Copy)]
struct Subtree {
    size: u64,
    files: u64,
    /// 子树内最近一次修改时间
    newest: Option<SystemTime>,
}

impl Subtree {
    fn merge(&mut self, other: &Subtree) {
        self.size += other.size;
        self.files += other.files;
        self.newest = newest_of(self.newest, other.newest);
    }
}

#[derive(Default)]
struct Output {
    files: Vec<BigFile>,
    folders: Vec<BigFolder>,
}

struct Walker<'a> {
    opts: &'a BigFileOptions,
    cancel: &'a AtomicBool,
    scanned: &'a AtomicU64,
    out: Output,
    last_emit: Instant,
    tx: &'a Sender<BigFileMsg>,
}

impl Walker<'_> {
    /// 递归遍历 `dir`，返回整棵子树的合计数据。
    fn run(&mut self, dir: &Path) -> Subtree {
        let mut acc = Subtree::default();
        if self.cancel.load(Ordering::Relaxed) {
            return acc;
        }

        let Ok(reader) = std::fs::read_dir(dir) else {
            return acc;
        };

        for entry in reader.flatten() {
            if self.cancel.load(Ordering::Relaxed) {
                return acc;
            }

            let path = entry.path();
            let Ok(meta) = std::fs::symlink_metadata(&path) else {
                continue;
            };

            // 不跟随符号链接 / 目录联接，避免死循环与越界
            if meta.file_type().is_symlink() {
                continue;
            }

            if meta.is_dir() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str())
                    && crate::sysutil::is_recycle_metadata(name)
                {
                    continue;
                }
                // 系统目录整棵不进：既省时间，也少一处可能出错的地方。
                // 注意这里用的是 is_system_protected 而不是 is_protected ——
                // 「文档 / 下载」这类用户资料目录虽然不可删除，但**要进去算体积**。
                if safety::is_system_protected(&path) {
                    continue;
                }

                let mut sub = self.run(&path);
                // 目录自身的修改时间也算进来，反映"最近有没有动过"
                sub.newest = newest_of(sub.newest, meta.modified().ok());
                acc.merge(&sub);

                if self.opts.want_folders && sub.size >= self.opts.min_size {
                    let protected = safety::is_protected(&path);
                    self.out.folders.push(BigFolder {
                        protected,
                        path,
                        size: sub.size,
                        files: sub.files,
                        modified: sub.newest,
                        nested: false,
                    });
                }
                continue;
            }

            if !meta.is_file() {
                continue;
            }

            let count = self.scanned.fetch_add(1, Ordering::Relaxed) + 1;
            if self.last_emit.elapsed().as_millis() >= 150 {
                self.last_emit = Instant::now();
                let _ = self.tx.send(BigFileMsg::Progress {
                    scanned: count,
                    current: path.to_string_lossy().to_string(),
                });
            }

            let size = meta.len();
            let modified = meta.modified().ok();

            // 体积一律计入目录统计 —— 受保护的文件同样占地方
            acc.size += size;
            acc.files += 1;
            acc.newest = newest_of(acc.newest, modified);

            if !self.opts.want_files || size < self.opts.min_size {
                continue;
            }
            let Some(name) = path.file_name().map(|n| n.to_string_lossy().to_string()) else {
                continue;
            };
            if safety::is_protected_file_name(&name) {
                continue;
            }

            self.out.files.push(BigFile {
                protected: safety::is_protected(&path),
                path,
                size,
                modified,
            });

            // 阶段性裁剪，避免内存无限增长
            if self.out.files.len() > MAX_FILES * 4 {
                self.out.files.sort_by(|a, b| b.size.cmp(&a.size));
                self.out.files.truncate(MAX_FILES * 2);
            }
        }

        acc
    }
}

fn newest_of(a: Option<SystemTime>, b: Option<SystemTime>) -> Option<SystemTime> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (Some(a), None) => Some(a),
        (None, b) => b,
    }
}

/// 标记「已经被更大的目录包住」的条目。
///
/// 列表已按体积降序排列，容器一定排在它所包含的目录之前，所以只看前驱即可。
fn mark_nested(folders: &mut [BigFolder]) {
    let keys: Vec<String> = folders.iter().map(|f| safety::norm(&f.path)).collect();
    for index in 0..folders.len() {
        folders[index].nested = (0..index).any(|j| safety::is_within(&keys[j], &keys[index]));
    }
}

// ---------------------------------------------------------------------------
// 删除
// ---------------------------------------------------------------------------

/// 把某个大文件移入回收站（走与清理相同的安全检查）。
///
/// `roots` 是本次扫描声明的根目录，文件必须落在其中之一，否则拒绝删除。
pub fn recycle_one(path: &Path, roots: &[PathBuf]) -> Result<(), String> {
    let meta = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if meta.file_type().is_symlink() || !meta.is_file() {
        return Err("不是普通文件".to_string());
    }
    let root = roots
        .iter()
        .find(|r| safety::is_within(&safety::norm(r), &safety::norm(path)))
        .ok_or_else(|| "文件不在本次扫描的目录范围内".to_string())?;
    safety::verify(path, root, false).map_err(|r| r.reason().to_string())?;
    crate::recycle::recycle_one(&path.to_path_buf())
}

/// 把一整个大目录移入回收站。
///
/// 比删单个文件危险得多，所以额外加了三道闸：
/// * 必须是一个**真实目录**，不是符号链接、也不是目录联接（重解析点）；
/// * 不能是本次扫描的根目录之一；
/// * 仍然要过 [`safety::verify`]，即落在扫描根之内且不命中任何保护规则。
pub fn recycle_folder(path: &Path, roots: &[PathBuf]) -> Result<(), String> {
    check_folder_target(path, roots)?;
    crate::recycle::recycle_dir(path)
}

/// 删除一个目录前的全部校验，返回它所属的扫描根。
///
/// 单独拆出来是为了能在测试里跑校验、而**不真的去动回收站**。
fn check_folder_target(path: &Path, roots: &[PathBuf]) -> Result<PathBuf, String> {
    let meta = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !meta.is_dir() {
        return Err("不是一个目录".to_string());
    }
    if meta.file_type().is_symlink() || is_reparse_point(&meta) {
        return Err("是符号链接或目录联接，拒绝处理".to_string());
    }

    let target = safety::norm(path);
    if roots.iter().any(|r| safety::norm(r) == target) {
        return Err("这是本次扫描的根目录之一，不能删除".to_string());
    }

    let root = roots
        .iter()
        .find(|r| safety::is_within(&safety::norm(r), &target))
        .ok_or_else(|| "该目录不在本次扫描的范围内".to_string())?;

    safety::verify(path, root, false).map_err(|r| r.reason().to_string())?;
    Ok(root.clone())
}

/// 目录联接（junction）在 Windows 上是重解析点，但 `is_symlink` 未必认出来。
#[cfg(windows)]
fn is_reparse_point(meta: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
    meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_reparse_point(_meta: &std::fs::Metadata) -> bool {
    // macOS / Linux 上"目录联接"直接用符号链接实现，`file_type().is_symlink()`
    // 已经能识别，这里不需要额外判断。
    false
}

// ---------------------------------------------------------------------------
// 测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// 造一棵临时目录树，返回 (根, 守卫)。守卫析构时清理。
    struct TempTree(PathBuf);

    impl TempTree {
        fn new(tag: &str) -> Self {
            let base = std::env::temp_dir().join(format!(
                "cleanc-bigfiles-{tag}-{}-{:?}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&base).unwrap();
            Self(base)
        }

        fn file(&self, rel: &str, bytes: usize) {
            let path = self.0.join(rel);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(&path, vec![b'x'; bytes]).unwrap();
        }
    }

    impl Drop for TempTree {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// 同步跑完一次扫描，返回 (大文件, 大目录)。
    fn scan(
        root: &Path,
        min_size: u64,
        want_files: bool,
        want_folders: bool,
    ) -> (Vec<BigFile>, Vec<BigFolder>) {
        let handle = spawn(BigFileOptions {
            min_size,
            roots: vec![root.to_path_buf()],
            want_files,
            want_folders,
        });
        while let Ok(msg) = handle.rx.recv() {
            if let BigFileMsg::Finished { files, folders, .. } = msg {
                return (files, folders);
            }
        }
        (Vec::new(), Vec::new())
    }

    #[test]
    fn folder_sizes_sum_the_whole_subtree() {
        let tree = TempTree::new("sum");
        tree.file("a/one.bin", 300);
        tree.file("a/two.bin", 300);
        tree.file("a/deep/three.bin", 400);
        tree.file("b/other.bin", 200);

        let (files, folders) = scan(&tree.0, 1024, true, true);
        // 没有任何单个文件超过 1 KB，所以不该有大文件；
        // a 合计 1000 字节 < 1024，b 只有 200，都不该出现
        assert_eq!(files.len(), 0);
        assert_eq!(folders.len(), 0);

        let (_, folders) = scan(&tree.0, 900, true, true);
        assert_eq!(folders.len(), 1, "只有 a 超过 900 字节");
        assert_eq!(folders[0].size, 1000);
        assert_eq!(folders[0].files, 3);
    }

    #[test]
    fn scan_root_itself_is_never_listed() {
        let tree = TempTree::new("root");
        tree.file("sub/big.bin", 4096);
        let (_, folders) = scan(&tree.0, 1024, true, true);
        assert_eq!(folders.len(), 1, "只应列出 sub，不含扫描根自己");
    }

    #[test]
    fn want_switches_filter_the_results() {
        let tree = TempTree::new("switch");
        tree.file("sub/big.bin", 4096);

        let (files, folders) = scan(&tree.0, 1024, true, false);
        assert_eq!((files.len(), folders.len()), (1, 0), "只要文件");

        let (files, folders) = scan(&tree.0, 1024, false, true);
        assert_eq!((files.len(), folders.len()), (0, 1), "只要文件夹");

        let (files, folders) = scan(&tree.0, 1024, false, false);
        assert_eq!((files.len(), folders.len()), (0, 0), "都不勾选就没有结果");
    }

    #[test]
    fn nested_folders_are_flagged() {
        let tree = TempTree::new("nested");
        tree.file("outer/inner/big.bin", 4096);
        tree.file("outer/another.bin", 4096);

        let (_, folders) = scan(&tree.0, 1024, false, true);
        assert_eq!(folders.len(), 2);
        assert_eq!(folders[0].size, 8192, "外层更大，应排在前面");
        assert!(!folders[0].nested, "外层目录不该被标记");
        assert!(folders[1].nested, "内层目录应被标记为已被包住");
    }

    #[test]
    fn recycling_outside_scope_is_refused() {
        let tree = TempTree::new("guard");
        tree.file("sub/big.bin", 64);
        let roots = vec![tree.0.clone()];

        // 根目录本身
        assert!(check_folder_target(&tree.0, &roots).is_err());
        // 不在任何扫描根之下
        assert!(check_folder_target(Path::new(r"C:\Windows"), &roots).is_err());
        // 不存在的路径
        assert!(check_folder_target(&tree.0.join("nope"), &roots).is_err());
        // 传文件进来也不行
        assert!(check_folder_target(&tree.0.join("sub/big.bin"), &roots).is_err());
        // 正常的子目录应当通过（这里只跑校验，不碰回收站）
        assert!(check_folder_target(&tree.0.join("sub"), &roots).is_ok());
    }
}
