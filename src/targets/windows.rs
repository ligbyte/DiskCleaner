//! Windows 平台的清理目标白名单。
//!
//! **这是整个工具唯一允许删除文件的地方。** 所有路径都是硬编码的已知安全位置，
//! 不做任何用户输入的路径拼接，也不扫描用户文档目录。
//! 每个目标的 `risk` / `default_on` 共同决定了它在界面上的默认勾选状态。

use std::path::PathBuf;

use crate::model::{CleanTarget, Group, NameRule, RiskLevel, Root, browser_cache_rules};
use crate::sysutil;

fn win(sub: &str) -> PathBuf {
    sysutil::windows_dir().join(sub)
}

fn local(sub: &str) -> Option<PathBuf> {
    sysutil::local_app_data().map(|base| base.join(sub))
}

fn roaming(sub: &str) -> Option<PathBuf> {
    sysutil::roaming_app_data().map(|base| base.join(sub))
}

/// 构建当前机器上可用的清理目标列表。
pub fn build() -> Vec<CleanTarget> {
    let mut list = Vec::new();

    // -----------------------------------------------------------------
    // 用户临时文件
    // -----------------------------------------------------------------

    if let Some(temp) = sysutil::temp_dir() {
        list.push(CleanTarget {
            id: "user-temp",
            name: "用户临时文件",
            desc: "存放于 %TEMP% 的安装残留、程序运行中间文件。删不掉的文件会自动跳过。",
            group: Group::UserFiles,
            risk: RiskLevel::Safe,
            requires_admin: false,
            default_on: true,
            age_filtered: true,
            roots: vec![Root::all(temp).with_skip_dirs(&["cleanc"])],
            note: Some("默认只清理 7 天前的文件，可在「设置」里调整。"),
        });
    }

    list.push(CleanTarget {
        id: "windows-temp",
        name: "系统临时文件",
        desc: "系统级临时目录 C:\\Windows\\Temp，通常由安装程序与系统服务写入。",
        group: Group::UserFiles,
        risk: RiskLevel::Caution,
        requires_admin: true,
        default_on: true,
        age_filtered: true,
        roots: vec![Root::all(win("Temp"))],
        note: None,
    });

    if let Some(p) = local(r"Microsoft\Windows\INetCache") {
        list.push(CleanTarget {
            id: "inet-cache",
            name: "系统网络缓存",
            desc: "Internet 临时文件（INetCache），由 IE 内核 / 各类下载器共享的网页缓存。",
            group: Group::UserFiles,
            risk: RiskLevel::Safe,
            requires_admin: false,
            default_on: true,
            age_filtered: false,
            roots: vec![Root::all(p)],
            note: None,
        });
    }

    if let Some(p) = local(r"Microsoft\Windows\WER") {
        list.push(CleanTarget {
            id: "wer-user",
            name: "程序错误报告（用户）",
            desc: "程序崩溃时生成的错误报告与转储队列，仅用于回传诊断，可安全删除。",
            group: Group::UserFiles,
            risk: RiskLevel::Safe,
            requires_admin: false,
            default_on: true,
            age_filtered: false,
            roots: vec![Root::all(p)],
            note: None,
        });
    }

    if let Some(p) = local("CrashDumps") {
        list.push(CleanTarget {
            id: "crash-dumps",
            name: "应用崩溃转储",
            desc: "程序崩溃时写下的内存转储文件，单个可达数百 MB。",
            group: Group::UserFiles,
            risk: RiskLevel::Safe,
            requires_admin: false,
            default_on: true,
            age_filtered: false,
            roots: vec![Root::all(p)],
            note: None,
        });
    }

    if let Some(p) = local(r"Microsoft\Windows\Explorer") {
        list.push(CleanTarget {
            id: "thumbnail-cache",
            name: "资源管理器缩略图缓存",
            desc: "图片 / 视频缩略图数据库（thumbcache_*.db），删除后资源管理器会按需重建。",
            group: Group::UserFiles,
            risk: RiskLevel::Safe,
            requires_admin: false,
            default_on: true,
            age_filtered: false,
            roots: vec![Root::matched(p, vec![NameRule::prefix("thumbcache")])],
            note: Some("正被资源管理器占用的文件会自动跳过，属正常现象。"),
        });
    }

    if let Some(p) = local("") {
        list.push(CleanTarget {
            id: "icon-cache",
            name: "图标缓存",
            desc: "资源管理器图标数据库（iconcache_*.db），删除后会自动重建。",
            group: Group::UserFiles,
            risk: RiskLevel::Safe,
            requires_admin: false,
            default_on: false,
            age_filtered: false,
            roots: vec![Root::matched(p, vec![NameRule::prefix("iconcache")])],
            note: Some("重建期间桌面图标可能短暂闪烁。"),
        });
    }

    if let Some(p) = local("Packages") {
        list.push(CleanTarget {
            id: "uwp-temp-state",
            name: "应用商店应用临时状态",
            desc: "UWP / 商店应用的 TempState 目录，仅存放临时数据。",
            group: Group::UserFiles,
            risk: RiskLevel::Safe,
            requires_admin: false,
            default_on: false,
            age_filtered: false,
            roots: vec![Root::matched(p, vec![NameRule::exact("tempstate")])],
            note: Some("需要遍历 Packages 目录，扫描耗时较长。"),
        });
    }

    list.push(CleanTarget {
        id: "recycle-bin",
        name: "回收站",
        desc: "清空回收站中被删除的文件。",
        group: Group::UserFiles,
        risk: RiskLevel::Caution,
        requires_admin: false,
        default_on: false,
        age_filtered: false,
        roots: vec![Root::recycle_bin()],
        note: Some("清空后文件无法再恢复，默认不勾选。"),
    });

    // -----------------------------------------------------------------
    // 系统文件与更新
    // -----------------------------------------------------------------

    list.push(CleanTarget {
        id: "windows-update-cache",
        name: "Windows 更新下载缓存",
        desc: "已安装更新残留的安装包，通常体积最大的一项，可安全删除。",
        group: Group::SystemFiles,
        risk: RiskLevel::Caution,
        requires_admin: true,
        default_on: true,
        age_filtered: true,
        roots: vec![Root::all(win(r"SoftwareDistribution\Download"))],
        note: Some("若更新服务正在下载，相关文件会被占用并自动跳过。"),
    });

    list.push(CleanTarget {
        id: "delivery-optimization",
        name: "传递优化缓存",
        desc: "Windows 用于在局域网 / 互联网共享更新片段的缓存文件。",
        group: Group::SystemFiles,
        risk: RiskLevel::Caution,
        requires_admin: true,
        default_on: true,
        age_filtered: false,
        roots: vec![Root::all(win(
            r"ServiceProfiles\NetworkService\AppData\Local\Microsoft\Windows\DeliveryOptimization",
        ))],
        note: None,
    });

    list.push(CleanTarget {
        id: "windows-logs",
        name: "系统日志缓存",
        desc: "Windows 组件与更新留下的日志文件（CBS / DISM / WindowsUpdate 等）。",
        group: Group::SystemFiles,
        risk: RiskLevel::Caution,
        requires_admin: true,
        default_on: true,
        age_filtered: false,
        roots: vec![Root::all(win("Logs"))],
        note: Some("排障用的历史日志会被清除。"),
    });

    list.push(CleanTarget {
        id: "wer-system",
        name: "错误报告（系统）",
        desc: "系统级 Windows 错误报告队列与归档。",
        group: Group::SystemFiles,
        risk: RiskLevel::Caution,
        requires_admin: true,
        default_on: true,
        age_filtered: false,
        roots: vec![
            Root::all(sysutil::program_data().join(r"Microsoft\Windows\WER")),
            Root::all(win(
                r"ServiceProfiles\LocalService\AppData\Local\Microsoft\Windows\WER",
            )),
        ],
        note: None,
    });

    list.push(CleanTarget {
        id: "crash-dumps-system",
        name: "系统崩溃转储",
        desc: "系统蓝屏时生成的内存转储文件，单个可达数 GB。",
        group: Group::SystemFiles,
        risk: RiskLevel::Caution,
        requires_admin: true,
        default_on: true,
        age_filtered: false,
        roots: vec![Root::all(win("Minidump")), Root::file(win("MEMORY.DMP"))],
        note: Some("删除后无法再用于分析蓝屏原因。"),
    });

    list.push(CleanTarget {
        id: "prefetch",
        name: "预读取文件（Prefetch）",
        desc: "Windows 用来加速程序启动的预读取数据，删除后会自动重建。",
        group: Group::SystemFiles,
        risk: RiskLevel::Caution,
        requires_admin: true,
        default_on: false,
        age_filtered: false,
        roots: vec![Root::shallow_files(win("Prefetch"))],
        note: Some("删除后程序首次启动会略慢，默认不勾选。"),
    });

    list.push(CleanTarget {
        id: "font-cache",
        name: "字体缓存",
        desc: "系统字体缓存文件。",
        group: Group::SystemFiles,
        risk: RiskLevel::Caution,
        requires_admin: true,
        default_on: false,
        age_filtered: false,
        roots: vec![Root::all(win(
            r"ServiceProfiles\LocalService\AppData\Local\FontCache",
        ))],
        note: Some("缓存被系统服务占用时会被跳过；建议重启后再清理。"),
    });

    // -----------------------------------------------------------------
    // 浏览器缓存
    // -----------------------------------------------------------------

    if let Some(p) = local(r"Google\Chrome\User Data") {
        list.push(CleanTarget {
            id: "chrome-cache",
            name: "Google Chrome 缓存",
            desc: "网页缓存、GPU 着色器缓存与 Service Worker 缓存（不含书签、密码、Cookie）。",
            group: Group::Browsers,
            risk: RiskLevel::Safe,
            requires_admin: false,
            default_on: true,
            age_filtered: false,
            roots: vec![Root::matched(p, browser_cache_rules()).with_skip_dirs(&[
                "system profile",
                "guest profile",
                "snapshots",
            ])],
            note: Some("请先关闭浏览器，否则缓存文件会被占用而跳过。"),
        });
    }

    if let Some(p) = local(r"Microsoft\Edge\User Data") {
        list.push(CleanTarget {
            id: "edge-cache",
            name: "Microsoft Edge 缓存",
            desc: "网页缓存、GPU 着色器缓存与 Service Worker 缓存。",
            group: Group::Browsers,
            risk: RiskLevel::Safe,
            requires_admin: false,
            default_on: true,
            age_filtered: false,
            roots: vec![Root::matched(p, browser_cache_rules()).with_skip_dirs(&[
                "system profile",
                "guest profile",
                "snapshots",
            ])],
            note: Some("请先关闭浏览器，否则缓存文件会被占用而跳过。"),
        });
    }

    if let Some(p) = local(r"Mozilla\Firefox\Profiles") {
        list.push(CleanTarget {
            id: "firefox-cache",
            name: "Mozilla Firefox 缓存",
            desc: "Firefox 的 cache2 / startupCache 等缓存目录。",
            group: Group::Browsers,
            risk: RiskLevel::Safe,
            requires_admin: false,
            default_on: true,
            age_filtered: false,
            roots: vec![Root::matched(
                p,
                vec![
                    NameRule::exact("cache2"),
                    NameRule::exact("startupcache"),
                    NameRule::exact("shader-cache"),
                    NameRule::exact("thumbnails"),
                ],
            )],
            note: None,
        });
    }

    if let Some(p) = local(r"Microsoft\Windows\INetCookies") {
        // 仅清缓存目录，不碰 Cookie 本体
        list.push(CleanTarget {
            id: "inet-cookies-cache",
            name: "IE 内核 Cookie 缓存",
            desc: "旧版 IE 内核留下的 Cookie 缓存目录（不影响现代浏览器登录状态）。",
            group: Group::Browsers,
            risk: RiskLevel::Caution,
            requires_admin: false,
            default_on: false,
            age_filtered: false,
            roots: vec![Root::all(p)],
            note: Some("可能会让依赖 IE 内核的旧程序需要重新登录。"),
        });
    }

    // -----------------------------------------------------------------
    // 应用 / 显卡缓存
    // -----------------------------------------------------------------

    if let Some(base) = local("NVIDIA") {
        list.push(CleanTarget {
            id: "nvidia-cache",
            name: "NVIDIA 着色器缓存",
            desc: "显卡驱动的 DX / GL / 计算着色器缓存，删除后游戏首次加载会重新编译。",
            group: Group::Apps,
            risk: RiskLevel::Safe,
            requires_admin: false,
            default_on: true,
            age_filtered: false,
            roots: vec![
                Root::all(base.join("DXCache")),
                Root::all(base.join("GLCache")),
                Root::all(base.join("ComputeCache")),
                Root::all(base.join("NvBackend")),
            ],
            note: None,
        });
    }

    if let Some(base) = local("AMD") {
        list.push(CleanTarget {
            id: "amd-cache",
            name: "AMD 着色器缓存",
            desc: "AMD 显卡驱动的着色器缓存。",
            group: Group::Apps,
            risk: RiskLevel::Safe,
            requires_admin: false,
            default_on: true,
            age_filtered: false,
            roots: vec![
                Root::all(base.join("DxCache")),
                Root::all(base.join("DxcCache")),
                Root::all(base.join("GLCache")),
                Root::all(base.join("VkCache")),
            ],
            note: None,
        });
    }

    if let Some(base) = local("Intel") {
        list.push(CleanTarget {
            id: "intel-cache",
            name: "Intel 显卡缓存",
            desc: "Intel 核显 / 独显驱动的着色器缓存。",
            group: Group::Apps,
            risk: RiskLevel::Safe,
            requires_admin: false,
            default_on: true,
            age_filtered: false,
            roots: vec![
                Root::all(base.join("ShaderCache")),
                Root::all(base.join(r"Graphics\ShaderCache")),
            ],
            note: None,
        });
    }

    if let Some(p) = local("D3DSCache") {
        list.push(CleanTarget {
            id: "d3d-cache",
            name: "DirectX 着色器缓存",
            desc: "系统级 D3D 着色器缓存，由游戏与图形程序写入。",
            group: Group::Apps,
            risk: RiskLevel::Safe,
            requires_admin: false,
            default_on: true,
            age_filtered: false,
            roots: vec![Root::all(p)],
            note: None,
        });
    }

    if let Some(p) = roaming(r"Microsoft\Windows\Recent") {
        list.push(CleanTarget {
            id: "recent-shortcuts",
            name: "最近使用记录（快捷方式）",
            desc: "「最近使用的文件」列表里的快捷方式，体积很小，主要影响隐私。",
            group: Group::Apps,
            risk: RiskLevel::Caution,
            requires_admin: false,
            default_on: false,
            age_filtered: false,
            roots: vec![Root::all(p)],
            note: Some("清理后「最近使用」列表会清空。"),
        });
    }

    // -----------------------------------------------------------------
    // 高级项（默认全部关闭）
    // -----------------------------------------------------------------

    list.push(CleanTarget {
        id: "windows-old",
        name: "旧版 Windows 安装（C:\\Windows.old）",
        desc: "系统升级前的完整备份，通常占用 10~30 GB。",
        group: Group::Advanced,
        risk: RiskLevel::Advanced,
        requires_admin: true,
        default_on: false,
        age_filtered: false,
        roots: vec![Root::all(PathBuf::from(r"C:\Windows.old"))],
        note: Some("删除后无法再回滚到升级前的系统版本；部分文件归 TrustedInstaller 所有，可能无法完整删除。"),
    });

    list.push(CleanTarget {
        id: "winre-agent",
        name: "更新还原点残留（$WinREAgent）",
        desc: "Windows 功能更新过程中产生的临时还原数据。",
        group: Group::Advanced,
        risk: RiskLevel::Advanced,
        requires_admin: true,
        default_on: false,
        age_filtered: false,
        roots: vec![Root::all(PathBuf::from(r"C:\$WinREAgent"))],
        note: Some("若系统更新未完成，删除会失去回滚能力。"),
    });

    list
}
