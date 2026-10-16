//! 视觉主题：调色板、字体、全局样式。
//!
//! 同时注册浅色与深色两套样式，交给 egui 按 `ThemePreference` 自动切换，
//! 因此"跟随系统"是真正生效的。

use std::sync::Arc;

use eframe::egui::{
    self, Color32, CornerRadius, FontFamily, FontId, Margin, Shadow, Stroke, Style, TextStyle,
    Theme, Visuals, vec2,
};

// ---------------------------------------------------------------------------
// 尺寸常量
// ---------------------------------------------------------------------------

/// 所有带背景的块的统一圆角。
///
/// 界面里出现的每一块「背景」——卡片、次级分区、模态框、浮层提示、列表行、
/// 按钮、输入框——都用这一个值，视觉上才会整齐。
///
/// 例外只有两处：按钮左侧的细强调条（2）与胶囊形进度条（按高度取半）。
pub const CARD_RADIUS: u8 = 5;

// ---------------------------------------------------------------------------
// 调色板
// ---------------------------------------------------------------------------

/// 一套主题色。所有颜色都是常量，避免每帧重复构造。
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    /// 窗口 / 面板底色
    pub bg: Color32,
    /// 侧边导航的底色。
    ///
    /// 参考图里侧栏比主区**略深一点点**，形成"内容浮在浅底上"的层次；
    /// 差值刻意做得很小，只是分出区域，不做成两种颜色。
    pub nav_bg: Color32,
    /// 侧栏选中项的胶囊底色 —— 比侧栏底再浅一档，读起来像一块浮起来的白片。
    pub nav_pill: Color32,
    /// 侧栏里分隔线的颜色。
    ///
    /// 侧栏底本来就比主区深，直接拿 [`border`](#field.border) 画上去几乎看不见。
    /// 这条线专门用来把侧栏底部那组「系统」从上面的导航里分出去 ——
    /// 得看得见，但不该抢戏。
    pub nav_line: Color32,
    /// 卡片底色
    pub surface: Color32,
    /// 卡片内的次级底色
    pub surface_alt: Color32,
    /// 悬停底色
    pub hover: Color32,
    /// 描边
    pub border: Color32,
    /// 主文字
    pub text: Color32,
    /// 次级文字
    pub text_weak: Color32,
    /// 更弱的文字（提示、脚注）
    pub text_faint: Color32,
    /// 强调色（大块填充：按钮、进度条）
    pub accent: Color32,
    /// 强调色的浅底
    pub accent_soft: Color32,
    /// 浅底上的强调色文字 / 图标。
    ///
    /// [`accent`] 本身偏亮，直接当文字色压在浅底上对比度不够，
    /// 所以文字与图标改用这一档更深的绿。
    pub accent_ink: Color32,
    /// 安全 / 成功
    pub safe: Color32,
    pub safe_soft: Color32,
    /// 谨慎 / 警告
    pub caution: Color32,
    pub caution_soft: Color32,
    /// 高风险 / 错误
    pub danger: Color32,
    pub danger_soft: Color32,
    /// 输入框等深色凹槽
    pub inset: Color32,
    /// 智能清理页那颗大圆按钮的填充色（参考图里是一颗很亮的薄荷绿）。
    pub cta: Color32,
    /// 大圆按钮「正在扫描」时的填充色 —— 比 [`cta`] 更饱和、更沉，
    /// 一眼能看出"现在在动"。参考图实测 `#66EBB3` → `#37EC99`。
    pub cta_busy: Color32,
    /// 压在大圆按钮上的文字色。参考图里是近黑的深绿，不是白字。
    pub cta_ink: Color32,
}

impl Palette {
    pub const LIGHT: Palette = Palette {
        // 「淡蓝底 + 纯白卡片」：底色调子是从参考图上逐像素量出来的 ——
        // 侧栏 #DCE6F2、主区 #E2EAF5，两者只差一丁点，靠这点差值分出区域。
        bg: Color32::from_rgb(0xE2, 0xEA, 0xF5),
        nav_bg: Color32::from_rgb(0xDC, 0xE6, 0xF2),
        // 选中项是一块**比侧栏底更浅**的胶囊：参考图里它恰好提到主区的亮度，
        // 这里再白一点点，让"当前在哪一页"更好认。
        nav_pill: Color32::from_rgb(0xEF, 0xF5, 0xFC),
        nav_line: Color32::from_rgb(0xCD, 0xD8, 0xE7),
        surface: Color32::from_rgb(0xFF, 0xFF, 0xFF),
        surface_alt: Color32::from_rgb(0xEE, 0xF3, 0xFA),
        hover: Color32::from_rgb(0xE6, 0xEE, 0xF8),
        border: Color32::from_rgb(0xD7, 0xE1, 0xEF),
        // 中性灰阶偏冷，和蓝紫底调统一
        text: Color32::from_rgb(0x1B, 0x24, 0x30),
        text_weak: Color32::from_rgb(0x55, 0x65, 0x7B),
        text_faint: Color32::from_rgb(0x8B, 0x99, 0xAC),
        // 品牌绿（参考图里那颗「一键清理」按钮的颜色）
        accent: Color32::from_rgb(0x12, 0xCF, 0x91),
        accent_soft: Color32::from_rgb(0xDA, 0xF6, 0xEA),
        accent_ink: Color32::from_rgb(0x12, 0xA3, 0x76),
        safe: Color32::from_rgb(0x0A, 0xA0, 0x66),
        safe_soft: Color32::from_rgb(0xDD, 0xF5, 0xEB),
        caution: Color32::from_rgb(0xB4, 0x76, 0x0C),
        caution_soft: Color32::from_rgb(0xFD, 0xF3, 0xE0),
        danger: Color32::from_rgb(0xD1, 0x2C, 0x2C),
        danger_soft: Color32::from_rgb(0xFD, 0xE9, 0xE9),
        inset: Color32::from_rgb(0xE6, 0xED, 0xF7),
        // 智能清理页大圆按钮：直接照参考图量到的色值（薄荷绿 + 更饱和的扫描中绿），
        // 文字用近黑的深绿压在上面 —— 参考图里不是白字，白字在这么亮的绿上会糊。
        cta: Color32::from_rgb(0x66, 0xEB, 0xB3),
        cta_busy: Color32::from_rgb(0x37, 0xEC, 0x99),
        cta_ink: Color32::from_rgb(0x0E, 0x33, 0x27),
    };

    pub const DARK: Palette = Palette {
        bg: Color32::from_rgb(0x12, 0x16, 0x1C),
        nav_bg: Color32::from_rgb(0x0D, 0x11, 0x16),
        nav_pill: Color32::from_rgb(0x24, 0x2D, 0x38),
        nav_line: Color32::from_rgb(0x24, 0x2C, 0x37),
        surface: Color32::from_rgb(0x1A, 0x20, 0x28),
        surface_alt: Color32::from_rgb(0x1F, 0x26, 0x2F),
        hover: Color32::from_rgb(0x26, 0x2F, 0x3A),
        border: Color32::from_rgb(0x2C, 0x36, 0x42),
        text: Color32::from_rgb(0xE9, 0xED, 0xF2),
        text_weak: Color32::from_rgb(0x9B, 0xA8, 0xB8),
        text_faint: Color32::from_rgb(0x6B, 0x78, 0x89),
        // 深色底下用更亮的绿，保证可读
        accent: Color32::from_rgb(0x1F, 0xD7, 0x9A),
        accent_soft: Color32::from_rgb(0x11, 0x33, 0x29),
        accent_ink: Color32::from_rgb(0x3B, 0xD8, 0xA2),
        safe: Color32::from_rgb(0x27, 0xC0, 0x85),
        safe_soft: Color32::from_rgb(0x0E, 0x2E, 0x22),
        caution: Color32::from_rgb(0xF5, 0xB0, 0x42),
        caution_soft: Color32::from_rgb(0x33, 0x29, 0x0F),
        danger: Color32::from_rgb(0xF8, 0x7A, 0x7A),
        danger_soft: Color32::from_rgb(0x3B, 0x1D, 0x1F),
        inset: Color32::from_rgb(0x0E, 0x12, 0x16),
        // 深色底下那颗圆按钮不能照搬浅色的薄荷绿（太刺眼、和深底打架），
        // 换成两档沉绿，文字相应改成浅色。
        cta: Color32::from_rgb(0x1B, 0x8F, 0x69),
        cta_busy: Color32::from_rgb(0x1F, 0xC1, 0x83),
        cta_ink: Color32::from_rgb(0xF0, 0xFB, 0xF6),
    };

    /// 根据当前 `Ui` 的主题取调色板。
    pub fn of(ui: &egui::Ui) -> Palette {
        if ui.visuals().dark_mode {
            Palette::DARK
        } else {
            Palette::LIGHT
        }
    }

    /// 没有 `Ui` 可用时（例如浮层、模态框）按 `Context` 取调色板。
    pub fn of_ctx(ctx: &egui::Context) -> Palette {
        if ctx.theme() == Theme::Dark {
            Palette::DARK
        } else {
            Palette::LIGHT
        }
    }

    pub fn is_dark(&self) -> bool {
        self.surface.r() < 0x80
    }
}

// ---------------------------------------------------------------------------
// 字体
// ---------------------------------------------------------------------------

/// Windows 系统自带的简体中文字体（按优先级排列，`.ttc` 需要指定 face 索引）。
#[cfg(windows)]
const WINDOWS_FONTS: &[(&str, u32)] = &[
    ("msyh.ttc", 0), // 微软雅黑
    ("msyh.ttf", 0),
    ("msyhl.ttc", 0),
    ("Deng.ttf", 0),   // 等线
    ("simhei.ttf", 0), // 黑体
    ("simsun.ttc", 0), // 宋体
    ("msjh.ttc", 0),   // 微软正黑（繁体系统）
];

/// macOS 系统自带的中文字体。
#[cfg(target_os = "macos")]
const MACOS_FONTS: &[(&str, u32)] = &[
    ("/System/Library/Fonts/PingFang.ttc", 0),         // 苹方
    ("/System/Library/Fonts/Hiragino Sans GB.ttc", 0), // 冬青黑体简体中文
    ("/System/Library/Fonts/STHeiti Light.ttc", 0),    // 华文黑体
    ("/System/Library/Fonts/Supplemental/Songti.ttc", 0),
    ("/Library/Fonts/Arial Unicode.ttf", 0),
];

/// Linux 发行版常见的中文字体路径。
///
/// 各家的装法差别很大（Debian 在 `/usr/share/fonts/opentype/noto`、
/// Arch 在 `/usr/share/fonts/noto-cjk`、Fedora 在
/// `/usr/share/fonts/google-noto-cjk`），所以这里先列常见路径，
/// 全都没命中时再退回 `scan_font_dirs()` 递归扫一遍字体目录。
#[cfg(target_os = "linux")]
const LINUX_FONTS: &[(&str, u32)] = &[
    // Noto Sans CJK 的 .ttc 里依次是 JP / KR / SC / TC / HK，简体在索引 2
    ("/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc", 2),
    (
        "/usr/share/fonts/opentype/noto/NotoSansCJKsc-Regular.otf",
        0,
    ),
    ("/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc", 2),
    ("/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc", 2),
    (
        "/usr/share/fonts/google-noto-cjk/NotoSansCJK-Regular.ttc",
        2,
    ),
    (
        "/usr/share/fonts/truetype/noto/NotoSansCJKsc-Regular.otf",
        0,
    ),
    // 文泉驿（很多老发行版的默认中文）
    ("/usr/share/fonts/truetype/wqy/wqy-zenhei.ttc", 0),
    ("/usr/share/fonts/wenquanyi/wqy-zenhei/wqy-zenhei.ttc", 0),
    ("/usr/share/fonts/truetype/wqy/wqy-microhei.ttc", 0),
    // 兜底：只要字形够全就行，字体好不好看是次要的
    (
        "/usr/share/fonts/truetype/droid/DroidSansFallbackFull.ttf",
        0,
    ),
    ("/usr/share/fonts/truetype/arphic/uming.ttc", 0),
];

/// 扫字体目录时认得的文件名关键词（都小写）。
#[cfg(target_os = "linux")]
const LINUX_FONT_HINTS: &[&str] = &[
    "notosanscjk",
    "notosanssc",
    "notosanshans",
    "sourcehansans",
    "wqy-zenhei",
    "wqy-microhei",
    "droidsansfallback",
    "uming",
];

/// 一个字体候选：文件 + 期望的 face 索引。
struct FontCandidate {
    path: std::path::PathBuf,
    face: u32,
    /// 用于界面提示的可读名字
    label: String,
}

/// 收集当前平台上的中文字体候选（按优先级）。
fn font_candidates() -> Vec<FontCandidate> {
    let mut out = Vec::new();

    #[cfg(windows)]
    {
        let dir = crate::sysutil::windows_dir().join("Fonts");
        for (file, face) in WINDOWS_FONTS {
            out.push(FontCandidate {
                path: dir.join(file),
                face: *face,
                label: format!(
                    "{file}（{}\\Fonts）",
                    crate::sysutil::windows_dir().display()
                ),
            });
        }
    }

    #[cfg(target_os = "macos")]
    {
        for (path, face) in MACOS_FONTS {
            out.push(FontCandidate {
                path: std::path::PathBuf::from(path),
                face: *face,
                label: (*path).to_string(),
            });
        }
        // 用户自己装的字体也认
        if let Some(home) = crate::sysutil::home_dir() {
            for name in ["PingFang.ttc", "Songti.ttc", "SimHei.ttf"] {
                out.push(FontCandidate {
                    path: home.join("Library/Fonts").join(name),
                    face: 0,
                    label: format!("~/Library/Fonts/{name}"),
                });
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        for (path, face) in LINUX_FONTS {
            out.push(FontCandidate {
                path: std::path::PathBuf::from(path),
                face: *face,
                label: (*path).to_string(),
            });
        }
        if let Some(home) = crate::sysutil::home_dir() {
            out.push(FontCandidate {
                path: home.join(".local/share/fonts/NotoSansCJK-Regular.ttc"),
                face: 2,
                label: "~/.local/share/fonts/NotoSansCJK-Regular.ttc".to_string(),
            });
        }
        out.extend(scan_font_dirs());
    }

    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
    {
        let _ = &mut out;
    }

    out
}

/// Linux 兜底：递归扫一遍标准字体目录，按文件名关键词挑出中文字体。
#[cfg(target_os = "linux")]
fn scan_font_dirs() -> Vec<FontCandidate> {
    const ROOTS: &[&str] = &[
        "/usr/share/fonts",
        "/usr/local/share/fonts",
        "/usr/share/X11/fonts",
    ];

    let mut found = Vec::new();
    let mut stack: Vec<(std::path::PathBuf, u32)> = ROOTS
        .iter()
        .map(|r| (std::path::PathBuf::from(r), 0u32))
        .collect();

    while let Some((dir, depth)) = stack.pop() {
        if depth > 5 {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push((path, depth + 1));
                continue;
            }
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let lower = name.to_ascii_lowercase();
            if !LINUX_FONT_HINTS.iter().any(|hint| lower.contains(hint)) {
                continue;
            }
            // Noto 的合集里简体在索引 2，单字重的 .otf 就是索引 0
            let face = if lower.contains("notosanscjk") && lower.ends_with(".ttc") {
                2
            } else {
                0
            };
            found.push(FontCandidate {
                label: path.to_string_lossy().to_string(),
                path,
                face,
            });
        }
    }

    // 让列表稳定下来，避免每次启动挑到不同的字体
    found.sort_by(|a, b| a.label.cmp(&b.label));
    found
}

/// `.ttc` 里塞了多个 face，索引越界会让字体加载失败。
///
/// TTC 文件头是 `ttcf` + 版本(4 字节) + face 数量(4 字节)，读一下就能
/// 把索引收进合法范围，比直接猜要稳。
fn clamp_face_index(bytes: &[u8], wanted: u32) -> u32 {
    if bytes.len() < 12 || &bytes[0..4] != b"ttcf" {
        return 0;
    }
    let count = u32::from_be_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
    if count == 0 {
        return 0;
    }
    wanted.min(count - 1)
}

/// 找到的第一个可用中文字体：返回（字体名, 字节, face 索引, 来源说明）。
fn load_system_cjk_font() -> Option<(String, Vec<u8>, u32, String)> {
    for candidate in font_candidates() {
        let Ok(bytes) = std::fs::read(&candidate.path) else {
            continue;
        };
        if bytes.is_empty() {
            continue;
        }
        let face = clamp_face_index(&bytes, candidate.face);
        let name = format!("sys-{}", candidate.path.file_name()?.to_string_lossy());
        return Some((name, bytes, face, candidate.label));
    }
    None
}

/// 供 `--selftest` 使用：报告中文体的探测结果。
pub fn probe_cjk_font() -> String {
    match load_system_cjk_font() {
        Some((_, bytes, face, label)) => {
            format!("已找到 {label}（face {face}，{} KB）", bytes.len() / 1024)
        }
        None => format!(
            "未在 {} 上找到中文字体，界面中文可能缺字",
            crate::sysutil::PLATFORM.label()
        ),
    }
}

/// 给 egui 装上中文字体；未找到系统字体时原样返回（界面仍可运行，只是中文会缺字）。
pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();

    if let Some((name, bytes, index, label)) = load_system_cjk_font() {
        let mut data = egui::FontData::from_owned(bytes);
        data.index = index;
        fonts.font_data.insert(name.clone(), Arc::new(data));

        // 放在第一位：中文优先用系统字体渲染，英文字符也保持同一套字形的观感
        fonts
            .families
            .entry(FontFamily::Proportional)
            .or_default()
            .insert(0, name.clone());
        // 等宽字体里作为兜底（中文注释、路径不出现豆腐块）
        fonts
            .families
            .entry(FontFamily::Monospace)
            .or_default()
            .push(name);
        eprintln!("中文字体：{label}");
    }

    ctx.set_fonts(fonts);
}

// ---------------------------------------------------------------------------
// 样式
// ---------------------------------------------------------------------------

fn build_visuals(p: &Palette) -> Visuals {
    let dark = p.is_dark();
    let mut v = if dark {
        Visuals::dark()
    } else {
        Visuals::light()
    };

    v.dark_mode = dark;
    v.panel_fill = p.bg;
    v.window_fill = p.surface;
    v.window_stroke = Stroke::new(1.0, p.border);
    v.window_corner_radius = CornerRadius::same(CARD_RADIUS);
    v.window_shadow = Shadow {
        offset: [0, 8],
        blur: 24,
        spread: 0,
        color: Color32::from_black_alpha(if dark { 120 } else { 20 }),
    };
    v.popup_shadow = Shadow {
        offset: [0, 4],
        blur: 16,
        spread: 0,
        color: Color32::from_black_alpha(if dark { 90 } else { 22 }),
    };
    v.override_text_color = None;
    v.hyperlink_color = p.accent;
    v.warn_fg_color = p.caution;
    v.error_fg_color = p.danger;
    v.faint_bg_color = p.surface_alt;
    v.extreme_bg_color = p.inset;
    v.code_bg_color = p.surface_alt;
    v.selection.bg_fill = p.accent.gamma_multiply(if dark { 0.45 } else { 0.28 });
    v.selection.stroke = Stroke::new(1.0, p.accent);
    v.window_highlight_topmost = false;

    let radius = CornerRadius::same(CARD_RADIUS);

    // 静态元素（标签、分隔线等）
    v.widgets.noninteractive.bg_fill = p.surface_alt;
    v.widgets.noninteractive.weak_bg_fill = p.surface_alt;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    v.widgets.noninteractive.corner_radius = radius;
    v.widgets.noninteractive.expansion = 0.0;

    // 常态
    v.widgets.inactive.bg_fill = p.surface;
    v.widgets.inactive.weak_bg_fill = p.surface_alt;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, p.border);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, p.text);
    v.widgets.inactive.corner_radius = radius;
    v.widgets.inactive.expansion = 0.0;

    // 悬停 —— 参考图的悬停反馈很轻，只换底色、描边几乎不动
    v.widgets.hovered.bg_fill = p.hover;
    v.widgets.hovered.weak_bg_fill = p.hover;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, p.border);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, p.text);
    v.widgets.hovered.corner_radius = radius;
    v.widgets.hovered.expansion = 0.0;

    // 按下
    v.widgets.active.bg_fill = p.accent.gamma_multiply(if dark { 0.55 } else { 0.85 });
    v.widgets.active.weak_bg_fill = p.accent;
    v.widgets.active.bg_stroke = Stroke::new(1.0, p.accent);
    v.widgets.active.fg_stroke = Stroke::new(1.0, Color32::WHITE);
    v.widgets.active.corner_radius = radius;
    v.widgets.active.expansion = 0.0;

    // 展开
    v.widgets.open.bg_fill = p.surface_alt;
    v.widgets.open.weak_bg_fill = p.surface_alt;
    v.widgets.open.bg_stroke = Stroke::new(1.0, p.border);
    v.widgets.open.fg_stroke = Stroke::new(1.0, p.text);
    v.widgets.open.corner_radius = radius;

    v
}

fn build_style(p: &Palette) -> Style {
    let mut style = Style::default();

    style.spacing.item_spacing = vec2(8.0, 8.0);
    style.spacing.button_padding = vec2(12.0, 7.0);
    style.spacing.window_margin = Margin::same(16);
    style.spacing.menu_margin = Margin::same(8);
    style.spacing.indent = 18.0;
    style.spacing.interact_size = vec2(28.0, 26.0);
    style.spacing.slider_width = 160.0;
    style.spacing.combo_width = 150.0;
    style.spacing.icon_width = 16.0;
    style.spacing.scroll.bar_width = 9.0;
    style.spacing.scroll.bar_inner_margin = 2.0;
    style.spacing.scroll.floating = true;
    style.animation_time = 0.12;
    style.visuals = build_visuals(p);

    style.text_styles = [
        (
            TextStyle::Small,
            FontId::new(11.0, FontFamily::Proportional),
        ),
        (TextStyle::Body, FontId::new(13.5, FontFamily::Proportional)),
        (
            TextStyle::Button,
            FontId::new(13.5, FontFamily::Proportional),
        ),
        (
            TextStyle::Heading,
            FontId::new(20.0, FontFamily::Proportional),
        ),
        (
            TextStyle::Monospace,
            FontId::new(12.5, FontFamily::Monospace),
        ),
    ]
    .into();

    style
}

/// 注册浅色 / 深色共两套样式，并设置主题偏好。
pub fn install(ctx: &egui::Context, pref: crate::settings::ThemePref) {
    use crate::settings::ThemePref;

    ctx.set_style_of(Theme::Light, build_style(&Palette::LIGHT));
    ctx.set_style_of(Theme::Dark, build_style(&Palette::DARK));

    let preference = match pref {
        ThemePref::System => egui::ThemePreference::System,
        ThemePref::Light => egui::ThemePreference::Light,
        ThemePref::Dark => egui::ThemePreference::Dark,
    };
    ctx.set_theme(preference);
}

// ---------------------------------------------------------------------------
// 常用组合
// ---------------------------------------------------------------------------

/// 卡片的统一外观 —— 参考图那种「白底 + 一根极细边框、完全没有阴影」的平面卡片。
pub fn card_frame(p: &Palette) -> egui::Frame {
    egui::Frame::new()
        .fill(p.surface)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(CARD_RADIUS))
        .inner_margin(Margin::same(20))
}

/// 内嵌小块的统一外观（卡片内部的次级分区）。
///
/// 参考图里这种浅灰区块**不带边框**，只靠底色区分层级。
pub fn section_frame(p: &Palette) -> egui::Frame {
    egui::Frame::new()
        .fill(p.surface_alt)
        .corner_radius(CornerRadius::same(CARD_RADIUS))
        .inner_margin(Margin::symmetric(14, 12))
}

/// 胶囊标签的底色与文字色。
pub fn pill_colors(p: &Palette, kind: PillKind) -> (Color32, Color32) {
    match kind {
        PillKind::Neutral => (p.surface_alt, p.text_weak),
        PillKind::Accent => (p.accent_soft, p.accent),
        PillKind::Safe => (p.safe_soft, p.safe),
        PillKind::Caution => (p.caution_soft, p.caution),
        PillKind::Danger => (p.danger_soft, p.danger),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PillKind {
    Neutral,
    Accent,
    Safe,
    Caution,
    Danger,
}

/// 磁盘占用条的颜色：越满越红。
pub fn usage_color(p: &Palette, ratio: f32) -> Color32 {
    if ratio >= 0.9 {
        p.danger
    } else if ratio >= 0.75 {
        p.caution
    } else {
        p.accent
    }
}

/// 两个颜色之间线性插值（`t` = 0 取 `a`，`t` = 1 取 `b`）。
///
/// 自己写而不用 egui 自带的插值，是为了不去记它到底在哪个色彩空间里做 ——
/// 这里的用法（渐变的两个端点、给一块浅底掺一点色）都只要求"看着均匀"。
pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let f = |x: u8, y: u8| {
        (x as f32 + (y as f32 - x as f32) * t)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    Color32::from_rgb(f(a.r(), b.r()), f(a.g(), b.g()), f(a.b(), b.b()))
}

/// 主按钮的上下渐变端点（上浅下深）。
///
/// 参考图那颗「一键清理」是带一点纵向渐变的 —— 纯色大圆角块会显得很平，
/// 上下差个一两成就有了体积感。
pub fn accent_gradient(p: &Palette) -> (Color32, Color32) {
    (
        mix(p.accent, Color32::WHITE, 0.20),
        mix(p.accent, Color32::BLACK, 0.10),
    )
}

/// 在 `rect` 里铺一层纵向渐变，四角仍然是圆角，边缘带 1px 羽化（抗锯齿）。
///
/// egui 的 `Mesh` 只能给顶点上色，也没有现成的"圆角 + 渐变"图元，所以这里
/// 自己把矩形切成若干条横带，每条按圆角曲线算出左右内缩，逐条插值出上下两个
/// 颜色（渐变本身是线性的，三角形内部又是一次线性插值，所以条数只影响轮廓，
/// 不影响颜色过渡）。
///
/// 两个容易踩的点，都在圆角上：
///
/// 1. **切片不能按高度均匀取。** 圆角的顶端"横着走"得极快 —— 半径 26 时，
///    离顶边 0.25px 的地方轮廓已经内缩了 3.6px，均匀切片会把这段画成一截截
///    十来像素宽的台阶，也就是肉眼看到的锯齿。改成**按角度**采样圆角，横向
///    跑得最快的地方自然就密，几何误差降到 0.05px 以下。
/// 2. **`Mesh` 不走 egui 的羽化。** `rect_filled` 之所以边缘是软的，是因为它
///    走路径填充、外面多画了一圈渐隐到透明的边；三角形网格没有这一层，轮廓再
///    准也是硬边。所以最后沿左右轮廓再补一圈 1px 的半透明外扩环，把硬边糊掉。
pub fn paint_vertical_gradient(
    painter: &egui::Painter,
    rect: egui::Rect,
    radius: f32,
    top: Color32,
    bottom: Color32,
) {
    /// 每个圆角切成多少段 —— 半径 26 时单段误差约 0.03px
    const ARC_SEGMENTS: usize = 12;
    /// 羽化环的宽度
    const FEATHER: f32 = 1.0;
    /// 填充相对真实轮廓内缩的量。
    ///
    /// 填充是**硬边**光栅化（一个像素中心落在轮廓里，整个像素就被涂满），
    /// 所以它天生比真实覆盖率最多多盖 0.5px；而羽化环只能**往外**补覆盖率、
    /// 减不掉已经涂满的像素。让填充停在「轮廓往里 0.5px」处，刚好接上羽化环
    /// alpha=1 的那条内边 —— 环带覆盖度 `0.5 − 到轮廓的距离`，于是逐像素的
    /// 覆盖率就等于真实覆盖率。内缩多了（试过 1px）反而会在两者之间留出
    /// 半像素的缝，轮廓上出现一圈空洞。
    const FILL_INSET: f32 = 0.5;

    let h = rect.height();
    if h <= 0.0 || rect.width() <= 0.0 {
        return;
    }
    let r = radius.min(h / 2.0).min(rect.width() / 2.0);

    // 形状太小时按比例缩水，避免内缩之后梯形自交
    let inset = FILL_INSET
        .min(r * 0.5)
        .min(rect.width() * 0.25)
        .min(h * 0.25);
    let fill_rect = rect.shrink(inset);
    let fill_r = (r - inset).max(0.0);

    let mut mesh = egui::Mesh::default();

    // ---- 主体：相邻两片之间填一个梯形 ----
    let fill_rows = gradient_rows(fill_rect, fill_r, ARC_SEGMENTS);
    for pair in fill_rows.windows(2) {
        let (y0, y1) = (pair[0], pair[1]);
        if (y1 - y0).abs() < f32::EPSILON {
            continue;
        }
        let (in0, in1) = (
            corner_inset(y0, fill_rect, fill_r),
            corner_inset(y1, fill_rect, fill_r),
        );
        // 渐变色仍然按**真实**矩形取，这样环带和填充的颜色能接上
        let (c0, c1) = (
            color_at(y0, rect, top, bottom),
            color_at(y1, rect, top, bottom),
        );

        let base = mesh.vertices.len() as u32;
        mesh.colored_vertex(egui::pos2(fill_rect.left() + in0, y0), c0);
        mesh.colored_vertex(egui::pos2(fill_rect.right() - in0, y0), c0);
        mesh.colored_vertex(egui::pos2(fill_rect.right() - in1, y1), c1);
        mesh.colored_vertex(egui::pos2(fill_rect.left() + in1, y1), c1);
        mesh.add_triangle(base, base + 1, base + 2);
        mesh.add_triangle(base, base + 2, base + 3);
    }

    // ---- 羽化：沿**真实**轮廓补一圈骑缝的半透明边 ----
    let ys = gradient_rows(rect, r, ARC_SEGMENTS);
    if ys.len() < 2 {
        painter.add(egui::Shape::mesh(mesh));
        return;
    }
    let left: Vec<egui::Pos2> = ys
        .iter()
        .map(|y| egui::pos2(rect.left() + corner_inset(*y, rect, r), *y))
        .collect();
    let right: Vec<egui::Pos2> = ys
        .iter()
        .map(|y| egui::pos2(rect.right() - corner_inset(*y, rect, r), *y))
        .collect();

    // 两条轮廓都自上而下：左轮廓外法线取 (-dy, dx)，右轮廓取 (dy, -dx)。
    // 右轮廓**不能反转**——方向一反，外法线就指到形状里面去了，羽化环会画在
    // 填充底下，边缘看起来还是硬的。
    feather_chain(&mut mesh, &left, FEATHER, false, rect, top, bottom);
    feather_chain(&mut mesh, &right, FEATHER, true, rect, top, bottom);

    // 上下两条直边
    feather_edge(
        &mut mesh,
        left[0],
        right[0],
        egui::vec2(0.0, -1.0),
        FEATHER,
        color_at(rect.top(), rect, top, bottom),
    );
    feather_edge(
        &mut mesh,
        *right.last().unwrap_or(&right[0]),
        *left.last().unwrap_or(&left[0]),
        egui::vec2(0.0, 1.0),
        FEATHER,
        color_at(rect.bottom(), rect, top, bottom),
    );

    painter.add(egui::Shape::mesh(mesh));
}

/// 渐变在高度 `y` 处的颜色。
fn color_at(y: f32, rect: egui::Rect, top: Color32, bottom: Color32) -> Color32 {
    let t = ((y - rect.top()) / rect.height().max(f32::EPSILON)).clamp(0.0, 1.0);
    mix(top, bottom, t)
}

/// 纵向切片的位置列表（升序、已去重）。
///
/// 圆角按角度采样：`y = top + r·(1 - cos φ)`。φ 在 90° 附近时 `cos` 变化很慢，
/// 于是切片密集地落在圆角顶端 —— 正好是轮廓横向跑得最快的一段。
fn gradient_rows(rect: egui::Rect, r: f32, segments: usize) -> Vec<f32> {
    let step = std::f32::consts::FRAC_PI_2 / segments.max(1) as f32;
    let mut ys: Vec<f32> = Vec::with_capacity(segments * 4 + 4);
    ys.push(rect.top());
    for i in 1..segments {
        ys.push(rect.top() + r - r * (step * i as f32).cos());
        ys.push(rect.bottom() - r + r * (step * i as f32).cos());
    }
    ys.push(rect.top() + r);
    ys.push(rect.bottom() - r);
    ys.push(rect.bottom());

    ys.retain(|y| *y > f32::NEG_INFINITY && *y < f32::INFINITY);
    ys.sort_by(|a, b| a.total_cmp(b));
    ys.dedup();
    ys
}

/// 沿一条轮廓折线补一圈外扩的半透明边。
///
/// `on_right` 为真时外法线取 `(dy, -dx)`（右轮廓），否则取 `(-dy, dx)`（左轮廓）。
fn feather_chain(
    mesh: &mut egui::Mesh,
    points: &[egui::Pos2],
    feather: f32,
    on_right: bool,
    rect: egui::Rect,
    top: Color32,
    bottom: Color32,
) {
    for pair in points.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let dx = b.x - a.x;
        let dy = b.y - a.y;
        let len = (dx * dx + dy * dy).sqrt();
        if len < f32::EPSILON {
            continue;
        }
        let n = if on_right {
            egui::vec2(dy / len, -dx / len)
        } else {
            egui::vec2(-dy / len, dx / len)
        };
        feather_edge(
            mesh,
            a,
            b,
            n,
            feather,
            color_at((a.y + b.y) / 2.0, rect, top, bottom),
        );
    }
}

/// 一条边的羽化：环带**骑在轮廓上**（内半像素不透明、外半像素透明）。
///
/// 之前把整条环带贴在外侧，于是"轮廓正好落在像素边界上"的行一个过渡像素都没有
/// —— 实测 54 行里有 12 行是 1.0 → 0.0 的硬跳变，圆角看上去还是有台阶。
/// 环带居中之后，像素中心的覆盖度 = `0.5 - 到轮廓的有向距离`，正好等于真实的
/// 面积覆盖率（直边情形是精确解），每一行都能拿到过渡像素。
fn feather_edge(
    mesh: &mut egui::Mesh,
    a: egui::Pos2,
    b: egui::Pos2,
    normal: egui::Vec2,
    feather: f32,
    color: Color32,
) {
    /// 环带向内让出的宽度 —— 半个像素，于是不透明顶点正好压在轮廓内侧
    const INNER: f32 = 0.5;

    // 预乘 alpha 下的"透明"必须是全 0，只把 a 置 0 会留下颜色残渣
    let clear = Color32::from_rgba_premultiplied(0, 0, 0, 0);
    let (ai, bi) = (a - normal * INNER, b - normal * INNER);
    let (ao, bo) = (
        a + normal * (feather - INNER),
        b + normal * (feather - INNER),
    );
    let base = mesh.vertices.len() as u32;
    mesh.colored_vertex(ai, color);
    mesh.colored_vertex(bi, color);
    mesh.colored_vertex(bo, clear);
    mesh.colored_vertex(ao, clear);
    mesh.add_triangle(base, base + 1, base + 2);
    mesh.add_triangle(base, base + 2, base + 3);
}

/// 圆角矩形在高度 `y` 处需要往内缩多少。
fn corner_inset(y: f32, rect: egui::Rect, r: f32) -> f32 {
    let dy = if y < rect.top() + r {
        rect.top() + r - y
    } else if y > rect.bottom() - r {
        y - (rect.bottom() - r)
    } else {
        return 0.0;
    };
    r - (r * r - dy * dy).max(0.0).sqrt()
}
