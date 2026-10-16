//! 界面骨架：顶栏、侧边导航、底部状态栏、浮层提示。

use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, Layout, Margin, Rect, Response, RichText, Sense,
    Stroke, Vec2, vec2,
};

use crate::app::{App, BigState, CleanState, Page, ScanState, ToastKind};
use crate::model::{human_bytes, thousands};
use crate::settings::ThemePref;
use crate::sysutil;
use crate::theme::{Palette, PillKind};
use crate::ui::icons;
use crate::ui::widgets;

// ---------------------------------------------------------------------------
// 标题栏（自绘，替代系统自带的那一套）
// ---------------------------------------------------------------------------

/// 自绘标题栏的高度。
pub const TITLE_BAR_HEIGHT: f32 = 36.0;

/// 窗口按钮的点击区边长。
const WIN_BUTTON: f32 = 26.0;
/// 窗口按钮之间的间隙 —— 加上按钮宽度正好是参考图里 31px 左右的节拍。
const WIN_BUTTON_GAP: f32 = 2.0;
/// 线框图形的边长。
///
/// 参考图里量出来是 12px，但那是**人家 920px 宽窗口**上的 12px；同一颗图形放到
/// 1120px 的窗口上会显得偏大、偏重，所以缩到 10px。
const WIN_GLYPH: f32 = 10.0;
/// 按钮离窗口右边缘的距离。
///
/// 参考图里关闭键那根线的右端距窗口右边缘 17px，这里按同样的观感取 9px —— 因为
/// 点击区宽 26、图形只有 10，两侧各有 8px 空白，9 + 8 正好把图形右端放到离边 17px。
const WIN_BUTTON_MARGIN: f32 = 9.0;
/// 图形线宽。参考图里的最小化是**一根 1px 细线**，之前用 1.5px 会显得笨重。
const WIN_STROKE: f32 = 1.0;

#[derive(Clone, Copy)]
enum WinButton {
    Minimize,
    Maximize,
    Close,
}

/// 窗口标题栏：左侧拖动 + 最小化 / 最大化 / 关闭。
///
/// 参考图里的这三个按钮是**极简线框**——没有按钮底框，只有一根 12px 的灰线，
/// 三个之间等距排开，右边缘留一指宽。这里照做：平时只有线，悬停才浮出一层浅底
/// （关闭键转红），把"能点"这件事留到鼠标移上去的那一刻再说。
pub fn title_bar(ui: &mut egui::Ui) {
    let p = Palette::of(ui);

    egui::Panel::top("cleanc_titlebar")
        .exact_size(TITLE_BAR_HEIGHT)
        .resizable(false)
        .show_separator_line(false)
        .frame(
            // 和主区同一个底色，中间不画线 —— 标题栏是靠"留白"分出来的区域
            egui::Frame::new().fill(p.bg),
        )
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                // 横向间距清零：这一条是**手算**出来的排布，再叠上样式里 8px 的
                // 默认间距就会把最右边的按钮挤出窗口。
                ui.spacing_mut().item_spacing.x = 0.0;

                let group_w = WIN_BUTTON * 3.0 + WIN_BUTTON_GAP * 2.0;
                let drag_w = (ui.available_width() - group_w - WIN_BUTTON_MARGIN).max(60.0);

                // ---- 左侧整片都是拖动区 ----
                // 双击等同最大化和还原，这是各平台的通用习惯
                let (rect, resp) =
                    ui.allocate_exact_size(vec2(drag_w, TITLE_BAR_HEIGHT), Sense::click_and_drag());
                if resp.drag_started() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }
                if resp.double_clicked() {
                    toggle_maximized(ui.ctx());
                }
                ui.painter().text(
                    egui::pos2(rect.left() + 16.0, rect.center().y),
                    Align2::LEFT_CENTER,
                    "磁盘清理 · CleanC",
                    egui::FontId::proportional(11.5),
                    p.text_faint,
                );

                // ---- 右侧三个窗口按钮 ----
                //
                // 用**右对齐布局**把这一组钉在窗口右边缘，而不是靠前面"算准拖动区
                // 剩多少"。这样三个按钮的位置只由窗口宽度和下面几个常量决定，翻到
                // 哪一页、页面内容多宽多窄都影响不到它 —— 左右两栏的比例一变，按钮
                // 也不会跟着漂。
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = WIN_BUTTON_GAP;
                    ui.add_space(WIN_BUTTON_MARGIN);
                    // 右对齐下要**倒着**给：先声明的贴在最右边
                    for kind in [WinButton::Close, WinButton::Maximize, WinButton::Minimize] {
                        if win_button(ui, &p, kind).clicked() {
                            match kind {
                                WinButton::Minimize => ui
                                    .ctx()
                                    .send_viewport_cmd(egui::ViewportCommand::Minimized(true)),
                                WinButton::Maximize => toggle_maximized(ui.ctx()),
                                WinButton::Close => {
                                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close)
                                }
                            }
                        }
                    }
                });
            });
        });
}

fn toggle_maximized(ctx: &egui::Context) {
    let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
    ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
}

fn win_button(ui: &mut egui::Ui, p: &Palette, kind: WinButton) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(WIN_BUTTON), Sense::click());
    let hovered = resp.hovered();
    let painter = ui.painter();

    let (fill, glyph) = match kind {
        WinButton::Close if hovered => (Some(p.danger), Color32::WHITE),
        _ if hovered => (Some(p.hover), p.text),
        _ => (None, p.text_weak),
    };
    if let Some(fill) = fill {
        painter.rect_filled(rect, CornerRadius::same(4), fill);
    }

    let box_ = Rect::from_center_size(rect.center(), Vec2::splat(WIN_GLYPH));
    let stroke = Stroke::new(WIN_STROKE, glyph);
    let (min, max) = (box_.min, box_.max);

    match kind {
        // 最小化：一条横线
        WinButton::Minimize => {
            painter.line_segment(
                [
                    egui::pos2(min.x, rect.center().y),
                    egui::pos2(max.x, rect.center().y),
                ],
                stroke,
            );
        }
        // 最大化：一个方框；已经最大化时换成"两个错开的小方框"，表示点下去是还原
        WinButton::Maximize => {
            let maximized = ui.input(|i| i.viewport().maximized.unwrap_or(false));
            if !maximized {
                painter.rect_stroke(box_, CornerRadius::ZERO, stroke, egui::StrokeKind::Outside);
            } else {
                let side = WIN_GLYPH - 4.0;
                let back = Rect::from_min_size(egui::pos2(min.x + 2.0, min.y), vec2(side, side));
                let front = Rect::from_min_size(egui::pos2(min.x, min.y + 2.0), vec2(side, side));
                painter.rect_stroke(back, CornerRadius::ZERO, stroke, egui::StrokeKind::Outside);
                // 前面那个要用底色垫一下，交叠处才不会露出后面那根线
                painter.rect_filled(front, CornerRadius::ZERO, fill.unwrap_or(p.bg));
                painter.rect_stroke(front, CornerRadius::ZERO, stroke, egui::StrokeKind::Outside);
            }
        }
        // 关闭：一个叉
        WinButton::Close => {
            painter.line_segment([min, max], stroke);
            painter.line_segment([egui::pos2(max.x, min.y), egui::pos2(min.x, max.y)], stroke);
        }
    }

    if hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

/// 无边框窗口的缩放手柄。
///
/// 关掉系统装饰后，窗口四周就没有可拖的边了，最大化之外没法改尺寸。这里沿着
/// 窗口的三条边和四个角各放一条看不见的拖拽区，拖下去就交给 winit 去改窗口大小
/// —— 只有 6px 宽，平时完全不会挡住内容。
///
/// 顶边不铺：那里是标题栏的拖动区，拖顶边 resize 的需求也最少。
pub fn resize_handles(ctx: &egui::Context) {
    /// 边上的手柄厚度
    const EDGE: f32 = 6.0;
    /// 角落手柄的边长
    const CORNER: f32 = 14.0;

    let window = ctx.content_rect();
    if window.width() < CORNER * 2.0 || window.height() < CORNER * 2.0 {
        return;
    }

    use egui::viewport::ResizeDirection as Dir;
    let (l, t, r, b) = (window.left(), window.top(), window.right(), window.bottom());
    let handles: [(Rect, Dir, egui::CursorIcon); 7] = [
        (
            Rect::from_min_max(egui::pos2(l + CORNER, b - EDGE), egui::pos2(r - CORNER, b)),
            Dir::South,
            egui::CursorIcon::ResizeVertical,
        ),
        (
            Rect::from_min_max(egui::pos2(l, t + CORNER), egui::pos2(l + EDGE, b - CORNER)),
            Dir::West,
            egui::CursorIcon::ResizeHorizontal,
        ),
        (
            Rect::from_min_max(egui::pos2(r - EDGE, t + CORNER), egui::pos2(r, b - CORNER)),
            Dir::East,
            egui::CursorIcon::ResizeHorizontal,
        ),
        (
            Rect::from_min_size(egui::pos2(l, b - CORNER), Vec2::splat(CORNER)),
            Dir::SouthWest,
            egui::CursorIcon::ResizeSouthWest,
        ),
        (
            Rect::from_min_size(egui::pos2(r - CORNER, b - CORNER), Vec2::splat(CORNER)),
            Dir::SouthEast,
            egui::CursorIcon::ResizeSouthEast,
        ),
        (
            Rect::from_min_size(egui::pos2(l, t), Vec2::splat(CORNER)),
            Dir::NorthWest,
            egui::CursorIcon::ResizeNorthWest,
        ),
        (
            Rect::from_min_size(egui::pos2(r - CORNER, t), Vec2::splat(CORNER)),
            Dir::NorthEast,
            egui::CursorIcon::ResizeNorthEast,
        ),
    ];

    for (index, (rect, dir, cursor)) in handles.iter().enumerate() {
        let inner = egui::Area::new(egui::Id::new(("cleanc_resize", index)))
            .fixed_pos(rect.min)
            .order(egui::Order::Foreground)
            .interactable(true)
            .show(ctx, |ui| {
                ui.allocate_exact_size(rect.size(), Sense::drag()).1
            })
            .inner;
        if inner.hovered() {
            ctx.set_cursor_icon(*cursor);
        }
        if inner.drag_started() {
            ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(*dir));
        }
    }
}

// ---------------------------------------------------------------------------
// 顶栏
// ---------------------------------------------------------------------------

/// 顶栏的总高度，以及扣掉上下内边距之后留给内容的高度。
const TOP_BAR_HEIGHT: f32 = 64.0;
const TOP_BAR_INNER: f32 = TOP_BAR_HEIGHT - 20.0;

pub fn top_bar(app: &mut App, ui: &mut egui::Ui) {
    let p = Palette::of(ui);
    let ctx = ui.ctx().clone();

    egui::Panel::top("cleanc_topbar")
        .exact_size(TOP_BAR_HEIGHT)
        .resizable(false)
        .show_separator_line(false)
        .frame(
            // 参考图里顶栏和主区是同一个底色，中间不画分隔线 ——
            // 靠"内容留白"分区，而不是靠线。
            egui::Frame::new()
                .fill(p.bg)
                .inner_margin(Margin::symmetric(22, 10)),
        )
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                // 宽度在这里**手算**，不靠布局自己分。
                //
                // 直接"左边标题 + 右边靠右排"是不行的：副标题是 `Page::subtitle()`
                // 里的页面文案，长度不受这一层控制，而横向布局里的 `Label` 又不换行，
                // 长文案会直接压到磁盘信息上。反过来先摆右侧、再让标题吃剩下的宽度
                // 也不行 —— 嵌套的那层 `Ui` 在 RTL 方向上量出来的可用宽度是错的，
                // 标题会被挤成一列一个字。
                let full = ui.available_width();
                let right_w = (full * 0.62).min(660.0);
                let title_w = (full - right_w).max(120.0);

                ui.allocate_ui_with_layout(
                    vec2(title_w, TOP_BAR_INNER),
                    Layout::left_to_right(Align::Center),
                    |ui| {
                        // 标题统一收到顶栏，页内就不再重复一遍大标题了
                        let (head, sub) = (app.page.title(), app.page.subtitle());
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 1.0;
                            ui.label(RichText::new(head).size(17.0).color(p.text));
                            let room = (ui.available_width() - 6.0).max(24.0);
                            let sub = clip_to_width(&p, ui, sub, 11.0, room);
                            ui.label(RichText::new(sub).size(11.0).color(p.text_faint));
                        });
                    },
                );

                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    // 主题切换
                    theme_switch(ui, app, &p, &ctx);

                    ui.add_space(10.0);

                    // 权限状态
                    if app.admin {
                        widgets::pill(ui, sysutil::privilege_label(), PillKind::Safe, &p);
                    } else {
                        let resp = ui
                            .horizontal(|ui| widgets::pill(ui, "普通权限", PillKind::Caution, &p))
                            .response;
                        resp.on_hover_text(sysutil::elevation_hint());
                    }

                    ui.add_space(6.0);
                    disk_mini(ui, app, &p);
                });
            });
        });
}

/// 把一段文字裁进 `max_width` 里，超出部分换成省略号。
///
/// 横向布局里的 `Label` 不会换行，超长就直接压到旁边的控件上；顶栏的副标题
/// 是页面文案、长度不可控，所以先量一遍再决定截到哪个字。
fn clip_to_width(p: &Palette, ui: &egui::Ui, text: &str, size: f32, max_width: f32) -> String {
    let font = egui::FontId::proportional(size);
    let width = |candidate: &str| {
        ui.painter()
            .layout_no_wrap(candidate.to_owned(), font.clone(), p.text)
            .size()
            .x
    };
    if width(text) <= max_width {
        return text.to_string();
    }

    let chars: Vec<char> = text.chars().collect();
    for cut in (1..chars.len()).rev() {
        let candidate: String = chars[..cut].iter().collect::<String>() + "…";
        if width(&candidate) <= max_width {
            return candidate;
        }
    }
    "…".to_string()
}

/// 顶栏右上角的主题切换。
///
/// 只是一层「主题枚举 ↔ 序号」的映射，样式与点选逻辑全部交给
/// [`widgets::segmented`]，保证和设置页「外观」那一处长得完全一样。
fn theme_switch(ui: &mut egui::Ui, app: &mut App, p: &Palette, ctx: &egui::Context) {
    let labels: Vec<&str> = ThemePref::ALL.iter().map(|pref| pref.label()).collect();
    let selected = ThemePref::ALL
        .iter()
        .position(|pref| *pref == app.settings.theme)
        .unwrap_or(0);

    if let Some(index) = widgets::segmented(ui, p, &labels, selected, widgets::SEGMENTED_HEIGHT) {
        app.apply_theme(ThemePref::ALL[index], ctx);
    }
}

fn paint_logo(ui: &mut egui::Ui, p: &Palette, rect: Rect) {
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(10), p.accent);

    // 一个简化的"磁盘 + 光圈"标记
    let ring = Rect::from_center_size(rect.center(), Vec2::splat(16.0));
    painter.circle_stroke(ring.center(), 7.0, Stroke::new(2.0, Color32::WHITE));
    let dot = rect.center() + vec2(0.0, -0.5);
    painter.circle_filled(dot, 2.0, Color32::WHITE);
    // 在圆环右侧开一个缺口，形成 "C" 的意象
    let notch = Rect::from_min_size(
        egui::pos2(rect.center().x + 4.4, rect.center().y - 3.2),
        vec2(5.0, 6.4),
    );
    painter.rect_filled(notch, CornerRadius::same(1), p.accent);
}

// ---------------------------------------------------------------------------
// 品牌图标（assets/disk.png）
// ---------------------------------------------------------------------------

/// 顶栏品牌图标。`include_bytes!` 编进可执行文件，单文件分发不会丢图。
const LOGO_PNG: &[u8] = include_bytes!("../../assets/disk.png");

/// 取品牌图标贴图：第一次调用时解码 `assets/disk.png` 并上传成 GPU 贴图，之后一直复用。
///
/// 缓存放在 `App` 上而不是全局变量，是为了让贴图跟着 egui 上下文一起生灭 ——
/// wgpu 后端初始化失败时会换 Glow 再开一次，全局缓存那样会留下失效的句柄。
fn logo_texture(app: &mut App, ctx: &egui::Context) -> Option<egui::TextureHandle> {
    if app.logo_tex.is_none() {
        match decode_icon_png(LOGO_PNG) {
            Ok(image) => {
                app.logo_tex =
                    Some(ctx.load_texture("cleanc_logo", image, egui::TextureOptions::LINEAR));
            }
            Err(err) => {
                eprintln!("assets/disk.png 解码失败（{err}），顶栏改用内置绘制的图标");
            }
        }
    }
    app.logo_tex.clone()
}

/// 解码 PNG 并裁掉四周的透明留白，只留图形主体（外加 4px 呼吸边）。
///
/// 源图是 256×256、四周透明边很大的图标，直接整幅绘制的话有效图形
/// 只有六成大，还会带一片不可见区域干扰布局，所以这里先裁再上传。
///
/// 顶栏品牌图标与概览页的磁盘卡片共用这一套处理。
pub(crate) fn decode_icon_png(bytes: &[u8]) -> Result<egui::ColorImage, image::ImageError> {
    let rgba = image::load_from_memory_with_format(bytes, image::ImageFormat::Png)?.into_rgba8();
    let (width, height) = rgba.dimensions();
    let (width, height) = (width as usize, height as usize);
    let buf = rgba.into_raw();

    // 找出不透明像素的包围盒（alpha > 8 算可见）
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (width, height, 0usize, 0usize);
    for y in 0..height {
        for x in 0..width {
            if buf[(y * width + x) * 4 + 3] > 8 {
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }
        }
    }
    // 全透明图不该出现，兜底用整幅
    if max_x < min_x || max_y < min_y {
        return Ok(egui::ColorImage::from_rgba_unmultiplied(
            [width, height],
            &buf,
        ));
    }

    // 四周各留 4px 呼吸边，避免图形紧贴边缘
    let pad = 4;
    let x0 = min_x.saturating_sub(pad);
    let y0 = min_y.saturating_sub(pad);
    let x1 = (max_x + pad + 1).min(width);
    let y1 = (max_y + pad + 1).min(height);
    let (cw, ch) = (x1 - x0, y1 - y0);

    let mut cropped = Vec::with_capacity(cw * ch * 4);
    for y in y0..y1 {
        let start = (y * width + x0) * 4;
        cropped.extend_from_slice(&buf[start..start + cw * 4]);
    }
    Ok(egui::ColorImage::from_rgba_unmultiplied([cw, ch], &cropped))
}

fn disk_mini(ui: &mut egui::Ui, app: &App, p: &Palette) {
    let Some(disk) = &app.disk else {
        ui.label(
            RichText::new(format!(
                "{} 容量读取失败",
                sysutil::mount_label(&sysutil::default_mount())
            ))
            .size(11.5)
            .color(p.text_faint),
        );
        return;
    };

    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 3.0;
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let (rect, _) = ui.allocate_exact_size(vec2(120.0, 7.0), Sense::hover());
            let ratio = disk.used_ratio();
            let painter = ui.painter();
            painter.rect_filled(rect, CornerRadius::same(4), p.inset);
            if ratio > 0.0 {
                let w = (rect.width() * ratio).max(7.0);
                painter.rect_filled(
                    Rect::from_min_size(rect.min, vec2(w, rect.height())),
                    CornerRadius::same(4),
                    crate::theme::usage_color(p, ratio),
                );
            }
            ui.label(
                RichText::new(format!("{:.0}%", ratio * 100.0))
                    .size(12.0)
                    .strong()
                    .color(crate::theme::usage_color(p, ratio)),
            );
            ui.add_space(2.0);
            ui.label(
                RichText::new(format!(
                    "{} {} / {}",
                    sysutil::mount_label(&disk.mount),
                    human_bytes(disk.used()),
                    human_bytes(disk.total)
                ))
                .size(11.5)
                .color(p.text_weak),
            );
        });
    });
}

// ---------------------------------------------------------------------------
// 侧边导航
// ---------------------------------------------------------------------------

/// 侧栏宽度。
const NAV_WIDTH: f32 = 212.0;

/// 侧栏的分组结构。
///
/// 第一组用空标题表示「不加分组标签」，就是参考图里最上面那个独立的「概览」。
/// 分组只影响导航的排布，和页面本身没有耦合。
const NAV_GROUPS: &[(&str, &[Page])] = &[
    ("", &[Page::Overview]),
    ("专项清理", &[Page::Clean, Page::BigFiles]),
    ("应用", &[Page::Apps]),
    ("系统", &[Page::Settings, Page::Safety]),
];

pub fn side_nav(app: &mut App, ui: &mut egui::Ui) {
    let p = Palette::of(ui);

    egui::Panel::left("cleanc_nav")
        .exact_size(NAV_WIDTH)
        .resizable(false)
        .show_separator_line(false)
        .frame(
            // 侧栏底比主区**略深一点**，把导航区从内容里"退"出去；
            // 侧栏占满整个高度（在 `app.rs` 里它是最先声明的面板）。
            egui::Frame::new()
                .fill(p.nav_bg)
                .inner_margin(Margin::symmetric(12, 12)),
        )
        .show(ui, |ui| {
            brand(app, ui, &p);

            // ---- 页脚：「系统」这一组永远贴侧栏底部 ----
            //
            // 参考图把这一组单独压到底部，中间空出一大段，让"常用"和"其他"
            // 在视线上先分开。用一条分隔线给这段空白一个交代 —— 否则「系统」
            // 那块标签像是孤零零飘在侧栏中间。
            //
            // 实现上交给 egui 的**贴底面板**：面板高度按内容自适应，而且它会
            // 先把这块空间从父级扣掉，上面几组再顺着剩下的高度往下排。
            //
            // 之前是手算「可用高度 − 这一组的高度」再 add_space 沉底，而那个
            // "这一组的高度"漏算了 egui 在每个 widget 之后追加的 item_spacing
            // （本程序全局设成 8），于是「安全说明」压出侧栏下边距 14px、
            // 连窗口底都越过了 2px —— 这正是改这段的原因。
            if let Some((group, pages)) = NAV_GROUPS.last() {
                egui::Panel::bottom("cleanc_nav_footer")
                    .resizable(false)
                    .show_separator_line(false)
                    // 只换底色：左右和底部的留白由外层侧栏的 margin 统一给，
                    // 这一层要是再加 margin，上下两段留白就对不齐了。
                    .frame(egui::Frame::new().fill(p.nav_bg))
                    .show(ui, |ui| {
                        // 分隔线到标题之间**不加额外的留白** —— `nav_group_block`
                        // 里已经有一段统一的分组上留白，再叠一次会比别处厚一倍。
                        widgets::nav_separator(ui, &p);
                        nav_group_block(app, ui, &p, group, pages);
                    });
            }

            // ---- 上面几组：正常自上而下 ----
            for (group, pages) in &NAV_GROUPS[..NAV_GROUPS.len() - 1] {
                nav_group_block(app, ui, &p, group, pages);
            }

            // 版本、运行方式、日志目录这些"关于信息"统一收在「设置」页，
            // 侧栏只负责导航。
        });
}

/// 画一组导航：可选的小标题 + 若干条目。
///
/// 小标题上下的留白放在这里（而不是 `widgets::nav_group` 里），上面几组和
/// 贴底的页脚那一组都走这一条路径，留白就只有一份定义。
fn nav_group_block(app: &mut App, ui: &mut egui::Ui, p: &Palette, group: &str, pages: &[Page]) {
    if !group.is_empty() {
        ui.add_space(widgets::NAV_GROUP_TOP);
        widgets::nav_group(ui, p, group);
        ui.add_space(widgets::NAV_GROUP_UNDER);
    }
    for page in pages {
        let badge = nav_badge(app, *page);
        // 「全部结果」是从「大文件」跳进去的子页面，导航里仍然高亮「大文件」
        let selected =
            app.page == *page || (*page == Page::BigFiles && app.page == Page::BigDetail);
        let resp = widgets::nav_item(
            ui,
            p,
            nav_icon(*page),
            page.title(),
            selected,
            badge.as_deref(),
        );
        if resp.clicked() {
            app.page = *page;
        }
        ui.add_space(widgets::NAV_ITEM_GAP);
    }
}

/// 侧栏顶部的品牌区：图标 + 名字，只占一行。
///
/// 参考图里这块做得很轻（顶部基本只有一个小图标），所以也收成单行 ——
/// 侧栏的主角是导航，不是品牌。
fn brand(app: &mut App, ui: &mut egui::Ui, p: &Palette) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::hover());

    let logo = Rect::from_min_size(
        egui::pos2(rect.left() + 4.0, rect.center().y - 13.0),
        vec2(26.0, 26.0),
    );
    match logo_texture(app, ui.ctx()) {
        Some(tex) => {
            let size = tex.size_vec2();
            let scale = (logo.width() / size.x).min(logo.height() / size.y);
            egui::Image::new(&tex)
                .paint_at(ui, Rect::from_center_size(logo.center(), size * scale));
        }
        // 解码失败（几乎不可能，文件已编进可执行文件）才退回内置绘制
        None => paint_logo(ui, p, logo),
    }

    ui.painter().text(
        egui::pos2(logo.right() + 9.0, rect.center().y),
        Align2::LEFT_CENTER,
        "磁盘清理",
        egui::FontId::proportional(15.0),
        p.text,
    );
}

/// 导航项对应的图标。
fn nav_icon(page: Page) -> icons::Icon {
    match page {
        Page::Overview => icons::Icon::Overview,
        Page::Clean => icons::Icon::Clean,
        Page::Apps => icons::Icon::Apps,
        Page::BigFiles | Page::BigDetail => icons::Icon::BigFiles,
        Page::Settings => icons::Icon::Settings,
        Page::Safety => icons::Icon::Safety,
    }
}

fn nav_badge(app: &App, page: Page) -> Option<String> {
    match page {
        Page::Clean => {
            let (targets, _, bytes) = app.pending_selection();
            if bytes > 0 {
                let _ = targets;
                Some(human_bytes(bytes))
            } else {
                None
            }
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// 底部状态栏
// ---------------------------------------------------------------------------

pub fn status_bar(app: &mut App, ui: &mut egui::Ui) {
    let p = Palette::of(ui);

    egui::Panel::bottom("cleanc_status")
        .exact_size(42.0)
        .resizable(false)
        .show_separator_line(false)
        .frame(
            egui::Frame::new()
                .fill(p.bg)
                .inner_margin(Margin::symmetric(22, 8)),
        )
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                status_content(app, ui, &p);
            });
        });
}

fn status_content(app: &mut App, ui: &mut egui::Ui, p: &Palette) {
    let total_width = ui.available_width();

    // 左侧：当前操作
    let label = match &app.scan {
        ScanState::Running {
            current,
            finished,
            total,
            ..
        } => Some((
            format!(
                "正在扫描 {}/{}：{}",
                finished + 1,
                total,
                if current.is_empty() {
                    "准备中"
                } else {
                    current.as_str()
                }
            ),
            p.accent,
        )),
        _ => None,
    };

    let clean_label = match &app.clean {
        CleanState::Running {
            current,
            files,
            bytes,
            ..
        } => Some((
            format!(
                "正在清理：{} · 已处理 {} 个文件 / {}",
                current,
                thousands(*files),
                human_bytes(*bytes)
            ),
            p.danger,
        )),
        _ => None,
    };

    let big_label = match &app.big {
        BigState::Running {
            scanned, current, ..
        } => Some((
            format!(
                "正在查找大文件 · 已扫描 {} 个文件 · {}",
                thousands(*scanned),
                shorten(current)
            ),
            p.accent,
        )),
        _ => None,
    };

    let active = clean_label.or(label).or(big_label);

    if let Some((text, color)) = active {
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
            ui.painter().circle_filled(rect.center(), 4.0, color);
            ui.label(RichText::new(text).size(12.0).color(p.text));
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let (text, action) = if app.cleaning() {
                ("取消清理", 2u8)
            } else if app.scanning() {
                ("取消扫描", 1u8)
            } else {
                ("取消", 3u8)
            };
            if widgets::ghost_button(ui, p, text).clicked() {
                match action {
                    1 => app.cancel_scan(),
                    2 => app.cancel_clean(),
                    _ => app.cancel_big_scan(),
                }
            }
        });
    } else {
        let (hint, color) = idle_status(app, p);
        let _ = total_width;
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
            ui.painter().circle_filled(rect.center(), 4.0, color);
            ui.label(RichText::new(hint).size(12.0).color(p.text_weak));
        });

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if let Some(disk) = &app.disk {
                ui.label(
                    RichText::new(format!(
                        "{} 可用 {}",
                        sysutil::mount_label(&disk.mount),
                        human_bytes(disk.free)
                    ))
                    .size(11.5)
                    .color(p.text_faint),
                );
            }
        });
    }
}

fn idle_status(app: &App, p: &Palette) -> (String, Color32) {
    if let Some(summary) = app.summary() {
        let (chosen, _, chosen_bytes) = app.pending_selection();
        return (
            format!(
                "扫描完成（用时 {:.1}s）：共 {} 个文件 / {}，勾选 {} 项，预计释放 {}",
                summary.elapsed_ms as f32 / 1000.0,
                thousands(summary.total_files()),
                human_bytes(summary.total_bytes()),
                chosen.len(),
                human_bytes(chosen_bytes)
            ),
            p.safe,
        );
    }
    (
        "就绪 · 建议先到「智能清理」执行一次扫描，看清每一类会删掉什么再动手".to_string(),
        p.text_faint,
    )
}

fn shorten(text: &str) -> String {
    let limit = 68;
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let tail: String = text
        .chars()
        .rev()
        .take(limit - 3)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("…{tail}")
}

// ---------------------------------------------------------------------------
// 浮层提示
// ---------------------------------------------------------------------------

pub fn toasts(app: &mut App, ctx: &egui::Context) {
    if app.toasts.is_empty() {
        return;
    }
    let p = Palette::of_ctx(ctx);
    let mut expired = false;

    egui::Area::new(egui::Id::new("cleanc_toasts"))
        .anchor(Align2::RIGHT_BOTTOM, vec2(-20.0, -52.0))
        .order(egui::Order::Foreground)
        .interactable(false)
        .show(ctx, |ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            for toast in app.toasts.iter().rev() {
                let (bg, fg) = match toast.kind {
                    ToastKind::Info => (p.surface, p.text),
                    ToastKind::Success => (p.safe_soft, p.safe),
                    ToastKind::Warn => (p.caution_soft, p.caution),
                    ToastKind::Error => (p.danger_soft, p.danger),
                };
                egui::Frame::new()
                    .fill(bg)
                    .stroke(Stroke::new(1.0, fg.gamma_multiply(0.35)))
                    .corner_radius(CornerRadius::same(crate::theme::CARD_RADIUS))
                    .inner_margin(Margin::symmetric(14, 10))
                    .shadow(egui::epaint::Shadow {
                        offset: [0, 4],
                        blur: 16,
                        spread: 0,
                        color: Color32::from_black_alpha(40),
                    })
                    .show(ui, |ui| {
                        ui.set_max_width(420.0);
                        ui.horizontal(|ui| {
                            let (rect, _) = ui.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
                            ui.painter().circle_filled(rect.center(), 4.0, fg);
                            ui.add_space(2.0);
                            ui.label(RichText::new(&toast.text).size(12.5).color(fg));
                        });
                    });
            }
            if app
                .toasts
                .iter()
                .any(|t| t.at.elapsed().as_secs_f32() > 12.0)
            {
                expired = true;
            }
        });

    if expired {
        app.toasts.clear();
    }
}
