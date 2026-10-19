//! 磁盘清理工具 · 程序入口。
//!
//! 这是一个纯本地运行的跨平台磁盘清理工具（Windows / macOS / Linux）：
//! * 只清理代码里硬编码的白名单目录；
//! * 删除前逐条做安全检查（详见 `safety` 模块）；
//! * 默认把文件移入回收站，可一键切换为永久删除；
//! * 界面基于 egui / eframe，支持浅色与深色主题。

// 发布构建不弹控制台窗口（只对 Windows 有意义）
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod advice;
mod app;
mod apps;
mod bigfiles;
mod cleaner;
mod duplicates;
mod model;
mod recycle;
mod safety;
mod scanner;
mod settings;
mod sysutil;
mod targets;
mod theme;
mod ui;

use eframe::egui;

fn main() -> eframe::Result<()> {
    // 命令行自检模式：不开窗口，直接把扫描结果打到控制台，便于排障
    if std::env::args().any(|a| a == "--selftest" || a == "-t") {
        selftest();
        return Ok(());
    }

    // 演练清理：完整走一遍清理流程但不真正删除，用于验证白名单与安全校验
    if std::env::args().any(|a| a == "--dryrun") {
        dryrun();
        return Ok(());
    }

    // 回收站通道检测：只对自建的临时文件生效
    if std::env::args().any(|a| a == "--test-trash") {
        test_trash();
        return Ok(());
    }

    // 列出已安装软件及其占用空间（只读）
    if std::env::args().any(|a| a == "--list-apps") {
        list_apps();
        return Ok(());
    }

    // 统计大文件 / 大目录并打印处置建议（只读）
    if let Some(thresh) = bigfiles_threshold() {
        list_big(thresh);
        return Ok(());
    }

    match run(eframe::Renderer::Wgpu) {
        Ok(()) => Ok(()),
        Err(err) => {
            // 部分虚拟机 / 远程桌面没有可用的 D3D12 适配器，退回 OpenGL 后端再试一次
            eprintln!("wgpu 后端初始化失败：{err}");
            eprintln!("正在尝试 OpenGL 后端…");
            run(eframe::Renderer::Glow)
        }
    }
}

fn run(renderer: eframe::Renderer) -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        renderer,
        // 启动时把窗口放到屏幕正中间
        centered: true,
        viewport: egui::ViewportBuilder::default()
            .with_title("磁盘清理 · CleanC")
            .with_inner_size([1120.0, 760.0])
            .with_min_inner_size([960.0, 640.0])
            .with_icon(app_icon())
            // 关掉系统自带的标题栏与最小化 / 最大化 / 关闭，改由 `chrome::title_bar`
            // 自绘一套（样式取自设计稿）。窗口因此没有原生边框，缩放交给
            // `chrome::resize_handles` 在三条边和四个角上补回来。
            .with_decorations(false)
            .with_resizable(true),
        ..Default::default()
    };

    eframe::run_native(
        "磁盘清理",
        options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc)))),
    )
}

// ---------------------------------------------------------------------------
// 命令行自检
// ---------------------------------------------------------------------------

/// `clean_c --selftest`：只读地跑一遍扫描，验证环境与白名单是否正常。
fn selftest() {
    println!("=== 磁盘清理工具 · 自检 ===\n");

    // 五个标签都是 4 个字，后面固定跟 "   : "，列就对齐了
    println!("运行平台   : {}", sysutil::PLATFORM.label());
    println!(
        "权限状态   : {}",
        if sysutil::is_admin() {
            "已有（系统级目录一并纳入）".to_string()
        } else {
            format!(
                "没有（系统级目录会被跳过）· 需要 {} 权限",
                sysutil::privilege_label()
            )
        }
    );

    let mount = sysutil::default_mount();
    match sysutil::disk_info(&mount) {
        Some(d) => println!(
            "磁盘容量   : {} 总 {} / 已用 {} / 可用 {}",
            sysutil::mount_label(&mount),
            model::human_bytes(d.total),
            model::human_bytes(d.used()),
            model::human_bytes(d.free)
        ),
        None => println!("磁盘容量   : {} 读取失败", sysutil::mount_label(&mount)),
    }

    println!("中文字体   : {}", theme::probe_cjk_font());

    match decode_ico(APP_ICON_BYTES) {
        Ok(icon) => println!(
            "程序图标   : {}×{}（来自 assets/app_icon.ico，已嵌入可执行文件）",
            icon.width, icon.height
        ),
        Err(err) => println!("程序图标   : assets/app_icon.ico 解码失败（{err}），将退回内置图标"),
    }

    println!("\n--- 安全层抽样校验 ---");
    #[cfg(windows)]
    let samples: &[(&str, bool)] = &[
        (r"C:\Users\Public\Documents\a.txt", true),
        (r"C:\Windows\System32\kernel32.dll", true),
        (r"C:\Program Files\App\a.exe", true),
        (r"C:\Windows\Temp\abc.tmp", false),
        (r"C:\Users\X\AppData\Local\Temp\x.log", false),
    ];
    #[cfg(target_os = "macos")]
    let samples: &[(&str, bool)] = &[
        ("/System/Library/CoreServices/x", true),
        ("/usr/bin/ls", true),
        ("/etc/hosts", true),
        ("/Applications/Safari.app", true),
        ("/Users/x/Documents/a.txt", true),
        ("/Users/x/Library/Mail/a.eml", true),
        ("/var/log/system.log", false),
        ("/Users/x/Library/Caches/com.foo/x", false),
    ];
    #[cfg(target_os = "linux")]
    let samples: &[(&str, bool)] = &[
        ("/usr/bin/ls", true),
        ("/etc/hosts", true),
        ("/var/lib/dpkg/status", true),
        ("/home/x/.ssh/id_rsa", true),
        ("/home/x/Documents/a.txt", true),
        ("/var/log/syslog", false),
        ("/var/cache/apt/archives/a.deb", false),
        ("/tmp/build/x.o", false),
    ];
    for (path, expect_protected) in samples {
        let actual = safety::is_protected(std::path::Path::new(path));
        let flag = if actual == *expect_protected {
            "ok  "
        } else {
            "FAIL"
        };
        println!("  [{flag}] {:40} 受保护={actual}", path);
    }

    println!("\n--- 扫描白名单目录（只读） ---");
    let targets = targets::build();
    println!("共 {} 个清理类别，最小文件年龄 7 天\n", targets.len());

    let handle = scanner::spawn(targets.clone(), scanner::ScanOptions { min_age_days: 7 });

    let mut results: Vec<model::TargetResult> = Vec::new();
    let mut elapsed = 0u64;
    while let Ok(msg) = handle.rx.recv() {
        match msg {
            scanner::ScanMsg::TargetFinished(r) => results.push(r),
            scanner::ScanMsg::Finished { elapsed_ms } => {
                elapsed = elapsed_ms;
                break;
            }
            scanner::ScanMsg::Canceled => break,
            _ => {}
        }
    }

    results.sort_by(|a, b| b.bytes.cmp(&a.bytes));

    println!(
        "{} {:>12} {:>10}  {}",
        pad_display("类别", 26),
        "体积",
        "文件数",
        "状态"
    );
    println!("{}", "-".repeat(66));
    let mut total = 0u64;
    let mut total_files = 0u64;
    for r in &results {
        let name = targets
            .iter()
            .find(|t| t.id == r.id)
            .map(|t| t.name)
            .unwrap_or(r.id);
        println!(
            "{} {:>12} {:>10}  {}",
            pad_display(name, 26),
            model::human_bytes(r.bytes),
            model::thousands(r.files),
            r.status.label()
        );
        total += r.bytes;
        total_files += r.files;
    }
    println!("{}", "-".repeat(66));
    println!(
        "合计可清理 {} / {} 个文件，扫描耗时 {} ms",
        model::human_bytes(total),
        model::thousands(total_files),
        elapsed
    );

    let blocked: u64 = results.iter().map(|r| r.skipped).sum();
    println!(
        "因占用 / 权限 / 年龄被跳过：{} 条",
        model::thousands(blocked)
    );

    println!("\n--- {}{} ---", recycle::trash_name(), "通道");
    let available = recycle::is_available();
    println!(
        "{} : {}",
        recycle::action_label(),
        if available {
            format!("可用（探针文件已成功移入{}）", recycle::trash_name())
        } else {
            "不可用（探针已实测失败，建议改用永久删除）".to_string()
        }
    );

    println!("\n自检完成。GUI 模式下不会自动删除任何文件。");
}

/// `clean_c --dryrun`：完整执行一遍清理流程，但 `dry_run = true` 不会删除任何文件。
///
/// 它会逐条调用与真实清理相同的安全检查，因此：
/// * 报告的「安全拦截」必须为 0，否则说明白名单或保护名单配置有误；
/// * 「跳过」反映的是文件占用 / 权限 / 年龄过滤的真实情况。
fn dryrun() {
    println!("=== 磁盘清理工具 · 演练清理（不会删除任何文件）===\n");

    let targets: Vec<model::CleanTarget> = targets::build()
        .into_iter()
        .filter(|t| t.default_on)
        .collect();
    println!("默认勾选 {} 个类别，开始演练…\n", targets.len());

    let opts = cleaner::CleanOptions {
        mode: cleaner::CleanMode::RecycleBin,
        min_age_days: 7,
        dry_run: true,
    };

    let handle = cleaner::spawn(targets, opts);
    let report = loop {
        match handle.rx.recv() {
            Ok(cleaner::CleanMsg::Finished(r)) | Ok(cleaner::CleanMsg::Canceled(r)) => break r,
            Ok(_) => {}
            Err(_) => break model::CleanReport::default(),
        }
    };

    for entry in &report.entries {
        println!(
            "  {} {:>10} / {:>8} 个文件   跳过 {}",
            pad_display(&entry.name, 26),
            model::human_bytes(entry.bytes),
            model::thousands(entry.files),
            entry.skipped
        );
    }

    println!("\n{}", "-".repeat(70));
    println!("预计可释放 : {}", model::human_bytes(report.freed_bytes));
    println!("涉及文件   : {} 个", model::thousands(report.deleted_files));
    println!("空目录     : {} 个", model::thousands(report.deleted_dirs));
    println!(
        "跳过       : {} 条（占用 / 权限 / 年龄）",
        model::thousands(report.skipped)
    );
    println!("安全拦截   : {} 条", model::thousands(report.blocked));
    println!("耗时       : {} ms", report.elapsed_ms);

    if !report.failures.is_empty() {
        println!("\n被拦截明细（前 10 条）：");
        for line in report.failures.iter().take(10) {
            println!("  {line}");
        }
    }

    if report.blocked == 0 {
        println!("\n✅ 安全拦截为 0，白名单与保护名单配置正常。");
    } else {
        println!(
            "\n⚠️ 出现 {} 条安全拦截 —— 请检查清理目标的路径配置！",
            report.blocked
        );
    }
    if let Some(log) = &report.log_path {
        println!("日志已写入：{}", log.display());
    }
}

// ---------------------------------------------------------------------------
// 程序图标
// ---------------------------------------------------------------------------

/// 应用图标文件（`assets/app_icon.ico`）。
///
/// 用 `include_bytes!` 在编译期读进来，运行时就不需要额外携带 assets 目录，
/// 单文件分发也不会丢图标。
const APP_ICON_BYTES: &[u8] = include_bytes!("../assets/app_icon.ico");

/// 窗口 / 任务栏图标：优先用 `assets/app_icon.ico`，解码失败才退回程序内绘制的图标。
///
/// 同一个 `.ico` 里可能塞了多个尺寸，`image` 的 ICO 解码器会自动挑
/// 「位深最高、尺寸最大」的那一条，所以拿到的一定是最清晰的一张。
fn app_icon() -> egui::IconData {
    match decode_ico(APP_ICON_BYTES) {
        Ok(icon) => icon,
        Err(err) => {
            eprintln!("assets/app_icon.ico 解码失败（{err}），改用内置绘制的图标");
            make_icon()
        }
    }
}

/// 把 `.ico` 的字节解码成 egui 需要的 RGBA 数据。
fn decode_ico(bytes: &[u8]) -> Result<egui::IconData, image::ImageError> {
    let rgba = image::load_from_memory_with_format(bytes, image::ImageFormat::Ico)?.into_rgba8();
    let (width, height) = rgba.dimensions();
    Ok(egui::IconData {
        rgba: rgba.into_raw(),
        width,
        height,
    })
}

/// 兜底图标：运行时用像素直接绘制，不依赖任何外部资源文件。
fn make_icon() -> egui::IconData {
    const ICON_SIZE: u32 = 64;
    let size = ICON_SIZE;
    let mut rgba = vec![0u8; (size * size * 4) as usize];

    // 主题蓝
    let accent = (0x25u8, 0x63u8, 0xEBu8);
    let white = (0xFFu8, 0xFFu8, 0xFFu8);

    let center = size as f32 / 2.0;

    for y in 0..size {
        for x in 0..size {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;

            let mut color = (0u8, 0u8, 0u8, 0u8);

            if in_rounded_square(px, py, center, center - 1.0, 11.0) {
                color = (accent.0, accent.1, accent.2, 255);

                let dx = px - center;
                let dy = py - center;
                let dist = (dx * dx + dy * dy).sqrt();

                // 圆环
                let on_ring = (dist - 13.5).abs() <= 2.3;
                // 右侧缺口，形成 "C" 的样子
                let in_gap = dx > 3.2 && dy.abs() < 4.4;
                // 中心圆点
                let is_dot = dist <= 2.6;

                if (on_ring && !in_gap) || is_dot {
                    color = (white.0, white.1, white.2, 255);
                }
            }

            let idx = ((y * size + x) * 4) as usize;
            rgba[idx] = color.0;
            rgba[idx + 1] = color.1;
            rgba[idx + 2] = color.2;
            rgba[idx + 3] = color.3;
        }
    }

    egui::IconData {
        rgba,
        width: size,
        height: size,
    }
}

/// 判断点是否落在（圆角）正方形内。
fn in_rounded_square(x: f32, y: f32, center: f32, half: f32, radius: f32) -> bool {
    let dx = (x - center).abs();
    let dy = (y - center).abs();
    if dx > half || dy > half {
        return false;
    }
    let ex = dx - (half - radius);
    let ey = dy - (half - radius);
    if ex > 0.0 && ey > 0.0 {
        return (ex * ex + ey * ey).sqrt() <= radius;
    }
    true
}

/// `clean_c --test-trash`：验证「移入回收站」这条通道在当前系统上是否可用。
///
/// 只会处理本工具自己创建的临时文件，不会碰任何已有文件。
fn test_trash() {
    println!("=== {}通道检测 ===\n", recycle::trash_name());

    let probe = std::env::temp_dir().join(format!("cleanc-selftest-{}.tmp", std::process::id()));
    if let Err(e) = std::fs::write(&probe, b"cleanc self test") {
        println!("无法创建测试文件 {}：{e}", probe.display());
        return;
    }
    println!("已创建测试文件：{}", probe.display());

    let mut single_ok = false;
    match recycle::recycle_one(&probe) {
        Ok(()) => {
            let gone = !probe.exists();
            single_ok = gone;
            println!("单个文件   : 成功（文件已从原位置消失：{gone}）");
        }
        Err(e) => {
            println!("单个文件   : 失败（{e}）");
            let _ = std::fs::remove_file(&probe);

            println!("\n--- 分项诊断 ---");
            for (label, result) in recycle::diagnose_matrix() {
                match result {
                    Ok(()) => println!("  [成功] {label}"),
                    Err(msg) => println!("  [失败] {label} -> {msg}"),
                }
            }
        }
    }

    // 批量通道就是清理流程真正走的路径，单独验一次
    let batch = batch_probe();
    println!("批量文件   : {batch}");
    let batch_ok = batch.starts_with("成功");

    println!();
    if single_ok && batch_ok {
        println!(
            "✅ 「{}」通道可用，默认删除方式是安全的。",
            recycle::action_label()
        );
        println!(
            "   测试文件名为 cleanc-selftest-*.tmp，可在{}里直接删除它们。",
            recycle::trash_name()
        );
    } else {
        println!(
            "⚠️ 当前系统的{}不可用，建议在「设置」里改用「永久删除」，",
            recycle::trash_name()
        );
        println!("   否则清理时会大量跳过文件。");
    }
}

/// 造一批临时文件，走一遍清理流程里实际使用的批量接口。
fn batch_probe() -> String {
    let dir = std::env::temp_dir().join(format!("cleanc-selftest-batch-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return format!("失败（无法创建测试目录：{e}）");
    }

    const COUNT: usize = 5;
    let mut paths = Vec::with_capacity(COUNT);
    for i in 0..COUNT {
        let path = dir.join(format!("item-{i}.tmp"));
        if std::fs::write(&path, format!("cleanc batch {i}")).is_err() {
            let _ = std::fs::remove_dir_all(&dir);
            return "失败（无法创建测试文件）".to_string();
        }
        paths.push(path);
    }

    let before = paths.iter().filter(|p| p.exists()).count();
    let (ok, failed) = recycle::recycle_stats(&paths);
    let left = paths.iter().filter(|p| p.exists()).count();
    let _ = std::fs::remove_dir_all(&dir);

    if ok == before && left == 0 {
        format!("成功（{COUNT} 个文件全部移入回收站，失败 {failed}）")
    } else {
        format!("失败（提交 {before} 个，成功 {ok} 个，仍存在 {left} 个）")
    }
}

/// `clean_c --bigfiles [MB]`：只读地统计大文件与大目录，并打印处置建议。
///
/// 参数是体积阈值（MB），缺省 500；也支持 `--bigfiles=1024` 这种写法。
fn list_big(thresh_mb: u64) {
    let roots = settings::Settings::default_big_file_roots();
    let min_size = thresh_mb.max(1) * 1024 * 1024;

    println!("=== 大文件 / 大目录统计（体积大于 {thresh_mb} MB）===\n");
    println!("扫描目录：");
    for root in &roots {
        println!("  {}", root.display());
    }
    println!();

    let handle = bigfiles::spawn(bigfiles::BigFileOptions {
        min_size,
        roots,
        want_files: true,
        want_folders: true,
    });

    let (mut files, mut folders) = (Vec::new(), Vec::new());
    let mut totals = bigfiles::AdviceTotals::default();
    let (mut elapsed, mut scanned, mut canceled) = (0u64, 0u64, false);
    while let Ok(msg) = handle.rx.recv() {
        if let bigfiles::BigFileMsg::Finished {
            files: f,
            folders: d,
            totals: t,
            scanned: s,
            elapsed_ms,
            canceled: c,
        } = msg
        {
            files = f;
            folders = d;
            totals = t;
            scanned = s;
            elapsed = elapsed_ms;
            canceled = c;
            break;
        }
    }

    if canceled {
        println!("（扫描被取消，下面是已经拿到的部分结果）\n");
    }

    println!("--- 大文件夹（{} 个）---", folders.len());
    if folders.is_empty() {
        println!("  （无）");
    }
    for (index, folder) in folders.iter().enumerate() {
        let kind = advice::for_dir(&folder.path);
        let tag = if folder.protected {
            "受保护".to_string()
        } else {
            kind.short().to_string()
        };
        println!(
            "  {:>2}) {:>10} · {} · {}",
            index + 1,
            model::human_bytes(folder.size),
            tag,
            folder.path.display()
        );
        println!(
            "      {} · {}",
            kind.label(),
            if folder.protected {
                advice::PROTECTED_ADVICE
            } else {
                kind.advice()
            }
        );
        println!(
            "      {} 个文件，最近修改 {}{}",
            model::thousands(folder.files),
            cleaner::format_time(folder.modified),
            if folder.nested {
                "，已被上面某项包含"
            } else {
                ""
            }
        );
    }

    println!("\n--- 大文件（{} 个）---", files.len());
    if files.is_empty() {
        println!("  （无）");
    }
    for (index, file) in files.iter().enumerate() {
        let kind = advice::for_file(&file.path);
        let tag = if file.protected {
            "受保护".to_string()
        } else {
            kind.short().to_string()
        };
        println!(
            "  {:>2}) {:>10} · {} · {}",
            index + 1,
            model::human_bytes(file.size),
            tag,
            file.path.display()
        );
        println!(
            "      {} · {}",
            kind.label(),
            if file.protected {
                advice::PROTECTED_ADVICE
            } else {
                kind.advice()
            }
        );
    }

    // 与界面一致的「删除建议」分档（扫描层已经做过"每个字节只算一次"的去重）
    fn slot(kind: advice::Kind) -> usize {
        match kind {
            advice::Kind::Cache | advice::Kind::Installer => 0,
            advice::Kind::Media => 1,
            _ => 2,
        }
    }

    let mut bytes = [0u64; 3];
    let mut counts = [0usize; 3];
    for kind in advice::Kind::ALL {
        let (kind_bytes, kind_count) = totals.of(kind);
        let index = slot(kind);
        bytes[index] += kind_bytes;
        counts[index] += kind_count;
    }
    bytes[2] += totals.protected.0;
    counts[2] += totals.protected.1;

    println!("\n--- 删除建议（已去重，每个字节只算一次）---");
    for (label, index) in [("可以先清理", 0usize), ("确认后再删", 1), ("建议保留", 2)]
    {
        println!(
            "  {:<6} {:>2} 项 · {}",
            label,
            counts[index],
            model::human_bytes(bytes[index])
        );
    }

    println!(
        "\n共检查 {} 个文件，耗时 {} ms。本命令完全只读，不会删除任何东西。",
        model::thousands(scanned),
        elapsed
    );
}

/// 解析 `--bigfiles` 后面的可选阈值（MB）。返回 `None` 表示没启用这条命令。
fn bigfiles_threshold() -> Option<u64> {
    let args: Vec<String> = std::env::args().collect();
    let mut found = None;
    for (index, arg) in args.iter().enumerate() {
        if arg == "--bigfiles" {
            found = Some(500);
            // 允许 `--bigfiles 1024`
            if let Some(next) = args.get(index + 1)
                && let Ok(value) = next.parse::<u64>()
            {
                found = Some(value);
            }
        } else if let Some(rest) = arg.strip_prefix("--bigfiles=")
            && let Ok(value) = rest.parse::<u64>()
        {
            found = Some(value);
        }
    }
    found
}

// ---------------------------------------------------------------------------
// 控制台排版小工具
// ---------------------------------------------------------------------------

/// 按**显示宽度**补空格对齐。
///
/// `{:<26}` 是按字符个数补的，而汉字在终端里占两列，于是纯中文的列会明显偏左。
/// 这里把东亚宽字符按 2 列计算，表格就能对齐。
fn pad_display(text: &str, width: usize) -> String {
    let used: usize = text.chars().map(|c| if is_wide(c) { 2 } else { 1 }).sum();
    let mut out = text.to_string();
    if used < width {
        out.push_str(&" ".repeat(width - used));
    }
    out
}

/// 是否是"占两列"的字符（东亚宽字符、全角标点、emoji）。
fn is_wide(c: char) -> bool {
    let cp = c as u32;
    matches!(cp,
        0x1100..=0x115F      // 韩文字母
        | 0x2E80..=0xA4CF    // 中日韩部首、汉字、假名、注音…
        | 0xAC00..=0xD7A3    // 韩文音节
        | 0xF900..=0xFAFF    // 兼容汉字
        | 0xFE30..=0xFE6F    // 中日韩兼容形式
        | 0xFF00..=0xFF60    // 全角 ASCII
        | 0xFFE0..=0xFFE6
        | 0x1F300..=0x1FAFF  // emoji 与符号
    )
}

/// `clean_c --list-apps`：列出已安装软件与占用空间（完全只读）。
fn list_apps() {
    println!("=== 已安装软件 · 占用空间统计 ===\n");

    let handle = apps::spawn();
    let mut list: Vec<apps::InstalledApp> = Vec::new();
    let mut sizes: Vec<Option<u64>> = Vec::new();
    let mut elapsed = 0u64;

    loop {
        match handle.rx.recv() {
            Ok(apps::AppMsg::Found { apps }) => {
                sizes = vec![None; apps.len()];
                list = apps;
                println!("共读到 {} 条记录，开始统计体积…\n", list.len());
            }
            Ok(apps::AppMsg::Size { index, bytes }) => {
                if let Some(slot) = sizes.get_mut(index) {
                    *slot = Some(bytes);
                }
            }
            Ok(apps::AppMsg::Finished { elapsed_ms, .. }) => {
                elapsed = elapsed_ms;
                break;
            }
            Err(_) => break,
        }
    }

    // 把实测值写回，便于按体积排序
    for (index, bytes) in sizes.iter().enumerate() {
        if let (Some(app), Some(bytes)) = (list.get_mut(index), bytes) {
            app.measured_bytes = Some(*bytes);
        }
    }
    list.sort_by(|a, b| b.size_bytes().cmp(&a.size_bytes()));

    println!(
        "{} {:>12} {:>14}  {}",
        pad_display("软件名称", 44),
        "体积",
        "版本",
        "发布者"
    );
    println!("{}", "-".repeat(100));

    let mut total = 0u64;
    for app in &list {
        let size = app.size_bytes().unwrap_or(0);
        total += size;
        let label = if app.size_is_estimated() {
            format!("≈ {}", model::human_bytes(size))
        } else {
            model::human_bytes(size)
        };
        let name = if app.name.chars().count() > 42 {
            let trimmed: String = app.name.chars().take(41).collect();
            format!("{trimmed}…")
        } else {
            app.name.clone()
        };
        println!(
            "{} {:>12} {:>14}  {}",
            pad_display(&name, 44),
            label,
            if app.version.is_empty() {
                "-".to_string()
            } else {
                app.version.clone()
            },
            if app.publisher.is_empty() {
                "-"
            } else {
                &app.publisher
            }
        );
    }

    println!("{}", "-".repeat(100));
    println!(
        "合计 {} 条记录，占用约 {}（耗时 {} ms）",
        list.len(),
        model::human_bytes(total),
        elapsed
    );
    let uninstallable = list.iter().filter(|a| a.can_uninstall()).count();
    println!("其中 {uninstallable} 条有可用的卸载方式，可在界面上卸载。");

    // `--list-apps --cmds`：把每条记录的卸载动作打出来，便于核对安全性
    if std::env::args().any(|a| a == "--cmds") {
        println!("\n---------------- 卸载动作明细 ----------------");
        for app in &list {
            if app.can_uninstall() {
                println!("  {:<38} => {}", app.name, app.uninstall_display());
            } else {
                println!("  {:<38} => （无，不可卸载）", app.name);
            }
        }
    }

    println!("\n本命令完全只读，不会卸载任何软件。");
}
