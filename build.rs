//! 构建脚本：把 `assets/app_icon.ico` 编译进 Windows 可执行文件的资源段。
//!
//! 嵌进去之后，资源管理器、任务栏、Alt+Tab 里显示的都是应用自己的图标，
//! 而不是 rustc 默认生成的空白 exe 图标（那个图标在窗口创建之前就已经显示了）。
//!
//! 两点刻意的设计：
//! * **只在 Windows 目标上做事**，其它平台直接返回；
//! * **找不到资源编译器不会让构建失败** —— 只打一条 `cargo:warning` 然后跳过。
//!   窗口图标走的是运行时加载（见 `src/main.rs`），与这里无关，
//!   所以即使这一步被跳过，应用依然有正确的图标。

use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=assets/app_icon.ico");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let icon = manifest.join("assets").join("app_icon.ico");
    if !icon.is_file() {
        println!(
            "cargo:warning=未找到 {}，跳过 exe 图标嵌入（窗口图标不受影响）",
            icon.display()
        );
        return;
    }

    // rc.exe 的字符串字面量里 '\' 是转义符，统一换成 '/' 更稳妥
    let icon_literal = icon.to_string_lossy().replace('\\', "/");

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let rc_path = out_dir.join("clean_c_icon.rc");
    let res_path = out_dir.join("clean_c_icon.res");

    // 资源 ID 用 1：Windows 取 ID 最小的图标组作为「应用程序图标」
    if let Err(err) = std::fs::write(&rc_path, format!("1 ICON \"{icon_literal}\"\n")) {
        println!("cargo:warning=无法写入 .rc 文件（{err}），跳过 exe 图标嵌入");
        return;
    }

    let Some(rc_exe) = find_rc() else {
        println!("cargo:warning=未找到 rc.exe，跳过 exe 图标嵌入（窗口图标不受影响）");
        return;
    };

    let built = Command::new(&rc_exe)
        .arg("/nologo")
        .arg("/fo")
        .arg(&res_path)
        .arg(&rc_path)
        .output();

    match built {
        Ok(out) if out.status.success() && res_path.is_file() => {
            // MSVC 的 link.exe 可以直接把 .res 当作输入文件
            println!("cargo:rustc-link-arg={}", res_path.display());
        }
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            println!(
                "cargo:warning=rc.exe 执行失败，跳过 exe 图标嵌入：{}",
                stderr.trim()
            );
        }
        Err(err) => println!("cargo:warning=无法启动 rc.exe（{err}），跳过 exe 图标嵌入"),
    }
}

/// 找一个可用的资源编译器：先看环境变量，再翻 Windows SDK，最后交给 PATH。
fn find_rc() -> Option<PathBuf> {
    // 1) rustc / cargo 生态里常见的覆盖方式
    for key in ["RC", "RC_x86_64_pc_windows_msvc", "RCC"] {
        if let Ok(value) = std::env::var(key) {
            let path = PathBuf::from(value);
            if path.is_file() {
                return Some(path);
            }
        }
    }

    // 2) Windows Kits\10\bin\<版本>\x64\rc.exe —— 取版本号最高的那个
    let mut sdk_candidates: Vec<(u64, PathBuf)> = Vec::new();
    for root in [
        r"C:\Program Files (x86)\Windows Kits\10\bin",
        r"C:\Program Files\Windows Kits\10\bin",
    ] {
        let root = Path::new(root);
        let Ok(entries) = std::fs::read_dir(root) else {
            continue;
        };
        for entry in entries.flatten() {
            let dir = entry.path();
            for arch in ["x64", "x86", "arm64"] {
                let candidate = dir.join(arch).join("rc.exe");
                if candidate.is_file() {
                    let rank = version_rank(&entry.file_name().to_string_lossy());
                    sdk_candidates.push((rank, candidate));
                }
            }
        }
    }
    sdk_candidates.sort_by(|a, b| b.0.cmp(&a.0));
    if let Some((_, path)) = sdk_candidates.into_iter().next() {
        return Some(path);
    }

    // 3) 交给 PATH —— 找不到时 Command 会返回 Err，调用方已经处理了
    Some(PathBuf::from("rc.exe"))
}

/// 把 `10.0.26100.0` 这类版本号压成一个可比较的整数。
fn version_rank(name: &str) -> u64 {
    name.split('.')
        .filter_map(|part| part.parse::<u64>().ok())
        .fold(0u64, |acc, part| acc * 100_000 + part.min(99_999))
}
