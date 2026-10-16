//! 安全说明页：把"为什么敢删"讲清楚。
//!
//! 页面上的例子、保护名单、注意事项全部按平台生成 —— 同一套机制在
//! Windows / macOS / Linux 上拦的是不同的路径，说明文字也必须跟着换，
//! 否则macOS 用户会读到一堆 `C:\Windows\System32`。

use eframe::egui::{self, RichText};

use crate::app::App;
use crate::theme::{Palette, PillKind};
use crate::ui::widgets::{self, bullet, card, pill};

struct Item {
    title: &'static str,
    body: String,
    level: PillKind,
}

impl Item {
    fn new(title: &'static str, body: impl Into<String>, level: PillKind) -> Self {
        Self {
            title,
            body: body.into(),
            level,
        }
    }
}

fn level_label(level: PillKind) -> &'static str {
    match level {
        PillKind::Safe => "强保证",
        PillKind::Caution => "需留意",
        PillKind::Accent => "机制",
        PillKind::Danger => "风险",
        PillKind::Neutral => "说明",
    }
}

/// 平台相关的小片段，拼句子时反复要用。
fn env_temp() -> &'static str {
    if cfg!(windows) { "%TEMP%" } else { "$TMPDIR" }
}

fn app_data_hint() -> String {
    crate::sysutil::app_data_dir().to_string_lossy().to_string()
}

/// 具体条款。每个平台的举例、目录名、关键文件名都不一样，所以整段按平台生成。
fn items() -> Vec<Item> {
    let mut out = Vec::new();

    out.push(Item::new(
        "白名单：只有写死的路径才会被清理",
        "所有清理目标都在代码里硬编码，全部是操作系统、浏览器、显卡驱动已知的缓存与临时目录。程序不支持任何形式的自定义清理路径，也不做路径字符串拼接，因此不存在「配置写错就误删」的可能。",
        PillKind::Safe,
    ));

    // 三重校验里的举例按平台换
    #[cfg(windows)]
    let protect_examples =
        "受保护目录树（System32、WinSxS、Program Files、ProgramData 关键子目录等）";
    #[cfg(target_os = "macos")]
    let protect_examples = "受保护目录树（/System、/Library、/usr、/etc、/private/var/db 等）";
    #[cfg(target_os = "linux")]
    let protect_examples = "受保护目录树（/usr、/etc、/boot、/var/lib、/proc、/sys 等）";
    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
    let protect_examples = "受保护目录树";

    #[cfg(windows)]
    let key_files = "desktop.ini、pagefile.sys、hiberfil.sys、ntuser.dat 等";
    #[cfg(not(windows))]
    let key_files =
        "fstab、passwd、shadow、sudoers、authorized_keys、id_rsa、id_ed25519、known_hosts 等";

    out.push(Item::new(
        "默认拒绝：删除前必须通过三重校验",
        format!(
            "任何一次删除，都要依次通过：① 条目必须位于本类别声明的根目录之内；② 不得命中{protect_examples}；③ 不得是受保护的关键文件（{key_files}）。任一不通过就拦截并记录日志。"
        ),
        PillKind::Safe,
    ));

    out.push(Item::new(
        "不跟随符号链接与目录联接",
        "遇到符号链接、目录联接、挂载点一律跳过，既不删除链接本身，也绝不顺着链接删到别处去。这堵住了「通过一个链接把删除操作引到别处」的路径。",
        PillKind::Safe,
    ));

    out.push(Item::new(
        "只清内容，不删根目录",
        "所有清理都是「清空目录里的东西」，清理目标本身的目录一定保留。对浏览器缓存目录，连缓存文件夹本身也会保留，避免应用因为找不到缓存路径而报错。",
        PillKind::Safe,
    ));

    #[cfg(windows)]
    let personal = "文档、桌面、下载、图片、视频、音乐、OneDrive、联系人、收藏夹等目录";
    #[cfg(target_os = "macos")]
    let personal =
        "文稿、桌面、下载、图片、影片、音乐，以及 ~/Library 下的邮件、信息、Safari、钥匙串等目录";
    #[cfg(target_os = "linux")]
    let personal =
        "文档、桌面、下载、图片、视频、音乐、模板，以及 ~/.ssh、~/.gnupg、~/.config 等目录";

    out.push(Item::new(
        "用户资料目录完全不参与",
        format!(
            "{personal}不在扫描范围，同时也在保护名单内。程序不会读取这些目录里的任何文件内容。"
        ),
        PillKind::Safe,
    ));

    out.push(Item::new(
        "年龄过滤：临时文件默认只动 7 天前的",
        format!(
            "对 {temp} 这类「可能正在被程序使用」的目录，默认只清理修改时间早于 7 天的文件。断网重连、安装中途重启留下的文件基本都符合条件，而正在运行的安装程序的文件会被自然排除。阈值可以在设置里改成 1 天、3 天、30 天或全部。",
            temp = env_temp()
        ),
        PillKind::Caution,
    ));

    out.push(Item::new(
        "占用与权限问题只会跳过，不会中断",
        "被其他程序占用的文件、没有权限的文件，都会被安静地跳过并计入「跳过」统计，不会导致清理失败，也不会重试、不会强制解锁句柄。",
        PillKind::Caution,
    ));

    out.push(Item::new(
        "默认移入回收站，永久删除需二次确认",
        format!(
            "默认的删除方式是「{}」，误删可以直接还原。切到「永久删除」后，确认弹窗会以红色警示，并且明确写出「删除后无法恢复」。执行前一定会弹出确认对话框，列出将要处理的类别与总体积。",
            crate::recycle::action_label()
        ),
        PillKind::Caution,
    ));

    out.push(Item::new(
        "完整日志 + 演练模式",
        "每次清理都会在本地写一份日志，逐条记录删除了什么、跳过了什么、拦截了什么。另外提供「演练模式」，只统计体积不真正删除，适合第一次使用前先看一遍效果。",
        PillKind::Accent,
    ));

    out.push(Item::new(
        "纯本地运行，不联网、不上传",
        format!(
            "程序没有任何网络请求，不收集、不上传任何文件信息或使用数据。配置文件与日志都保存在本机 {dir} 下。",
            dir = app_data_hint()
        ),
        PillKind::Accent,
    ));

    out
}

/// 保护名单分组的展示数据（只用于"讲明白拦了哪些地方"，不是判定依据）。
fn protected_groups() -> Vec<(&'static str, Vec<String>)> {
    let home = || {
        crate::sysutil::home_dir()
            .map(|h| h.to_string_lossy().to_string())
            .unwrap_or_else(|| {
                if cfg!(windows) {
                    "C:\\Users\\<你>".to_string()
                } else {
                    "<你的主目录>".to_string()
                }
            })
    };

    #[cfg(windows)]
    {
        vec![
            (
                "Windows 核心",
                vec![
                    r"C:\Windows\System32",
                    r"C:\Windows\SysWOW64",
                    r"C:\Windows\WinSxS",
                    r"C:\Windows\assembly",
                    r"C:\Windows\servicing",
                    r"C:\Windows\Boot",
                    r"C:\Windows\SystemApps",
                    r"C:\Windows\SystemResources",
                    r"C:\Windows\Security",
                    r"C:\Windows\Fonts",
                    r"C:\Windows\inf",
                    r"C:\Windows\IME",
                ]
                .into_iter()
                .map(String::from)
                .collect(),
            ),
            (
                "已安装程序",
                vec![r"C:\Program Files", r"C:\Program Files (x86)"]
                    .into_iter()
                    .map(String::from)
                    .collect(),
            ),
            (
                "系统维护数据",
                vec![
                    r"C:\$Recycle.Bin（仅通过系统 API 清空）",
                    r"C:\System Volume Information",
                    r"C:\Recovery",
                    r"C:\ProgramData\Package Cache",
                    r"C:\ProgramData\Microsoft\Crypto",
                ]
                .into_iter()
                .map(String::from)
                .collect(),
            ),
            (
                "用户资料目录",
                vec![
                    format!(r"{}\Documents", home()),
                    format!(r"{}\Desktop", home()),
                    format!(r"{}\Downloads", home()),
                    format!(r"{}\Pictures", home()),
                    format!(r"{}\Videos", home()),
                    format!(r"{}\Music", home()),
                    format!(r"{}\OneDrive", home()),
                    format!(r"{}\AppData\Roaming\Microsoft\Protect", home()),
                ],
            ),
        ]
    }

    #[cfg(target_os = "macos")]
    {
        vec![
            (
                "系统核心",
                vec![
                    "/System",
                    "/usr",
                    "/bin",
                    "/sbin",
                    "/opt",
                    "/etc",
                    "/private/etc",
                ]
                .into_iter()
                .map(String::from)
                .collect(),
            ),
            (
                "系统数据与框架",
                vec![
                    "/var/db",
                    "/private/var/db",
                    "/Library/Application Support",
                    "/Library/Preferences",
                    "/Library/Keychains",
                    "/Library/LaunchDaemons",
                    "/Library/Extensions",
                    "/Library/Receipts",
                    "/Cores",
                ]
                .into_iter()
                .map(String::from)
                .collect(),
            ),
            (
                "应用与维护数据",
                vec![
                    "/Applications（应用包本身）",
                    "~/.Trash（废纸篓，仅通过移入操作使用）",
                ]
                .into_iter()
                .map(String::from)
                .collect(),
            ),
            (
                "用户资料目录",
                vec![
                    format!("{}/Documents", home()),
                    format!("{}/Desktop", home()),
                    format!("{}/Downloads", home()),
                    format!("{}/Pictures", home()),
                    format!("{}/Movies", home()),
                    format!("{}/Music", home()),
                    format!("{}/Library/Mail", home()),
                    format!("{}/Library/Messages", home()),
                    format!("{}/Library/Safari", home()),
                    format!("{}/Library/Keychains", home()),
                ],
            ),
        ]
    }

    #[cfg(target_os = "linux")]
    {
        vec![
            (
                "系统核心",
                vec![
                    "/usr", "/etc", "/boot", "/proc", "/sys", "/dev", "/run", "/root", "/srv",
                ]
                .into_iter()
                .map(String::from)
                .collect(),
            ),
            (
                "库与系统数据",
                vec![
                    "/lib",
                    "/lib32",
                    "/lib64",
                    "/libx32",
                    "/var/lib",
                    "/var/spool",
                    "/lost+found",
                ]
                .into_iter()
                .map(String::from)
                .collect(),
            ),
            (
                "回收站",
                vec!["~/.local/share/Trash（仅通过移入操作使用）"]
                    .into_iter()
                    .map(String::from)
                    .collect(),
            ),
            (
                "用户资料目录",
                vec![
                    format!("{}/Documents", home()),
                    format!("{}/Desktop", home()),
                    format!("{}/Downloads", home()),
                    format!("{}/Pictures", home()),
                    format!("{}/Videos", home()),
                    format!("{}/Music", home()),
                    format!("{}/.ssh", home()),
                    format!("{}/.gnupg", home()),
                    format!("{}/.config", home()),
                    format!("{}/.local/share/applications", home()),
                ],
            ),
        ]
    }

    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
    {
        let _ = home;
        Vec::new()
    }
}

/// 「仍然需要注意的地方」：(是否弱化显示, 文本)。
fn cautions() -> Vec<(bool, String)> {
    let trash = crate::recycle::trash_name();
    let mut out = vec![(
        false,
        format!("「{trash}」这一项默认不勾选：清空{trash}后文件无法恢复。"),
    )];

    #[cfg(windows)]
    out.push((
        false,
        "「旧版 Windows 安装 C:\\Windows.old」属于高风险项：删除后无法再回滚系统版本，且部分文件归 TrustedInstaller 所有，可能只能删掉一部分。".to_string(),
    ));

    #[cfg(target_os = "macos")]
    out.push((
        false,
        "「/Library/Caches」「/var/log」这类系统级目录需要 sudo 才能清理，普通权限下会被整项跳过。"
            .to_string(),
    ));

    #[cfg(target_os = "linux")]
    out.push((
        false,
        "systemd 的日志不在文件层面清理：需要时请在终端执行 `journalctl --vacuum-time=7d` 回收。"
            .to_string(),
    ));

    out.push((
        false,
        "「永久删除」模式下删除的缓存无法找回，但都是程序会自动重建的数据。".to_string(),
    ));
    out.push((
        false,
        "浏览器缓存清理前建议先关闭浏览器，否则大部分文件会因占用被跳过。".to_string(),
    ));

    #[cfg(windows)]
    out.push((
        true,
        "如果某一项被反复跳过，多半是权限问题：右键以管理员身份运行可以解决系统级目录的清理。"
            .to_string(),
    ));
    #[cfg(not(windows))]
    out.push((
        true,
        format!(
            "如果某一项被反复跳过，多半是权限问题：{}",
            crate::sysutil::elevation_hint()
        ),
    ));

    out
}

pub fn show(_app: &mut App, ui: &mut egui::Ui) {
    let p = Palette::of(ui);

    // 承诺卡片
    card(ui, &p, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            widgets::section_title(ui, &p, "一句话版本");
            ui.add_space(6.0);
            pill(ui, "白名单 + 默认拒绝", PillKind::Accent, &p);
            ui.add_space(6.0);
            pill(ui, crate::sysutil::PLATFORM.label(), PillKind::Neutral, &p);
        });
        ui.add_space(8.0);
        ui.label(
            RichText::new(
                "这个程序只认识一份写死的、由人工挑选过的清理清单；除此之外的任何路径，它既看不见，也删不掉。",
            )
            .size(13.5)
            .color(p.text),
        );
    });

    ui.add_space(12.0);

    card(ui, &p, |ui| {
        ui.set_width(ui.available_width());
        widgets::section_title(ui, &p, "风险等级说明");
        ui.add_space(8.0);
        for level in crate::model::RiskLevel::ALL {
            ui.horizontal(|ui| {
                widgets::risk_pill(ui, level, &p);
                ui.add_space(6.0);
                ui.label(RichText::new(level.hint()).size(12.0).color(p.text_weak));
            });
            ui.add_space(4.0);
        }
    });

    ui.add_space(12.0);

    for item in items() {
        card(ui, &p, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(item.title).size(14.0).strong().color(p.text));
                ui.add_space(6.0);
                pill(ui, level_label(item.level), item.level, &p);
            });
            ui.add_space(6.0);
            ui.label(RichText::new(&item.body).size(12.5).color(p.text_weak));
        });
        ui.add_space(8.0);
    }

    // 保护名单
    card(ui, &p, |ui| {
        ui.set_width(ui.available_width());
        widgets::section_title(ui, &p, "永远在保护名单内的目录");
        ui.add_space(6.0);
        ui.label(
            RichText::new("下面这些目录树及其全部子项，无论出现在哪个清理目标里都会被拒绝：")
                .size(12.0)
                .color(p.text_weak),
        );
        ui.add_space(8.0);

        for (title, paths) in protected_groups() {
            ui.label(RichText::new(title).size(12.5).strong().color(p.accent));
            ui.add_space(4.0);
            for path in paths {
                bullet(ui, &p, p.text_faint, &path);
            }
            ui.add_space(8.0);
        }
    });

    ui.add_space(8.0);
    card(ui, &p, |ui| {
        ui.set_width(ui.available_width());
        widgets::section_title(ui, &p, "仍然需要你注意的地方");
        ui.add_space(8.0);
        for (faint, text) in cautions() {
            bullet(ui, &p, if faint { p.text_faint } else { p.caution }, &text);
        }
    });
}
