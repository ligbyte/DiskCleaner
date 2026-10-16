//! 复用型 UI 组件。

use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, FontId, Layout, Margin, Rect, Response, RichText,
    Sense, Stroke, Ui, Vec2, vec2,
};

use crate::model::{RiskLevel, human_bytes, thousands};
use crate::theme::{Palette, PillKind, pill_colors};
use crate::ui::icons;

/// 胶囊标签。
pub fn pill(ui: &mut Ui, text: &str, kind: PillKind, p: &Palette) {
    let (bg, fg) = pill_colors(p, kind);
    pill_colored(ui, text, bg, fg);
}

pub fn pill_colored(ui: &mut Ui, text: &str, bg: Color32, fg: Color32) {
    egui::Frame::new()
        .fill(bg)
        .corner_radius(CornerRadius::same(crate::theme::CARD_RADIUS))
        .inner_margin(Margin::symmetric(8, 3))
        .show(ui, |ui| {
            ui.label(RichText::new(text).size(11.0).color(fg).strong());
        });
}

/// 风险等级标签。
pub fn risk_pill(ui: &mut Ui, risk: RiskLevel, p: &Palette) -> Response {
    let kind = match risk {
        RiskLevel::Safe => PillKind::Safe,
        RiskLevel::Caution => PillKind::Caution,
        RiskLevel::Advanced => PillKind::Danger,
    };
    let (bg, fg) = pill_colors(p, kind);
    egui::Frame::new()
        .fill(bg)
        .corner_radius(CornerRadius::same(crate::theme::CARD_RADIUS))
        .inner_margin(Margin::symmetric(8, 3))
        .show(ui, |ui| {
            ui.label(RichText::new(risk.label()).size(11.0).color(fg).strong());
        })
        .response
}

/// 体积数字（右对齐的大号数字）。
pub fn size_text(bytes: u64, p: &Palette) -> RichText {
    RichText::new(human_bytes(bytes))
        .size(17.0)
        .color(p.text)
        .strong()
}

/// 大文件/大目录的处置建议对应的胶囊样式。
pub fn advice_pill(kind: crate::advice::Kind) -> PillKind {
    use crate::advice::Kind;
    match kind {
        // 能重建 / 一次性的东西，放心清
        Kind::Cache | Kind::Installer => PillKind::Safe,
        // 删了不好找回，先确认
        Kind::Media => PillKind::Caution,
        // 碰了可能出问题
        Kind::System => PillKind::Danger,
        // 无法判断，交给用户
        Kind::Personal => PillKind::Neutral,
    }
}

/// 处置建议对应的强调色。
pub fn advice_color(kind: crate::advice::Kind, p: &Palette) -> Color32 {
    match advice_pill(kind) {
        PillKind::Safe => p.safe,
        PillKind::Caution => p.caution,
        PillKind::Danger => p.danger,
        PillKind::Accent => p.accent,
        PillKind::Neutral => p.text_weak,
    }
}

/// 卡片容器。
pub fn card<R>(ui: &mut Ui, p: &Palette, add: impl FnOnce(&mut Ui) -> R) -> R {
    crate::theme::card_frame(p).show(ui, add).inner
}

/// 次级分区容器。
pub fn section<R>(ui: &mut Ui, p: &Palette, add: impl FnOnce(&mut Ui) -> R) -> R {
    crate::theme::section_frame(p).show(ui, add).inner
}

/// 小节标题。
pub fn section_title(ui: &mut Ui, p: &Palette, text: &str) {
    ui.label(RichText::new(text).size(14.0).strong().color(p.text));
}

/// 主按钮。
pub fn primary_button(ui: &mut Ui, p: &Palette, text: &str, enabled: bool) -> Response {
    let btn = egui::Button::new(RichText::new(text).size(13.5).strong().color(if enabled {
        Color32::WHITE
    } else {
        p.text_faint
    }))
    .fill(if enabled { p.accent } else { p.inset })
    .corner_radius(CornerRadius::same(crate::theme::CARD_RADIUS))
    .min_size(vec2(0.0, 32.0));

    let resp = ui.add_enabled(enabled, btn);
    if enabled && resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

/// 次按钮（描边）。
pub fn ghost_button(ui: &mut Ui, p: &Palette, text: &str) -> Response {
    let btn = egui::Button::new(RichText::new(text).size(13.0).color(p.text))
        .fill(p.surface)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(crate::theme::CARD_RADIUS))
        .min_size(vec2(0.0, 32.0));
    let resp = ui.add(btn);
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

/// 危险按钮（红底）。
pub fn danger_button(ui: &mut Ui, p: &Palette, text: &str, enabled: bool) -> Response {
    let btn = egui::Button::new(RichText::new(text).size(13.5).strong().color(if enabled {
        Color32::WHITE
    } else {
        p.text_faint
    }))
    .fill(if enabled { p.danger } else { p.inset })
    .corner_radius(CornerRadius::same(crate::theme::CARD_RADIUS))
    .min_size(vec2(0.0, 32.0));
    let resp = ui.add_enabled(enabled, btn);
    if enabled && resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

/// 进度条（自绘，圆角更统一）。
pub fn progress_bar(ui: &mut Ui, p: &Palette, ratio: f32, height: f32, color: Color32) {
    let width = ui.available_width().max(40.0);
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let radius = CornerRadius::same((height / 2.0).round() as u8);
    let painter = ui.painter();
    painter.rect_filled(rect, radius, p.inset);

    let ratio = ratio.clamp(0.0, 1.0);
    if ratio > 0.0 {
        let width = (rect.width() * ratio).max(height.min(rect.width()));
        let fill = Rect::from_min_size(rect.min, vec2(width, height));
        painter.rect_filled(fill, radius, color);
    }
}

/// 磁盘占用条。
pub fn usage_bar(ui: &mut Ui, p: &Palette, ratio: f32, height: f32) {
    progress_bar(ui, p, ratio, height, crate::theme::usage_color(p, ratio));
}

/// 键值行。
pub fn kv_row(ui: &mut Ui, p: &Palette, key: &str, value: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(key).size(12.0).color(p.text_weak));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(RichText::new(value).size(12.0).color(p.text));
        });
    });
}

/// 一个导航条目的高度。
pub const NAV_ITEM_HEIGHT: f32 = 34.0;

/// 相邻导航条目之间**额外**留的空隙。
///
/// 注意 egui 在每个 widget 之后还会再加一个 `spacing.item_spacing`（本程序全局
/// 设成了 8），所以两条之间实际看到的是 8 + 这个值，不是单独这个值。
pub const NAV_ITEM_GAP: f32 = 2.0;

/// 分组小标题自身的高度。
pub const NAV_GROUP_LABEL_H: f32 = 20.0;

/// 分组小标题**上方**的呼吸位（把它和上一组内容拉开）。
pub const NAV_GROUP_TOP: f32 = 12.0;

/// 分组小标题与它下面第一条导航之间的补偿间距。
pub const NAV_GROUP_UNDER: f32 = 2.0;

/// 侧栏图标的边长，以及图标距导航条左边缘的距离。
///
/// 分组标题的文字起点**共用**这个左偏移 —— 两者对齐之后，整条侧栏的
/// 图标列才是一根竖线。
pub const NAV_ICON_SIDE: f32 = 17.0;
pub const NAV_ICON_LEFT: f32 = 13.0;

/// 侧栏导航项：图标 + 文字。
///
/// 选中态是**一块比侧栏底更浅的胶囊**（参考图的做法），文字与图标转成品牌绿。
/// 不靠底色深浅来强调，因为侧栏底本身就很浅，把选中项做成深色块会很突兀。
pub fn nav_item(
    ui: &mut Ui,
    p: &Palette,
    icon: icons::Icon,
    label: &str,
    selected: bool,
    badge: Option<&str>,
) -> Response {
    let width = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(width, NAV_ITEM_HEIGHT), Sense::click());

    let hovered = resp.hovered();
    let painter = ui.painter();

    if selected {
        painter.rect_filled(
            rect,
            CornerRadius::same(crate::theme::CARD_RADIUS),
            p.nav_pill,
        );
    } else if hovered {
        painter.rect_filled(rect, CornerRadius::same(crate::theme::CARD_RADIUS), p.hover);
    }

    // ---- 图标 ----
    let icon_rect = Rect::from_min_size(
        egui::pos2(
            rect.left() + NAV_ICON_LEFT,
            rect.center().y - NAV_ICON_SIDE / 2.0,
        ),
        Vec2::splat(NAV_ICON_SIDE),
    );
    icons::paint(
        painter,
        icon_rect,
        icon,
        if selected { p.accent_ink } else { p.text_faint },
        if selected { 1.7 } else { 1.45 },
    );

    // ---- 文字 ----
    painter.text(
        egui::pos2(
            rect.left() + NAV_ICON_LEFT + NAV_ICON_SIDE + 11.0,
            rect.center().y,
        ),
        egui::Align2::LEFT_CENTER,
        label,
        FontId::proportional(13.5),
        if selected { p.accent_ink } else { p.text_weak },
    );

    // ---- 右侧数字（例如「智能清理」待清理的体积）----
    if let Some(badge) = badge {
        painter.text(
            egui::pos2(rect.right() - 10.0, rect.center().y),
            egui::Align2::RIGHT_CENTER,
            badge,
            FontId::proportional(11.0),
            if selected { p.accent_ink } else { p.text_faint },
        );
    }

    if hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

/// 侧栏里的分组小标题（「专项清理」「系统」这类）。
///
/// **只画标题本身，不管上下留白** —— 留白交给调用方（`chrome::side_nav`）。
/// 之前这里自带 12px 上留白 + 2px 下留白，结果另一个人想算"这一组占多高"时
/// 得把这几个数一起抄一遍，抄漏一个就会把内容顶出侧栏。现在留白的数只有一份。
pub fn nav_group(ui: &mut Ui, p: &Palette, text: &str) {
    let (rect, _) = ui.allocate_exact_size(
        vec2(ui.available_width(), NAV_GROUP_LABEL_H),
        Sense::hover(),
    );
    ui.painter().text(
        egui::pos2(rect.left() + NAV_ICON_LEFT, rect.center().y),
        egui::Align2::LEFT_CENTER,
        text,
        FontId::proportional(11.0),
        p.text_faint,
    );
}

/// 侧栏页脚区顶部的分隔线。
///
/// 两端各内缩到图标列的位置：比"一条通到底"更像分界线，也顺势和图标列
/// 对齐成一根竖线。
pub fn nav_separator(ui: &mut Ui, p: &Palette) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    let line = Rect::from_min_max(
        egui::pos2(rect.left() + NAV_ICON_LEFT, rect.top()),
        egui::pos2(rect.right() - NAV_ICON_LEFT, rect.bottom()),
    );
    ui.painter()
        .rect_filled(line, CornerRadius::ZERO, p.nav_line);
}

/// 概览页的大号胶囊主按钮（参考图里那颗「一键清理」）。
///
/// 宽度跟着「图标 + 文字」走，上下用一点渐变，避免大块纯色显得很平。
/// `icon` 给 `None` 就是纯文字按钮。
pub fn hero_button(
    ui: &mut Ui,
    p: &Palette,
    icon: Option<icons::Icon>,
    text: &str,
    enabled: bool,
) -> Response {
    /// 按钮高度 —— 参考图里它明显比普通按钮粗壮
    const HEIGHT: f32 = 52.0;
    /// 文字左右各留的空白
    const SIDE: f32 = 46.0;
    /// 图标边长
    const ICON_SIDE: f32 = 19.0;
    /// 图标与文字之间
    const ICON_GAP: f32 = 9.0;

    let font = FontId::proportional(16.0);
    let text_width = ui
        .painter()
        .layout_no_wrap(text.to_owned(), font.clone(), Color32::WHITE)
        .size()
        .x;
    let icon_width = if icon.is_some() {
        ICON_SIDE + ICON_GAP
    } else {
        0.0
    };
    let (rect, resp) = ui.allocate_exact_size(
        vec2(text_width + icon_width + SIDE * 2.0, HEIGHT),
        Sense::click(),
    );

    let radius = HEIGHT / 2.0;
    let painter = ui.painter();

    if enabled {
        let (top, bottom) = crate::theme::accent_gradient(p);
        // 悬停时整体提亮一档
        let lift = if resp.hovered() { 0.10 } else { 0.0 };
        crate::theme::paint_vertical_gradient(
            painter,
            rect,
            radius,
            crate::theme::mix(top, Color32::WHITE, lift),
            crate::theme::mix(bottom, Color32::WHITE, lift),
        );
    } else {
        painter.rect_filled(rect, CornerRadius::same(radius as u8), p.inset);
    }

    let fg = if enabled {
        Color32::WHITE
    } else {
        p.text_faint
    };

    // 图标 + 文字作为一个整体居中，单独居中文字会让加图标的那一版偏沉
    let content_w = text_width + icon_width;
    let left = rect.center().x - content_w / 2.0;
    if let Some(icon) = icon {
        let icon_rect = Rect::from_min_size(
            egui::pos2(left, rect.center().y - ICON_SIDE / 2.0),
            Vec2::splat(ICON_SIDE),
        );
        icons::paint(painter, icon_rect, icon, fg, 1.9);
    }
    painter.text(
        egui::pos2(left + icon_width, rect.center().y),
        egui::Align2::LEFT_CENTER,
        text,
        font,
        fg,
    );

    if enabled && resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

/// 概览页的「快捷入口」：左边一个圆角色块图标，右边两行文字。
///
/// 参考图里这一排入口**没有卡片边框**，直接浮在底色上，所以平时也不画底，
/// 只有悬停时才浮出一层浅底。没有边框的排布看起来比一排卡片轻得多。
pub fn quick_entry(
    ui: &mut Ui,
    p: &Palette,
    icon: icons::Icon,
    tint: Color32,
    title: &str,
    sub: &str,
) -> Response {
    /// 整块的高度
    const HEIGHT: f32 = 58.0;
    /// 图标色块的边长
    const CHIP: f32 = 38.0;

    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), HEIGHT), Sense::click());
    let painter = ui.painter();

    if resp.hovered() {
        painter.rect_filled(
            Rect::from_min_max(rect.min - vec2(8.0, 4.0), rect.max + vec2(8.0, 4.0)),
            CornerRadius::same(crate::theme::CARD_RADIUS),
            p.hover,
        );
    }

    // ---- 图标色块 ----
    let chip = Rect::from_min_size(
        egui::pos2(rect.left(), rect.center().y - CHIP / 2.0),
        Vec2::splat(CHIP),
    );
    icons::paint_chip(
        painter,
        chip,
        icon,
        // 掺在**卡片底色**里的浅色调，而不是单纯压低透明度 ——
        // 深色主题下也能得到一块"淡淡的同色底"。
        crate::theme::mix(p.surface, tint, 0.16),
        tint,
        crate::theme::CARD_RADIUS as f32 + 5.0,
    );

    // ---- 两行文字 ----
    let text_left = chip.right() + 12.0;
    painter.text(
        egui::pos2(text_left, rect.center().y - 9.0),
        egui::Align2::LEFT_CENTER,
        title,
        FontId::proportional(13.5),
        p.text,
    );
    painter.text(
        egui::pos2(text_left, rect.center().y + 10.0),
        egui::Align2::LEFT_CENTER,
        sub,
        FontId::proportional(11.5),
        p.text_faint,
    );

    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

/// 一行"安全提示"。
pub fn bullet(ui: &mut Ui, p: &Palette, color: Color32, text: &str) {
    ui.horizontal_top(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(6.0, 18.0), Sense::hover());
        let dot = Rect::from_center_size(rect.center() + vec2(3.0, 0.0), Vec2::splat(6.0));
        ui.painter().circle_filled(dot.center(), 3.0, color);
        ui.add_space(4.0);
        ui.label(RichText::new(text).size(12.5).color(p.text_weak));
    });
}

/// 主题切换那种分段控件的默认高度。
///
/// 顶栏与设置页共用，避免同一件事在两处长成两个高度。
pub const SEGMENTED_HEIGHT: f32 = 39.0;

/// 分段控件：浅底圆角容器 + 选中项浮在里面的圆角块。
///
/// 返回被点中的那一项的序号，这一帧没点就返回 `None`。
/// 顶栏的主题切换与设置页的「外观」用的是同一个控件，样式才不会两处跑偏。
///
/// * 各段**等宽** —— 按最宽的那个标签定宽，选中块滑动时不会忽大忽小；
/// * 整体自绘，而不是拿一排 `Button` 去拼：按钮自带描边与间距，拼出来
///   总比设计稿"松"一圈。
pub fn segmented(
    ui: &mut Ui,
    p: &Palette,
    labels: &[&str],
    selected: usize,
    height: f32,
) -> Option<usize> {
    if labels.is_empty() {
        return None;
    }

    /// 容器内边距 —— 选中块就浮在这圈留白里
    const PAD: f32 = 4.0;
    /// 每段文字左右各留的空白
    const SIDE: f32 = 15.0;

    // 字号跟着高度走：把控件拉高时文字不会显得空在中间
    let font = FontId::proportional((height * 0.33).clamp(11.0, 15.0));

    // 先量一遍标签，各段统一取最宽的那个宽度
    let widest = {
        let painter = ui.painter();
        labels
            .iter()
            .map(|text| {
                painter
                    .layout_no_wrap((*text).to_owned(), font.clone(), Color32::BLACK)
                    .size()
                    .x
            })
            .fold(0.0_f32, f32::max)
    };
    let seg_w = widest + SIDE * 2.0;
    let size = vec2(seg_w * labels.len() as f32 + PAD * 2.0, height);

    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let selected = selected.min(labels.len() - 1);

    let painter = ui.painter();

    // 容器
    painter.rect_filled(
        rect,
        CornerRadius::same((height / 2.0).round() as u8),
        p.inset,
    );

    // 选中的那一段
    let inner_h = height - PAD * 2.0;
    painter.rect_filled(
        Rect::from_min_size(
            egui::pos2(rect.min.x + PAD + seg_w * selected as f32, rect.min.y + PAD),
            vec2(seg_w, inner_h),
        ),
        CornerRadius::same((inner_h / 2.0).round() as u8),
        p.surface,
    );

    // 文字
    for (i, text) in labels.iter().enumerate() {
        painter.text(
            egui::pos2(rect.min.x + PAD + seg_w * (i as f32 + 0.5), rect.center().y),
            Align2::CENTER_CENTER,
            *text,
            font.clone(),
            if i == selected { p.text } else { p.text_weak },
        );
    }

    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }

    // 点在哪段就是哪段
    response
        .interact_pointer_pos()
        .filter(|_| response.clicked())
        .map(|pos| {
            let offset = (pos.x - rect.min.x - PAD).max(0.0);
            ((offset / seg_w) as usize).min(labels.len() - 1)
        })
}

/// 简单的分隔线。
pub fn divider(ui: &mut Ui, p: &Palette) {
    ui.add_space(6.0);
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(width, 1.0), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::ZERO, p.border);
    ui.add_space(6.0);
}

/// 计数文本。
pub fn count_pair(files: u64, dirs: u64) -> String {
    if dirs > 0 {
        format!("{} 个文件 · {} 个目录", thousands(files), thousands(dirs))
    } else {
        format!("{} 个文件", thousands(files))
    }
}
