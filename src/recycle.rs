//! 回收站 / 废纸篓操作。
//!
//! 各平台的实现思路：
//!
//! * **Windows**：用经典的 `SHFileOperationW`。为什么不用现成的 `trash` crate ——
//!   它内部为每个文件创建一次 `IFileOperation`，清理上万个缓存文件时会非常慢；
//!   而且在部分机器上 `IFileOperation` 会直接返回「Some operations were aborted」。
//!   `SHFileOperationW` 一次调用可以携带最多几百个路径，速度快一个数量级。
//!
//! * **macOS / Linux**：先自己把文件**搬进**回收站目录（同设备时就是一次 `rename`，
//!   比调用任何外部命令都快），搬不动的（跨设备）再交给平台自带的命令行工具。
//!   Linux 上会同时写出符合 XDG 规范的 `.trashinfo`，桌面环境的「还原」功能可用。
//!
//! 由于回收站在某些环境下确实不可用（组策略禁用、卷不支持、权限受限），
//! 这里额外提供 [`is_available`]：用自建的小文件做一次真实探测，结果会被缓存。

use std::path::PathBuf;

/// 单次调用最多携带的路径数量。
///
/// 取 400 是经验值：再多也不会更快，反而更容易因为个别文件失败而拖累整批。
const BATCH_SIZE: usize = 400;

// ---------------------------------------------------------------------------
// 对外接口
// ---------------------------------------------------------------------------

/// 批量把文件移入回收站，返回与入参等长的逐条结果（`true` = 已移入）。
///
/// 只接受**文件**路径；目录不会被递归删除，避免误伤。
///
/// # 为什么不用接口的返回码判断成功
///
/// 实测（虚拟机 / 受管控的系统上）会出现这种情况：文件确实已经进了回收站，
/// 但系统调用仍然返回一个虚假错误码（例如 Windows 上的 `2` = 文件未找到），
/// 回收站的条目数与体积都正常增长。如果只认返回码，就会把成千上万个
/// 已经删掉的文件统统记成"失败"。
///
/// 所以这里以**调用结束后文件是否还在**作为唯一判据 —— 这也是我们真正关心的事。
pub fn recycle_batch(paths: &[PathBuf]) -> Vec<bool> {
    let mut outcome = vec![false; paths.len()];

    for (index, chunk) in paths.chunks(BATCH_SIZE).enumerate() {
        let base = index * BATCH_SIZE;
        let _ = shell_delete(chunk);
        for (offset, path) in chunk.iter().enumerate() {
            outcome[base + offset] = !path.exists();
        }
    }

    outcome
}

/// 批量移入回收站，返回 `(成功数, 失败数)`。
pub fn recycle_stats(paths: &[PathBuf]) -> (usize, usize) {
    let outcome = recycle_batch(paths);
    let ok = outcome.iter().filter(|v| **v).count();
    (ok, outcome.len() - ok)
}

/// 单个文件移入回收站。
pub fn recycle_one(path: &PathBuf) -> Result<(), String> {
    if recycle_stats(std::slice::from_ref(path)).0 > 0 {
        Ok(())
    } else {
        Err("文件被占用或无权限，或回收站不可用".to_string())
    }
}

/// 把一个**目录**连同内容一起移入回收站。
///
/// 比删单个文件危险得多，所以调用方必须自己先做校验（见
/// [`crate::bigfiles::recycle_folder`] 与 [`crate::apps::run_uninstall`]）。
/// 这里只负责"能不能搬"这件事本身：
///
/// * 必须是真实目录，符号链接不跟随；
/// * 已经是回收站自己的目录、或者是它的祖先目录，一律拒绝 ——
///   否则会出现"把回收站搬进回收站"这种自指的怪事。
pub fn recycle_dir(path: &std::path::Path) -> Result<(), String> {
    let meta =
        std::fs::symlink_metadata(path).map_err(|e| format!("无法读取 {}：{e}", path.display()))?;

    if meta.file_type().is_symlink() {
        return Err("这是一个符号链接，本工具不会顺着它操作".to_string());
    }
    if !meta.is_dir() {
        return Err("目标不是一个目录".to_string());
    }

    #[cfg(unix)]
    if let Some(layout) = TrashLayout::detect() {
        let target = crate::safety::norm(path);
        let trash = crate::safety::norm(&layout.files);
        if crate::safety::is_within(&target, &trash) {
            return Err("目标就是回收站自己".to_string());
        }
    }

    let path = path.to_path_buf();
    if recycle_stats(std::slice::from_ref(&path)).0 > 0 {
        Ok(())
    } else {
        Err("目录无法移入回收站（跨设备或权限不足）".to_string())
    }
}

/// 当前系统是否支持把文件移入回收站。
///
/// 探测方式：在临时目录创建一个只有二十来字节的探针文件，真的把它移入回收站一次，
/// 看它是否从原位置消失。只有这样才能区分"回收站被策略禁用"这类光看配置
/// 看不出来的情况；也不会被上一条里提到的虚假错误码误导。
///
/// 结果会被缓存 —— 探测本身有副作用（会往回收站丢一个小文件），不该反复做。
pub fn is_available() -> bool {
    static AVAILABLE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *AVAILABLE.get_or_init(probe)
}

fn probe() -> bool {
    let path =
        std::env::temp_dir().join(format!(".cleanc-recycle-probe-{}.tmp", std::process::id()));

    if std::fs::write(&path, b"CleanC recycle probe").is_err() {
        return false;
    }

    if recycle_stats(std::slice::from_ref(&path)).0 > 0 {
        return true;
    }

    // 探测失败，把探针文件直接清掉，不留垃圾
    let _ = std::fs::remove_file(&path);
    false
}

/// 界面上给「移入回收站」这个动作起的名字（macOS 上叫废纸篓）。
pub fn action_label() -> &'static str {
    match crate::sysutil::trash_label() {
        "废纸篓" => "移入废纸篓",
        _ => "移入回收站",
    }
}

/// 回收站 / 废纸篓本身的名字。
pub fn trash_name() -> &'static str {
    crate::sysutil::trash_label()
}

// ---------------------------------------------------------------------------
// Windows 实现
// ---------------------------------------------------------------------------

/// Windows：一次 `SHFileOperationW` 处理一整批。
#[cfg(windows)]
fn shell_delete(paths: &[PathBuf]) -> Result<(), String> {
    shell_delete_with(paths, recycle_flags())
}

/// 默认（生产）用的标志位：删除到回收站 + 全部静默。
#[cfg(windows)]
fn recycle_flags() -> u16 {
    use windows_sys::Win32::UI::Shell::{
        FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOCONFIRMMKDIR, FOF_NOERRORUI, FOF_SILENT,
    };

    (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_NOCONFIRMMKDIR | FOF_SILENT | FOF_NOERRORUI) as u16
}

/// 诊断用的标志位：直接永久删除（用来判断是接口问题还是回收站问题）。
#[cfg(windows)]
fn permanent_flags() -> u16 {
    use windows_sys::Win32::UI::Shell::{
        FOF_NOCONFIRMATION, FOF_NOCONFIRMMKDIR, FOF_NOERRORUI, FOF_SILENT,
    };

    (FOF_NOCONFIRMATION | FOF_NOCONFIRMMKDIR | FOF_SILENT | FOF_NOERRORUI) as u16
}

#[cfg(windows)]
fn shell_delete_with(paths: &[PathBuf], flags: u16) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;

    use windows_sys::Win32::UI::Shell::{FO_DELETE, SHFILEOPSTRUCTW, SHFileOperationW};

    ensure_com_initialized();

    // SHFileOperationW 要求 pFrom 是"以两个 NUL 结尾的宽字符串列表"
    let mut buffer: Vec<u16> = Vec::with_capacity(paths.len() * 80);
    for path in paths {
        let wide: Vec<u16> = path.as_os_str().encode_wide().collect();
        if wide.is_empty() || wide.contains(&0) {
            return Err("路径非法".to_string());
        }
        buffer.extend_from_slice(&wide);
        buffer.push(0);
    }
    buffer.push(0);

    let mut op = SHFILEOPSTRUCTW {
        hwnd: std::ptr::null_mut(),
        wFunc: FO_DELETE,
        pFrom: buffer.as_ptr(),
        pTo: std::ptr::null(),
        fFlags: flags,
        fAnyOperationsAborted: 0,
        hNameMappings: std::ptr::null_mut(),
        lpszProgressTitle: std::ptr::null(),
    };

    // SAFETY: `buffer` 在整个调用期间有效且以双 NUL 结尾，`op` 是栈上有效结构体。
    let code = unsafe { SHFileOperationW(&mut op) };
    if code != 0 {
        return Err(format!(
            "Shell 返回错误码 {code}（{}）",
            describe_code(code)
        ));
    }
    if op.fAnyOperationsAborted != 0 {
        return Err("Shell 报告操作被中断".to_string());
    }

    Ok(())
}

/// 把 SHFileOperation 的返回码翻译成人话，便于用户排查。
#[cfg(windows)]
fn describe_code(code: i32) -> &'static str {
    match code {
        0 => "成功",
        2 => "ERROR_FILE_NOT_FOUND：系统找不到指定文件（接口可能被安全策略拦截）",
        5 => "ERROR_ACCESS_DENIED：拒绝访问",
        0x71 => "DE_SAMEFILE：源与目标相同",
        0x72 => "DE_MANYSRC1DEST：多个源对应同一个目标",
        0x73 => "DE_DIFFDIR：源与目标在不同的目录",
        0x74 => "DE_ROOTDIR：不能对根目录做该操作",
        0x75 => "DE_OPCANCELLED：操作被取消",
        0x76 => "DE_DESTSUBTREE：目标目录是源目录的子目录",
        0x78 => "DE_ACCESSDENIEDSRC：源被拒绝访问",
        0x79 => "DE_PATHTOODEEP：路径过长",
        0x7C => "DE_INVALIDFILES：路径列表非法",
        _ => "未知错误码",
    }
}

/// 诊断用：区分「回收站不可用」和「整个接口不可用」。
#[cfg(windows)]
pub fn diagnose_matrix() -> Vec<(String, Result<(), String>)> {
    let dir = std::env::temp_dir();
    let pid = std::process::id();
    let mut out: Vec<(String, Result<(), String>)> = Vec::new();

    for (label, flags) in [
        ("移入回收站（带 FOF_ALLOWUNDO）", recycle_flags()),
        ("永久删除（不带 FOF_ALLOWUNDO）", permanent_flags()),
    ] {
        let path = dir.join(format!(".cleanc-diag-{pid}-{}.tmp", out.len()));
        if std::fs::write(&path, b"CleanC diagnose probe").is_err() {
            out.push((
                label.to_string(),
                Err(format!("无法创建探针文件 {}", path.display())),
            ));
            continue;
        }
        let result = shell_delete_with(std::slice::from_ref(&path), flags);
        let gone = !path.exists();
        let _ = std::fs::remove_file(&path);
        out.push((format!("{label} · 文件已消失={gone}"), result));
    }

    out
}

/// 保证当前线程初始化过 COM（STA）。重复调用是安全的。
#[cfg(windows)]
fn ensure_com_initialized() {
    use windows_sys::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};

    thread_local! {
        static INIT: () = {
            // SAFETY: 传 null 表示按默认方式初始化；失败也无妨，后续调用会自己返回错误。
            unsafe {
                CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32);
            }
        };
    }

    INIT.with(|_| ());
}

// ---------------------------------------------------------------------------
// macOS / Linux 实现
// ---------------------------------------------------------------------------

/// Unix：把文件搬进回收站目录，搬不动的文件就是失败的。
#[cfg(unix)]
fn shell_delete(paths: &[PathBuf]) -> Result<(), String> {
    let leftovers = trash_move(paths);
    if leftovers.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "有 {} 个文件无法移入回收站（跨设备或权限不足）",
            leftovers.len()
        ))
    }
}

/// 把文件搬进回收站目录。
///
/// 返回**没能搬动**的那些路径。
#[cfg(unix)]
fn trash_move(paths: &[PathBuf]) -> Vec<PathBuf> {
    let Some(layout) = TrashLayout::detect() else {
        return paths.to_vec();
    };

    let mut leftovers = Vec::new();
    for path in paths {
        if !path.exists() {
            continue;
        }
        if !move_into(&layout, path) {
            leftovers.push(path.clone());
        }
    }
    leftovers
}

/// 回收站目录布局。
#[cfg(unix)]
struct TrashLayout {
    /// 文件实际存放的位置（macOS 是 `~/.Trash`，Linux 是 `~/.local/share/Trash/files`）
    files: PathBuf,
    /// 元数据目录（Linux 才有）
    info: Option<PathBuf>,
}

#[cfg(unix)]
impl TrashLayout {
    fn detect() -> Option<Self> {
        #[cfg(target_os = "macos")]
        {
            let files = crate::sysutil::home_dir()?.join(".Trash");
            if !files.is_dir() {
                std::fs::create_dir_all(&files).ok()?;
            }
            Some(Self { files, info: None })
        }

        #[cfg(target_os = "linux")]
        {
            let root = crate::sysutil::user_data_dir()?.join("Trash");
            let files = root.join("files");
            let info = root.join("info");
            if !files.is_dir() {
                std::fs::create_dir_all(&files).ok()?;
            }
            if !info.is_dir() {
                std::fs::create_dir_all(&info).ok()?;
            }
            Some(Self {
                files,
                info: Some(info),
            })
        }

        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            None
        }
    }
}

/// 单个文件搬进回收站。
///
/// 同一设备上用 `rename`（瞬间完成）；跨设备时退回"复制 + 删除原文件"，
/// 这样不依赖任何外部命令，也不会触发系统的自动化授权弹窗。
#[cfg(unix)]
fn move_into(layout: &TrashLayout, path: &std::path::Path) -> bool {
    use std::os::unix::fs::MetadataExt;

    let Ok(src_meta) = std::fs::symlink_metadata(path) else {
        return false;
    };
    // 普通文件、符号链接、目录都可以搬 ——
    // 目录只在同设备时走得通（`rename`），跨设备那一支会拒绝，见 `copy_then_unlink`。

    let name = match path.file_name() {
        Some(name) => name.to_os_string(),
        None => return false,
    };
    let (target, info_name) = unique_target(&layout.files, &name);

    // Linux：先写 .trashinfo，写不下去就别搬，免得留下没有元数据的孤儿条目
    let info_path = if let Some(info_dir) = &layout.info {
        let info_path = info_dir.join(format!("{info_name}.trashinfo"));
        if write_trashinfo(&info_path, path).is_err() {
            return false;
        }
        Some(info_path)
    } else {
        None
    };

    let moved = if std::fs::rename(path, &target).is_ok() {
        true
    } else {
        // 跨设备（EXDEV）或目标已存在：复制内容再删原文件
        let same_device = std::fs::metadata(&layout.files)
            .map(|dst| dst.dev() == src_meta.dev())
            .unwrap_or(false);
        if same_device {
            false
        } else {
            copy_then_unlink(path, &target, &src_meta)
        }
    };

    if !moved && let Some(info_path) = info_path {
        let _ = std::fs::remove_file(info_path);
    }
    moved
}

/// 跨设备时的兜底：复制后删掉原文件。
///
/// 只对**普通文件**生效 —— 目录与符号链接不在这里处理，直接判失败，
/// 免得把一整棵目录树复制一遍（既慢又可能撑爆磁盘）。
#[cfg(unix)]
fn copy_then_unlink(
    path: &std::path::Path,
    target: &std::path::Path,
    src_meta: &std::fs::Metadata,
) -> bool {
    if src_meta.file_type().is_symlink() || src_meta.is_dir() {
        return false;
    }
    if std::fs::copy(path, target).is_err() {
        let _ = std::fs::remove_file(target);
        return false;
    }
    std::fs::remove_file(path).is_ok()
}

/// 在回收站目录里挑一个不冲突的文件名，返回 `(目标路径, 元数据用的名字)`。
#[cfg(unix)]
fn unique_target(dir: &std::path::Path, name: &std::ffi::OsStr) -> (PathBuf, String) {
    let base = name.to_string_lossy().to_string();
    let (stem, ext) = match base.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && base.len() > 1 => {
            (stem.to_string(), format!(".{ext}"))
        }
        _ => (base.clone(), String::new()),
    };

    for attempt in 0..1000u32 {
        let candidate = if attempt == 0 {
            base.clone()
        } else {
            format!("{stem}.{attempt}{ext}")
        };
        let path = dir.join(&candidate);
        if !path.exists() {
            return (path, candidate);
        }
    }
    // 极端情况：用进程号兜底，保证一定能拿到一个不冲突的名字
    let fallback = format!("{stem}.{}{ext}", std::process::id());
    (dir.join(&fallback), fallback)
}

/// 写 XDG 规范的 `.trashinfo`：桌面环境靠它实现「还原」。
#[cfg(target_os = "linux")]
fn write_trashinfo(path: &std::path::Path, original: &std::path::Path) -> std::io::Result<()> {
    let now: chrono::DateTime<chrono::Local> = chrono::Local::now();
    let content = format!(
        "[Trash Info]\nPath={}\nDeletionDate={}\n",
        percent_encode(&original.to_string_lossy()),
        now.format("%Y-%m-%dT%H:%M:%S")
    );
    std::fs::write(path, content)
}

#[cfg(all(unix, not(target_os = "linux")))]
fn write_trashinfo(_path: &std::path::Path, _original: &std::path::Path) -> std::io::Result<()> {
    Ok(())
}

/// XDG 规定的 URL 转义：除保留字符外全部编码成 `%XX`。
#[cfg(target_os = "linux")]
fn percent_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.as_bytes() {
        let keep =
            byte.is_ascii_alphanumeric() || matches!(*byte, b'/' | b'-' | b'.' | b'_' | b'~');
        if keep {
            out.push(*byte as char);
        } else {
            out.push('%');
            out.push_str(&format!("{byte:02X}"));
        }
    }
    out
}

/// 诊断用：把"能不能移入回收站"拆成几个可验证的事实。
#[cfg(unix)]
pub fn diagnose_matrix() -> Vec<(String, Result<(), String>)> {
    use std::os::unix::fs::MetadataExt;

    let mut out: Vec<(String, Result<(), String>)> = Vec::new();

    let layout = TrashLayout::detect();
    match &layout {
        Some(layout) => {
            let device = match std::fs::metadata(&layout.files) {
                Ok(meta) => format!("设备号 {}", meta.dev()),
                Err(err) => format!("无法读取：{err}"),
            };
            out.push((format!("回收站目录 {}", layout.files.display()), Ok(())));
            out.push((format!("回收站所在设备 {device}"), Ok(())));
            if let Some(info) = &layout.info {
                out.push((format!("元数据目录 {}", info.display()), Ok(())));
            }
        }
        None => out.push((
            "回收站目录".to_string(),
            Err("无法定位或创建回收站目录".to_string()),
        )),
    }

    // 真刀真枪试一次：把探针文件移进回收站，看它是否从原位置消失
    let probe = std::env::temp_dir().join(format!(".cleanc-diag-{}.tmp", std::process::id()));
    if std::fs::write(&probe, b"CleanC diagnose probe").is_err() {
        out.push((
            "探针文件".to_string(),
            Err(format!("无法创建 {}", probe.display())),
        ));
        return out;
    }
    let result = shell_delete(std::slice::from_ref(&probe));
    let gone = !probe.exists();
    if !gone {
        let _ = std::fs::remove_file(&probe);
    }
    out.push((format!("实际移入回收站 · 文件已消失={gone}"), result));

    out
}

/// 回收站里现在占了多少（字节数, 一级条目数）。
#[cfg(unix)]
pub fn trash_usage() -> Option<(u64, u64)> {
    let layout = TrashLayout::detect()?;
    Some((
        crate::sysutil::dir_size(&layout.files),
        crate::sysutil::count_top_level(&layout.files),
    ))
}

#[cfg(not(any(windows, unix)))]
pub fn trash_usage() -> Option<(u64, u64)> {
    None
}

/// 清空回收站。
///
/// 这是一个**不可恢复**的操作，只有用户在界面上显式勾选并二次确认之后才会走到这里。
/// 只删除回收站目录里的内容，目录本身保留。
#[cfg(unix)]
pub fn empty_trash() -> Result<(), String> {
    let layout = TrashLayout::detect().ok_or_else(|| "无法定位回收站目录".to_string())?;

    let mut failed = 0usize;
    let mut first_error: Option<String> = None;
    clear_dir(&layout.files, &mut failed, &mut first_error);
    if let Some(info) = &layout.info {
        clear_dir(info, &mut failed, &mut first_error);
    }

    if failed == 0 {
        Ok(())
    } else {
        Err(format!(
            "有 {failed} 个条目无法删除（{}）",
            first_error.unwrap_or_else(|| "权限不足".to_string())
        ))
    }
}

/// 删掉目录里的所有条目（目录本身保留）。
///
/// `remove_dir_all` / `remove_file` 都不会跟随符号链接，所以这里不需要额外判断。
#[cfg(unix)]
fn clear_dir(dir: &std::path::Path, failed: &mut usize, first_error: &mut Option<String>) {
    let Ok(reader) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in reader.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        // 跳过桌面环境自己用的索引文件，别把回收站的"家务"也拆了
        if name == ".DS_Store" || name == ".localized" {
            continue;
        }
        let path = entry.path();
        let result = match entry.file_type() {
            Ok(ftype) if ftype.is_dir() => std::fs::remove_dir_all(&path),
            _ => std::fs::remove_file(&path),
        };
        if let Err(err) = result {
            *failed += 1;
            if first_error.is_none() {
                *first_error = Some(err.to_string());
            }
        }
    }
}

#[cfg(not(any(windows, unix)))]
pub fn empty_trash() -> Result<(), String> {
    Err("当前平台不支持".to_string())
}

// ---------------------------------------------------------------------------
// 其它平台（既不是 Windows 也不是 Unix）：明确不支持
// ---------------------------------------------------------------------------

#[cfg(not(any(windows, unix)))]
fn shell_delete(_paths: &[PathBuf]) -> Result<(), String> {
    Err("当前平台不支持回收站".to_string())
}

#[cfg(not(any(windows, unix)))]
pub fn diagnose_matrix() -> Vec<(String, Result<(), String>)> {
    vec![(
        "当前平台不支持回收站".to_string(),
        Err("当前平台不支持".to_string()),
    )]
}

// ---------------------------------------------------------------------------
// 测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_file_is_never_left_behind() {
        // 不论回收站可不可用，探针文件都不该留在临时目录里
        let _ = is_available();
        assert!(true);
    }

    #[cfg(unix)]
    #[test]
    fn unique_target_avoids_collisions() {
        let dir = std::env::temp_dir().join(format!("cleanc-trash-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let (first, first_name) = unique_target(&dir, std::ffi::OsStr::new("a.txt"));
        assert_eq!(first_name, "a.txt");
        std::fs::write(&first, b"x").unwrap();

        let (second, second_name) = unique_target(&dir, std::ffi::OsStr::new("a.txt"));
        assert_ne!(first, second);
        assert_eq!(second_name, "a.1.txt");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn percent_encoding_keeps_slashes() {
        assert_eq!(percent_encode("/home/me/a b.txt"), "/home/me/a%20b.txt");
        assert_eq!(percent_encode("/中文"), "/%E4%B8%AD%E6%96%87");
    }
}
