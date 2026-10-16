//! Linux（XDG）平台的清理目标白名单。
//!
//! 只收三类位置：
//!
//! 1. `~/.cache` —— XDG 规范里"随时可以删"的那一类目录（最大的一项）；
//! 2. `/var/cache/*` 下的包管理器缓存与 `/var/log`（需要 `sudo`，默认不勾选）；
//! 3. 回收站（走 XDG Trash 目录，不逐文件枚举）。
//!
//! 刻意**不碰**的东西：`~/.config`（应用配置）、`~/.local/share` 下除回收站以外的内容、
//! `/var/lib`（包管理器数据库、容器数据）、`/etc`、`/usr`，
//! 以及「文档 / 桌面 / 下载 / 图片 / 视频 / 音乐」这些用户资料目录。
//! 它们同时也在 [`crate::safety`] 的保护名单里，属于双保险。

use std::path::{Path, PathBuf};

use crate::model::{CleanTarget, Group, NameRule, RiskLevel, Root};
use crate::sysutil;

/// `~/.cache/...`
fn cache(sub: &str) -> Option<PathBuf> {
    let base = sysutil::user_cache_dir()?;
    Some(if sub.is_empty() { base } else { base.join(sub) })
}

/// 把一条「只有目录存在时才生效」的目标加进列表。
fn add_if_dir(
    list: &mut Vec<CleanTarget>,
    path: impl Into<PathBuf>,
    make: impl FnOnce(PathBuf) -> CleanTarget,
) {
    let path = path.into();
    if path.is_dir() {
        list.push(make(path));
    }
}

/// 构建当前机器上可用的清理目标列表。
pub fn build() -> Vec<CleanTarget> {
    let mut list = Vec::new();

    // -----------------------------------------------------------------
    // 用户缓存
    // -----------------------------------------------------------------

    if let Some(dir) = cache("") {
        list.push(CleanTarget {
            id: "user-cache",
            name: "用户缓存（~/.cache）",
            desc: "XDG 规范里约定的缓存目录。所有程序都可以随时重建它，是 Linux 上最安全、体积最大的一项。",
            group: Group::UserFiles,
            risk: RiskLevel::Safe,
            requires_admin: false,
            default_on: true,
            age_filtered: false,
            // 浏览器缓存与缩略图单独列出，避免重复统计
            roots: vec![Root::all(dir).with_skip_dirs(BROWSER_CACHE_DIRS)],
            note: Some("删掉后首次启动应用会略慢，但不影响任何数据。"),
        });
    }

    list.push(CleanTarget {
        id: "recycle-bin",
        name: "回收站",
        desc: "清空 XDG 回收站（~/.local/share/Trash）里被删除的文件。",
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

    for (id, name, sub, desc) in [
        (
            "chrome-cache",
            "Google Chrome 缓存",
            "google-chrome",
            "Chrome 的网页缓存与 GPU 着色器缓存（不含书签、密码、Cookie）。",
        ),
        (
            "chromium-cache",
            "Chromium 缓存",
            "chromium",
            "Chromium 的网页缓存与 GPU 着色器缓存。",
        ),
        (
            "firefox-cache",
            "Mozilla Firefox 缓存",
            "mozilla",
            "Firefox 的 cache2 / startupCache 等缓存目录。",
        ),
    ] {
        if let Some(path) = cache(sub) {
            add_if_dir(&mut list, path, |path| CleanTarget {
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

    if let Some(path) = cache("thumbnails") {
        add_if_dir(&mut list, path, |path| CleanTarget {
            id: "thumbnail-cache",
            name: "文件管理器缩略图缓存",
            desc: "图片 / 视频缩略图数据库，删除后文件管理器会按需重建。",
            group: Group::Apps,
            risk: RiskLevel::Safe,
            requires_admin: false,
            default_on: true,
            age_filtered: false,
            roots: vec![Root::all(path)],
            note: None,
        });
    }

    // -----------------------------------------------------------------
    // 开发者缓存（体积很大，但删了要重新下依赖，默认关闭）
    // -----------------------------------------------------------------

    if let Some(home) = sysutil::home_dir() {
        let dev_roots: Vec<Root> = [
            ".npm/_cacache",
            ".cargo/registry/cache",
            ".gradle/caches",
            ".m2/repository",
        ]
        .iter()
        .map(|sub| home.join(sub))
        .filter(|path| path.is_dir())
        .map(Root::all)
        .collect();

        if !dev_roots.is_empty() {
            list.push(CleanTarget {
                id: "dev-cache",
                name: "开发工具缓存",
                desc: "npm / cargo / gradle / maven 的下载缓存，体积常常以 GB 计，联网后可以重新拉取。",
                group: Group::Apps,
                risk: RiskLevel::Caution,
                requires_admin: false,
                default_on: false,
                age_filtered: false,
                roots: dev_roots,
                note: Some("删掉后下一次构建需要重新下载依赖，离线环境下请勿清理。"),
            });
        }
    }

    // -----------------------------------------------------------------
    // 临时目录
    // -----------------------------------------------------------------

    for (id, name, path, note) in [
        (
            "tmp",
            "临时目录（/tmp）",
            "/tmp",
            "只处理 /tmp 下的一级文件，且默认只清理 7 天前的；别人创建的会话文件会因权限不足被跳过。",
        ),
        (
            "var-tmp",
            "临时目录（/var/tmp）",
            "/var/tmp",
            "/var/tmp 里的文件按约定允许跨重启保留，删除前请确认没有服务在用。",
        ),
    ] {
        if Path::new(path).is_dir() {
            list.push(CleanTarget {
                id,
                name,
                desc: "系统临时目录，存放程序运行期间的中间文件。",
                group: Group::UserFiles,
                risk: RiskLevel::Caution,
                requires_admin: false,
                default_on: false,
                age_filtered: true,
                roots: vec![Root::shallow_files(path)],
                note: Some(note),
            });
        }
    }

    // -----------------------------------------------------------------
    // 包管理器缓存（需要 sudo，默认关闭）
    // -----------------------------------------------------------------

    if Path::new("/var/cache/apt/archives").is_dir() {
        list.push(CleanTarget {
            id: "apt-cache",
            name: "APT 下载缓存（Debian / Ubuntu）",
            desc: "已经安装完成的 .deb 安装包，删掉后系统照常运行，需要时重新下载即可。",
            group: Group::SystemFiles,
            risk: RiskLevel::Caution,
            requires_admin: true,
            default_on: true,
            age_filtered: false,
            roots: vec![
                Root::matched("/var/cache/apt/archives", vec![NameRule::suffix(".deb")])
                    .with_skip_dirs(&["partial"]),
            ],
            note: Some("需要 root 权限；正在下载中的 partial 目录会原样保留。"),
        });
    }

    for (id, name, path, desc) in [
        (
            "pacman-cache",
            "Pacman 下载缓存（Arch）",
            "/var/cache/pacman/pkg",
            "已经安装完成的软件包文件，删掉后 pacman 会按需重新下载。",
        ),
        (
            "dnf-cache",
            "DNF 缓存（Fedora / RHEL）",
            "/var/cache/dnf",
            "DNF 的仓库元数据与下载的 RPM 包。",
        ),
        (
            "zypp-cache",
            "Zypper 缓存（openSUSE）",
            "/var/cache/zypp",
            "Zypper 的仓库元数据与下载的 RPM 包。",
        ),
    ] {
        if Path::new(path).is_dir() {
            list.push(CleanTarget {
                id,
                name,
                desc,
                group: Group::SystemFiles,
                risk: RiskLevel::Caution,
                requires_admin: true,
                default_on: true,
                age_filtered: false,
                roots: vec![Root::all(path).with_skip_dirs(&["sync", "partial"])],
                note: Some("需要 root 权限。"),
            });
        }
    }

    // -----------------------------------------------------------------
    // 系统日志（需要 sudo，默认关闭）
    // -----------------------------------------------------------------

    if Path::new("/var/log").is_dir() {
        list.push(CleanTarget {
            id: "system-logs",
            name: "系统日志（/var/log）",
            desc: "已轮转的日志文件（*.log / *.gz / *.old），默认只处理 7 天前的。",
            group: Group::SystemFiles,
            risk: RiskLevel::Caution,
            requires_admin: true,
            default_on: false,
            age_filtered: true,
            roots: vec![
                Root::matched(
                    "/var/log",
                    vec![
                        NameRule::suffix(".log"),
                        NameRule::suffix(".gz"),
                        NameRule::suffix(".old"),
                    ],
                )
                // systemd 的 journal 目录要用 `journalctl --vacuum` 处理，这里不碰
                .with_skip_dirs(&["journal", "private", "samba"]),
            ],
            note: Some(
                "需要 root 权限。systemd 日志请另行执行 `sudo journalctl --vacuum-time=7d`。",
            ),
        });
    }

    list
}

/// `~/.cache` 下由浏览器 / 缩略图条目单独接管的子目录。
const BROWSER_CACHE_DIRS: &[&str] = &["google-chrome", "chromium", "mozilla", "thumbnails"];
