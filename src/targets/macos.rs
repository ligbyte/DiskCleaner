//! macOS 平台的清理目标白名单。
//!
//! 只收三类位置：
//!
//! 1. `~/Library/Caches`、`~/Library/Logs` 这类**系统与 App 保证可以重建**的目录；
//! 2. `/Library/Caches`、`/var/log` 这类需要 `sudo` 的系统级缓存（默认不勾选）；
//! 3. 废纸篓（走系统接口，不逐文件枚举）。
//!
//! 刻意**不碰**的东西：`~/Library/Application Support`（应用数据）、
//! `~/Library/Preferences`（偏好设置）、钥匙串、邮件 / 信息 / 日历数据库、
//! 以及「文稿 / 桌面 / 下载 / 图片 / 影片 / 音乐」这些用户资料目录。
//! 它们同时也在 [`crate::safety`] 的保护名单里，属于双保险。

use std::path::{Path, PathBuf};

use crate::model::{CleanTarget, Group, NameRule, RiskLevel, Root};
use crate::sysutil;

/// `~/Library/...`
fn lib(sub: &str) -> Option<PathBuf> {
    let base = sysutil::mac_library()?;
    Some(if sub.is_empty() { base } else { base.join(sub) })
}

/// 仅当目录存在时才收进白名单 —— 减少界面上大量「不存在」的空条目。
fn push_if_exists(
    list: &mut Vec<CleanTarget>,
    path: Option<PathBuf>,
    make: impl FnOnce(PathBuf) -> CleanTarget,
) {
    if let Some(path) = path
        && path.is_dir()
    {
        list.push(make(path));
    }
}

/// 构建当前机器上可用的清理目标列表。
pub fn build() -> Vec<CleanTarget> {
    let mut list = Vec::new();
    let caches = sysutil::user_cache_dir();

    // -----------------------------------------------------------------
    // 用户缓存与日志
    // -----------------------------------------------------------------

    if let Some(dir) = &caches {
        list.push(CleanTarget {
            id: "mac-user-cache",
            name: "用户缓存（~/Library/Caches）",
            desc: "macOS 与各类 App 写入的缓存目录。删除后应用会在下次运行时按需重建，通常这是体积最大的一项。",
            group: Group::UserFiles,
            risk: RiskLevel::Safe,
            requires_admin: false,
            default_on: true,
            age_filtered: false,
            // 浏览器缓存单独列出，这里跳过它们避免重复统计
            roots: vec![Root::all(dir.clone()).with_skip_dirs(&BROWSER_CACHE_DIRS)],
            note: Some("请先退出正在使用的应用，否则它们的缓存文件会被占用并自动跳过。"),
        });
    }

    push_if_exists(&mut list, lib("Logs"), |path| CleanTarget {
        id: "mac-user-logs",
        name: "用户日志（~/Library/Logs）",
        desc: "应用与诊断工具写下的日志文件，只用于排障，可以安全删除。",
        group: Group::UserFiles,
        risk: RiskLevel::Safe,
        requires_admin: false,
        default_on: true,
        age_filtered: false,
        roots: vec![Root::all(path)],
        note: None,
    });

    push_if_exists(
        &mut list,
        lib("Application Support/CrashReporter"),
        |path| CleanTarget {
            id: "mac-crash-reports",
            name: "崩溃报告",
            desc: "应用崩溃时生成的诊断报告，通常不需要长期保留。",
            group: Group::UserFiles,
            risk: RiskLevel::Safe,
            requires_admin: false,
            default_on: true,
            age_filtered: false,
            roots: vec![Root::all(path)],
            note: None,
        },
    );

    list.push(CleanTarget {
        id: "recycle-bin",
        name: "废纸篓",
        desc: "清空废纸篓中被删除的文件。",
        group: Group::UserFiles,
        risk: RiskLevel::Caution,
        requires_admin: false,
        default_on: false,
        age_filtered: false,
        roots: vec![Root::recycle_bin()],
        note: Some("清空后文件无法再恢复，默认不勾选。"),
    });

    // -----------------------------------------------------------------
    // 浏览器缓存
    // -----------------------------------------------------------------

    if let Some(dir) = &caches {
        for (id, name, sub, desc) in [
            (
                "safari-cache",
                "Safari 缓存",
                "com.apple.Safari",
                "Safari 的网页缓存与 Service Worker 缓存（不含书签、密码、Cookie）。",
            ),
            (
                "chrome-cache",
                "Google Chrome 缓存",
                "Google/Chrome",
                "Chrome 的网页缓存与 GPU 着色器缓存（不含书签、密码、Cookie）。",
            ),
            (
                "edge-cache",
                "Microsoft Edge 缓存",
                "com.microsoft.edgemac",
                "Edge 的网页缓存与 GPU 着色器缓存。",
            ),
            (
                "firefox-cache",
                "Mozilla Firefox 缓存",
                "Firefox",
                "Firefox 的 cache2 / startupCache 等缓存目录。",
            ),
        ] {
            let path = dir.join(sub);
            if path.is_dir() {
                list.push(CleanTarget {
                    id,
                    name,
                    desc,
                    group: Group::Browsers,
                    risk: RiskLevel::Safe,
                    requires_admin: false,
                    default_on: true,
                    age_filtered: false,
                    roots: vec![Root::all(path)],
                    note: Some("请先退出浏览器，否则缓存文件会被占用而跳过。"),
                });
            }
        }
    }

    // -----------------------------------------------------------------
    // 开发者缓存（体积很大，但删了要重新下依赖，默认关闭）
    // -----------------------------------------------------------------

    let dev_roots: Vec<Root> = [
        "Developer/Xcode/DerivedData",
        "Developer/CoreSimulator/Caches",
    ]
    .iter()
    .filter_map(|sub| lib(sub))
    .filter(|path| path.is_dir())
    .map(Root::all)
    .collect();

    if !dev_roots.is_empty() {
        list.push(CleanTarget {
            id: "mac-dev-cache",
            name: "开发者缓存（Xcode）",
            desc: "Xcode 的构建产物与模拟器缓存，体积常常有几十 GB，删掉后需要重新编译。",
            group: Group::Apps,
            risk: RiskLevel::Caution,
            requires_admin: false,
            default_on: false,
            age_filtered: false,
            roots: dev_roots,
            note: Some("正在构建或正在运行的模拟器会被跳过；下次编译会明显变慢。"),
        });
    }

    // -----------------------------------------------------------------
    // 系统级缓存（需要 sudo，默认关闭）
    // -----------------------------------------------------------------

    push_if_exists(
        &mut list,
        Some(Path::new("/Library/Caches").to_path_buf()),
        |path| CleanTarget {
            id: "mac-system-cache",
            name: "系统缓存（/Library/Caches）",
            desc: "系统级应用与服务的缓存目录，需要 root 权限才能清理。",
            group: Group::SystemFiles,
            risk: RiskLevel::Caution,
            requires_admin: true,
            default_on: false,
            age_filtered: false,
            roots: vec![Root::all(path)],
            note: Some("需要以 sudo 运行本工具；被系统占用的文件会自动跳过。"),
        },
    );

    if Path::new("/var/log").is_dir() {
        list.push(CleanTarget {
            id: "mac-system-logs",
            name: "系统日志（/var/log）",
            desc: "系统与守护进程写下的日志文件，只保留最近 7 天的内容。",
            group: Group::SystemFiles,
            risk: RiskLevel::Caution,
            requires_admin: true,
            default_on: false,
            age_filtered: true,
            roots: vec![Root::all("/var/log")],
            note: Some("排障用的历史日志会被清除；需要 root 权限。"),
        });
    }

    push_if_exists(
        &mut list,
        Some(Path::new("/Library/Logs").to_path_buf()),
        |path| CleanTarget {
            id: "mac-system-logs-library",
            name: "系统日志（/Library/Logs）",
            desc: "安装器与系统服务写下的日志，需要 root 权限。",
            group: Group::SystemFiles,
            risk: RiskLevel::Caution,
            requires_admin: true,
            default_on: false,
            age_filtered: false,
            roots: vec![Root::all(path)],
            note: None,
        },
    );

    // -----------------------------------------------------------------
    // 下载目录里的安装包（macOS 上最常见的一类垃圾）
    // -----------------------------------------------------------------

    if let Some(home) = sysutil::home_dir() {
        let downloads = home.join("Downloads");
        if downloads.is_dir() {
            list.push(CleanTarget {
                id: "mac-installers",
                name: "下载目录里的安装包",
                desc: "Downloads 里已经装完的 .dmg / .pkg / .zip 安装包，以及 .crdownload 未完成下载。",
                group: Group::Apps,
                risk: RiskLevel::Caution,
                requires_admin: false,
                default_on: false,
                age_filtered: true,
                roots: vec![Root::matched(
                    downloads,
                    vec![
                        NameRule::suffix(".dmg"),
                        NameRule::suffix(".pkg"),
                        NameRule::suffix(".crdownload"),
                        NameRule::suffix(".part"),
                    ],
                )],
                note: Some("「下载」是用户资料目录，这里只匹配明确的安装包后缀，且默认只处理 7 天前的文件。"),
            });
        }
    }

    list
}

/// `~/Library/Caches` 下由浏览器缓存条目单独接管的子目录。
const BROWSER_CACHE_DIRS: &[&str] = &[
    "google",
    "com.microsoft.edgemac",
    "firefox",
    "com.apple.safari",
];
