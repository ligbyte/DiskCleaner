//! 垃圾文件页（原「智能清理」）：选类别、扫描、看明细、执行清理。
//!
//! 整页的节奏照抄参考图（360清理Pro 的「一键清理」页，空闲态 + 扫描中态两张）：
//!
//! ```text
//!   一键清理 12.4 GB                          ┌─────────┐
//!   流畅体验如初、如新！                        │  插画   │
//!   [盘] ▓▓▓▓▓▓▓▓░░░░  C: 可用 58.7 GB  共 200.0 GB
//!
//!   ┌ 用户临时文件 ─┐ ┌ 系统文件与更新 ┐ ┌ 浏览器缓存 ─┐ ┌ 应用/显卡缓存 ┐
//!   │ 描述         √│ │ 描述          √│ │ 描述       √│ │ 描述         √│
//!   │ 扫描 >        │ │ 扫描 >         │ │ 扫描 >      │ │ 扫描 >        │
//!   └───────────────┘ └────────────────┘ └─────────────┘ └───────────────┘
//!
//!                        （ ● ）
//!                         扫描          ← 大圆按钮，扫描中变成「7 %」
//!              点击 取消扫描 可重新选择扫描项
//! ```
//!
//! 两处刻意的取舍：
//!
//! 1. **卡片行严格四张**。本程序有五个清理分组，参考图只有四张卡，所以把
//!    「高级项（默认关闭）」并进「系统文件与更新」那一张（见 [`CARDS`]）。
//!    分组只是这一页的信息架构，不是安全边界 —— 高级项在明细里仍然是独立开关、
//!    仍然默认关闭，一个都没少。
//! 2. **逐项明细收进卡片的「扫描 >」**。参考图的卡片行以下就结束了，本程序需要
//!    看到每个清理项，所以点某张卡的「扫描 >」把那一组的明细就地展开在卡片行下面
//!    （复用 `render_groups`，靠 `app.group_filter` 只渲染这一组）。

use std::path::PathBuf;

use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, FontId, Layout, Rect, RichText, Sense, Stroke,
    Vec2, pos2, vec2,
};

use crate::app::{App, CleanState, ScanState};
use crate::model::{Group, TargetStatus, human_bytes, thousands};
use crate::theme::{Palette, PillKind, mix};
use crate::ui::widgets::{self, card, pill, risk_pill};

// ---------------------------------------------------------------------------
// 参考图量出来的尺寸
//
// 参考图是 920×580，页内容横跨 x100..822（宽 722）；本程序内容区横跨
// x234..1107（宽 873，= 1120 − 212 侧栏 − 22 左边距 − 10 右边距 − 12 滚动条过道）。
// 两者都是"内容铺满、左右对称留白"，所以**几何尺寸统一按内容宽比例 873/722 ≈ 1.209
// 换算**，照抄参考图的原始像素值反而会让元素在更宽的版面里显得偏小（大圆就是典型的
// 例子：108 在参考图里占内容宽 15.0%，照抄只有 12.4%）。
// 字号是唯一例外 —— 参考图本身是缩放过的截图，小字号估不准，只在它基础上微调。
//
// 逐像素量到的参考值（ref1 = 空闲态，ref2 = 扫描中，两张的版式完全一致）：
//   卡片行   4 张，x100..270 / 284..454 / 468..638 / 652..822 → 宽 170、间隙 14
//   卡片     上下 y300..383 → 高 84，底色 #E8EFF8（比页面底 #DFE8F7 只亮一点点）
//   勾选框   24×24，离卡片右边缘 / 上边缘各 8px，纯 #1DD594
//   进度条   y242..246，填充 x143..254
//   大圆     直径 108，圆心 (460.5, 471.5) —— 两张图的位置、尺寸一模一样
// ---------------------------------------------------------------------------

/// 第一行大标题。参考图里 CJK 字宽 ≈ 27.5px，按内容宽比例换算到本程序约 33px。
const HERO_TITLE: f32 = 33.0;
/// 第二行副标题（参考图 ≈ 23px，等比放大一点但不跟到 1.19，免得两行差不多大）
const HERO_SUB: f32 = 26.0;
/// 插画边长。
///
/// 和概览页的 `HERO_ART_SIZE` 取同一个值（220）—— 两个页面的右上角是同一张
/// disk1.png，尺寸不一致会明显看出"换页之后图变小了"。版面高度同样是这一档：
/// 两页的中心面板上边距一致，插画方块一样大，落点就自然对齐了。
const HERO_ART: f32 = 220.0;

/// 卡片行：卡片之间的横向间隙（参考图实测 14px）
const CARD_GAP: f32 = 17.0;
/// 卡片高度。
///
/// 参考图逐像素量出来是 y300..383 —— **84px**，不是先前估的 108：
/// 卡片底边在 y383 就结束了（y384 起整行回到页面底色 #E2EAF5）。
/// 84 × 1.209 ≈ 101.6，取 104 让「标题 / 说明 / 扫描」三行还有呼吸位。
const CARD_H: f32 = 104.0;
/// 卡片圆角（参考图 ≈10px，×1.209 ≈ 12）
const CARD_RADIUS: u8 = 12;
/// 卡片左右内边距（参考图里标题离卡片左边缘 18px）
const CARD_PAD: f32 = 22.0;
/// 右上角绿色勾选框的边长（参考图实测 24×24）
const CHECK_SIDE: f32 = 29.0;
/// 勾选框离卡片右上角的距离
const CHECK_INSET: f32 = 10.0;

/// 大圆按钮直径（参考图实测 108px）。
///
/// 参考图内容宽 722，本程序 873，所以这里的 108 不能照抄 —— 照抄会让大圆在
/// 更宽的版面里显得偏小（108/873 = 12.4%，参考图是 108/722 = 15.0%）。
/// 108 × 1.209 ≈ 130.6。
const CIRCLE: f32 = 130.0;

/// 进度条：轨道宽 158×7、圆角 3.5（参考图实测）。
/// 宽度按内容宽比例放到 190，否则在更宽的内容区里显得太短。
const BAR_W: f32 = 190.0;
const BAR_H: f32 = 7.0;
/// 进度行左侧那个"盘"图标的尺寸
const DRIVE_W: f32 = 38.0;
const DRIVE_H: f32 = 28.0;

/// 卡片行严格四张 —— 参考图就是四张。
struct CardSpec {
    /// 这张卡对应的主分组
    group: Group,
    /// 一并纳入的分组（只有「系统文件与更新」那张有）
    extra: Option<Group>,
    /// 卡片标题
    title: &'static str,
    /// 一行说明。照抄各分组里**实际**清理的东西，别写"深度清理释放空间"这种空话。
    desc: &'static str,
}

const CARDS: [CardSpec; 4] = [
    CardSpec {
        group: Group::UserFiles,
        extra: None,
        title: "用户临时文件",
        desc: "临时文件、缩略图、回收站",
    },
    CardSpec {
        group: Group::SystemFiles,
        extra: Some(Group::Advanced),
        title: "系统文件与更新",
        desc: "更新缓存、日志、预读取",
    },
    CardSpec {
        group: Group::Browsers,
        extra: None,
        title: "浏览器缓存",
        desc: "Chrome、Edge、Firefox",
    },
    CardSpec {
        group: Group::Apps,
        extra: None,
        title: "应用 / 显卡缓存",
        desc: "着色器缓存、崩溃转储",
    },
];

/// 每帧收集的交互动作，避免在遍历 targets 时可变借用 app。
enum Action {
    Toggle(String),
    ToggleExpand(String),
    GroupAll(Group, bool),
    /// 卡片右上角的勾选框：一次勾选 / 取消这一张卡覆盖的（一到两个）分组
    CheckCard(Group, Option<Group>, bool),
    /// 卡片上的「扫描 >」：展开这一组的明细；还没扫过就顺手起一次扫描
    OpenGroup(Group),
    /// 全选有内容的（Shift 点任一勾选框的快捷方式）
    SelectAllWithContent,
    Scan,
    Cancel,
    Clean,
    Reveal(PathBuf),
}

/// 一张卡当前该显示什么。
enum CardPhase {
    /// 空闲（还没扫过）：显示这一组清理什么
    Idle,
    /// 扫描中：这一组还没轮到
    Waiting,
    /// 扫描中：这一组正在扫
    Busy,
    /// 扫完了：这一组能腾出多少
    Done { bytes: u64, files: u64 },
}

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let p = Palette::of(ui);
    let mut actions: Vec<Action> = Vec::new();

    clean_report_banner(app, ui, &p);

    hero(app, ui, &p);
    ui.add_space(14.0);
    progress_row(app, ui, &p);
    ui.add_space(22.0);

    card_row(app, ui, &p, &mut actions);
    ui.add_space(10.0);
    big_button(app, ui, &p, &mut actions);
    hint_row(app, ui, &p, &mut actions);

    // 卡片上「扫描 >」展开的那一组明细，挂在卡片行下面。
    // 复用 render_groups 的过滤逻辑：把 group_filter 设成这一组就只渲染它。
    if let Some(group) = app.group_filter {
        ui.add_space(22.0);
        detail_header(app, ui, &p, group, &mut actions);
        ui.add_space(8.0);
        render_groups(app, ui, &p, &mut actions);
    }

    // ---- 应用动作 ----
    for action in actions {
        match action {
            Action::Toggle(id) => app.toggle_target(&id),
            Action::ToggleExpand(id) => {
                if app.expanded.contains(&id) {
                    app.expanded.remove(&id);
                } else {
                    app.expanded.insert(id);
                }
            }
            Action::GroupAll(group, checked) => app.set_group_selection(group, checked),
            Action::CheckCard(group, extra, checked) => {
                app.set_group_selection(group, checked);
                if let Some(extra) = extra {
                    app.set_group_selection(extra, checked);
                }
            }
            Action::OpenGroup(group) => {
                if app.group_filter == Some(group) {
                    app.group_filter = None;
                } else {
                    app.group_filter = Some(group);
                    // 明细里全是「未扫描」的话点开来没意义，顺手起一次扫描
                    if !app.scanning() && app.summary().is_none() {
                        app.start_scan();
                    }
                }
            }
            Action::SelectAllWithContent => app.select_card_row(true),
            Action::Scan => app.start_scan(),
            Action::Cancel => app.cancel_scan(),
            Action::Clean => app.request_clean(),
            Action::Reveal(path) => crate::sysutil::reveal_in_file_manager(&path),
        }
    }
}

// ---------------------------------------------------------------------------
// 顶部：两行大标题 + 右侧插画
// ---------------------------------------------------------------------------

/// 扫描中那一行小字显示当前正在读哪个文件。
fn current_file(app: &App) -> String {
    match &app.scan {
        ScanState::Running { current, .. } if !current.is_empty() => current.clone(),
        ScanState::Running { .. } => "正在准备…".to_string(),
        _ => String::new(),
    }
}

fn hero(app: &mut App, ui: &mut egui::Ui, p: &Palette) {
    let art = crate::ui::overview::disk_texture(app, ui.ctx());
    let scanning = app.scanning();
    let scanned = app.summary().is_some();
    let (_, _, bytes) = app.pending_selection();

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let text_w = (ui.available_width() - HERO_ART - 20.0).max(240.0);

        ui.allocate_ui_with_layout(
            vec2(text_w, HERO_ART),
            Layout::top_down(Align::LEFT),
            |ui| {
                ui.add_space(34.0);
                // 第一行：扫描中换文案，扫完把能腾出的体积直接写进标题
                // （参考图就是「一键清理50G」这个写法）
                let line1 = if scanning {
                    "正在扫描中…".to_string()
                } else if scanned {
                    format!("一键清理{}", human_bytes(bytes))
                } else {
                    "一键清理".to_string()
                };
                ui.label(RichText::new(line1).size(HERO_TITLE).strong().color(p.text));
                ui.add_space(4.0);

                // 第二行：平时是那句口号，扫描中换成"正在读哪个文件"
                if scanning {
                    let file = current_file(app);
                    if !file.is_empty() {
                        ui.label(
                            RichText::new(clip_middle(&file, 46))
                                .size(12.5)
                                .color(p.text_weak),
                        );
                    }
                } else {
                    ui.label(
                        RichText::new("流畅体验如初、如新！")
                            .size(HERO_SUB)
                            .strong()
                            .color(p.text),
                    );
                }
            },
        );

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| match &art {
            Some(tex) => {
                let (rect, _) = ui.allocate_exact_size(vec2(HERO_ART, HERO_ART), Sense::hover());
                // 裁剪后的素材**不是正方形**（原 256×256 裁掉透明边后是一张宽扁的
                // 磁盘图），直接铺满方块会横向压扁。按真实宽高比等比缩放、在方块
                // 里居中 —— 和概览页 Hero 的画法保持一致。
                let size = tex.size_vec2();
                let scale = (rect.width() / size.x).min(rect.height() / size.y);
                egui::Image::new(tex)
                    .paint_at(ui, Rect::from_center_size(rect.center(), size * scale));
            }
            // 素材解码失败的兜底：画一块圆角方块，不至于开天窗
            None => {
                let (rect, _) = ui.allocate_exact_size(vec2(HERO_ART, HERO_ART), Sense::hover());
                ui.painter().rect_filled(
                    Rect::from_center_size(rect.center(), Vec2::splat(HERO_ART * 0.62)),
                    CornerRadius::same(20),
                    p.inset,
                );
            }
        });
    });
}

/// 路径太长会把标题行撑爆，中间截断。
fn clip_middle(text: &str, max_chars: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max_chars {
        return text.to_string();
    }
    let keep = max_chars.saturating_sub(1) / 2;
    let head: String = chars[..keep].iter().collect();
    let tail: String = chars[chars.len() - keep..].iter().collect();
    format!("{head}…{tail}")
}

// ---------------------------------------------------------------------------
// 进度行：盘图标 + 进度条 + 「C: 可用 58.7 GB  共 200.0 GB」
// ---------------------------------------------------------------------------

fn progress_row(app: &App, ui: &mut egui::Ui, p: &Palette) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;

        // 左侧的"盘"图标 —— 三行就画完了，没必要为它塞一张素材
        let (icon_rect, _) = ui.allocate_exact_size(vec2(DRIVE_W, DRIVE_H), Sense::hover());
        paint_drive(ui.painter(), icon_rect, p);

        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 6.0;

            // 进度条 = 这个盘的占用率。参考图里它一直是盘占用率，不跟扫描走，
            // 扫描进度在大圆按钮那颗「%」上。
            let ratio = app.disk.as_ref().map(|d| d.used_ratio()).unwrap_or(0.0);
            let (bar_rect, _) = ui.allocate_exact_size(vec2(BAR_W, BAR_H), Sense::hover());
            let painter = ui.painter();
            let r = CornerRadius::same((BAR_H / 2.0).round() as u8);
            painter.rect_filled(bar_rect, r, p.surface_alt);
            if ratio > 0.0 {
                let mut filled = bar_rect;
                filled.set_right(bar_rect.left() + bar_rect.width() * ratio.clamp(0.02, 1.0));
                painter.rect_filled(filled, r, p.accent);
            }

            // 文字：跟条左端对齐，格式照参考图「C盘:可用58.7GB  共200.0GB」——
            // 盘名走 sysutil::mount_label，绝不硬编码 C 盘。
            let text = match &app.disk {
                Some(d) => format!(
                    "{}: 可用 {}   共 {}",
                    crate::sysutil::mount_label(&d.mount),
                    human_bytes(d.free),
                    human_bytes(d.total)
                ),
                None => "正在读取磁盘信息…".to_string(),
            };
            ui.label(RichText::new(text).size(12.0).color(p.text_weak));
        });
    });
}

/// 一个最小的"盘"图标：圆角机身 + 一条缝 + 右下角一点点厚度。
fn paint_drive(painter: &egui::Painter, rect: Rect, p: &Palette) {
    let body = Rect::from_min_size(rect.min, vec2(rect.width(), rect.height() * 0.78));
    let r = CornerRadius::same(6);
    painter.rect_filled(body.translate(vec2(0.0, rect.height() * 0.16)), r, p.border);
    painter.rect_filled(body, r, p.surface);
    painter.rect_stroke(
        body,
        r,
        Stroke::new(1.0, p.border),
        egui::StrokeKind::Inside,
    );
    let slot = Rect::from_min_size(
        pos2(body.left() + 6.0, body.bottom() - 9.0),
        vec2(body.width() - 12.0, 2.0),
    );
    painter.rect_filled(slot, CornerRadius::same(1), p.text_faint);
}

// ---------------------------------------------------------------------------
// 卡片行：严格四张
// ---------------------------------------------------------------------------

/// 这一张卡覆盖的所有分组。
fn card_groups(spec: &CardSpec) -> impl Iterator<Item = Group> {
    std::iter::once(spec.group).chain(spec.extra)
}

/// 卡片覆盖的目标（用来算体积、判断勾选状态）。
fn card_targets<'a>(app: &'a App, spec: &CardSpec) -> Vec<&'a crate::model::CleanTarget> {
    app.targets
        .iter()
        .filter(|t| card_groups(spec).any(|g| g == t.group))
        .collect()
}

/// 这一组的汇总（字节、文件数）—— **只算已勾选的目标**。
///
/// 卡片上写的应该是"点下去会清掉多少"，不是"这组里一共有多少"。两者在
/// 「系统文件与更新」那张卡上会差很多：它并进来的「高级项」默认关闭。
fn card_stat(app: &App, spec: &CardSpec) -> (u64, u64) {
    let mut bytes = 0;
    let mut files = 0;
    for t in card_targets(app, spec) {
        if !app.selected.contains(t.id) {
            continue;
        }
        if let Some(r) = app.result_for(t.id) {
            bytes += r.bytes;
            files += r.files;
        }
    }
    (bytes, files)
}

/// 卡片是否处于"已勾选"。
///
/// **只看主分组**，不看并进来的「高级项」。原因：高级项里是 `Windows.old`、
/// 更新还原点残留这种默认关闭的东西，如果把它们也算进"全勾上了吗"，这张卡
/// 一开始永远显示未勾选；而一旦点它一下，就会把高级项一起打开 —— 等于用一次
/// 普通的点选绕过了"默认关闭"这道闸。高级项保持它自己的开关（在展开的明细里）。
fn card_checked(app: &App, spec: &CardSpec) -> bool {
    let targets: Vec<_> = app
        .targets
        .iter()
        .filter(|t| t.group == spec.group)
        .collect();
    !targets.is_empty() && targets.iter().all(|t| app.selected.contains(t.id))
}

/// 扫描进行到哪一步 —— 参考图里卡片上写的就是「扫描中」/「待扫描」这两档。
fn card_phase(app: &App, spec: &CardSpec) -> CardPhase {
    if app.scanning() {
        let targets = card_targets(app, spec);
        let done = targets
            .iter()
            .filter(|t| {
                app.live_results()
                    .map(|rs| rs.iter().any(|r| r.id == t.id))
                    .unwrap_or(false)
            })
            .count();
        return match done {
            0 => CardPhase::Waiting,
            n if n == targets.len() => {
                let (bytes, files) = card_stat(app, spec);
                CardPhase::Done { bytes, files }
            }
            _ => CardPhase::Busy,
        };
    }
    if app.summary().is_some() {
        let (bytes, files) = card_stat(app, spec);
        CardPhase::Done { bytes, files }
    } else {
        CardPhase::Idle
    }
}

fn card_row(app: &App, ui: &mut egui::Ui, p: &Palette, actions: &mut Vec<Action>) {
    let full = ui.available_width();
    let w = ((full - CARD_GAP * 3.0) / 4.0).floor().max(120.0);

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = CARD_GAP;
        for (index, spec) in CARDS.iter().enumerate() {
            group_card(app, ui, p, spec, index, w, actions);
        }
    });
}

fn group_card(
    app: &App,
    ui: &mut egui::Ui,
    p: &Palette,
    spec: &CardSpec,
    index: usize,
    w: f32,
    actions: &mut Vec<Action>,
) {
    let (rect, _) = ui.allocate_exact_size(vec2(w, CARD_H), Sense::hover());
    let painter = ui.painter().clone();

    let checked = card_checked(app, spec);
    let phase = card_phase(app, spec);

    // 卡片底：比页面底**只浅一点点**（参考图里两者差不到 5/255，靠这点差值分层，
    // 不是靠白卡片）—— 所以这里和页面底混，而不是直接用 surface。
    painter.rect_filled(
        rect,
        CornerRadius::same(CARD_RADIUS),
        mix(p.bg, p.surface, 0.30),
    );

    // ---- 标题 ----
    // 纵向和右上角勾选框对齐（参考图里标题中线就压在勾选框中线上）。
    //
    // 标题**不随勾选变灰**：两张参考图里四张卡的标题取"最暗 5% 像素的平均亮度"
    // 都是 34（近黑），未勾选的那张也一样 —— 是否选中只由右上角那颗勾选框表达。
    // （勾选框本身 + 下面那行说明已经足够说明状态，标题变灰反而和参考图对不上。）
    paint_one_line(
        &painter,
        pos2(rect.left() + CARD_PAD, rect.top() + 23.0 - 7.0),
        spec.title,
        14.0,
        p.text,
        rect.width() - CARD_PAD - CHECK_INSET - CHECK_SIDE - 8.0,
    );

    // ---- 右上角勾选框 ----
    //
    // **扫描中整颗不画**。这不是偷懒：参考图的第二张（扫描中态）里四个勾选框位置
    // 都是干净的卡片底色（(231,238,247) 对卡片底 (232,238,248)），一个都没画；
    // 第一张空闲态才有（3 个绿 + 1 个灰）。扫描期间本来就轮不到改选择，跟着藏掉
    // 反而安静。命中区域一起藏，免得点到看不见的控件。
    if !app.scanning() {
        let check = Rect::from_min_size(
            pos2(
                rect.right() - CHECK_INSET - CHECK_SIDE,
                rect.top() + CHECK_INSET + 1.0,
            ),
            Vec2::splat(CHECK_SIDE),
        );
        let check_resp = ui.interact(
            check,
            ui.id().with(("clean_card_check", index)),
            Sense::click(),
        );
        paint_checkbox(&painter, check, checked, p, check_resp.hovered());
        if check_resp.clicked() {
            // Shift 点 = 一步把"所有有内容的项"都勾上。参考图里没有全选按钮，
            // 又不舍得把这个常用操作彻底删掉，就挂在勾选框上。
            if ui.input(|i| i.modifiers.shift) {
                actions.push(Action::SelectAllWithContent);
            } else {
                actions.push(Action::CheckCard(spec.group, spec.extra, !checked));
            }
        }
        if check_resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
    }

    // ---- 中间那一行：空闲时是说明，扫描中/扫完换成状态 ----
    let (mid_text, mid_color) = match phase {
        // 没勾选的卡就还是显示"这组清什么"，而不是体积 —— 否则会看到
        // 一个 0 字节的"已干净"，其实只是没勾。
        CardPhase::Idle => (
            spec.desc.to_string(),
            if checked { p.text_weak } else { p.text_faint },
        ),
        CardPhase::Done { .. } if !checked => (spec.desc.to_string(), p.text_faint),
        CardPhase::Waiting => ("待扫描".to_string(), p.text_faint),
        CardPhase::Busy => ("扫描中".to_string(), p.accent_ink),
        CardPhase::Done { bytes, files } => {
            if bytes > 0 {
                (
                    format!("{} · {} 个文件", human_bytes(bytes), thousands(files)),
                    p.text,
                )
            } else {
                ("已干净".to_string(), p.text_faint)
            }
        }
    };
    // 中间那一行的左端：空闲/扫完时没有记号，正文顶到内边距；
    // 扫描中/待扫描时先画一个小记号，正文再往右让 13px。
    let (has_mark, mark_center) = match phase {
        CardPhase::Waiting => (true, pos2(rect.left() + CARD_PAD + 3.0, rect.top() + 53.0)),
        CardPhase::Busy => (true, pos2(rect.left() + CARD_PAD + 3.0, rect.top() + 53.0)),
        _ => (false, egui::Pos2::ZERO),
    };
    if has_mark {
        match phase {
            CardPhase::Waiting => {
                painter.circle_filled(mark_center, 3.5, p.caution);
            }
            _ => {
                painter.circle_stroke(mark_center, 4.0, Stroke::new(1.5, p.accent));
            }
        }
    }
    let text_x = if has_mark {
        rect.left() + CARD_PAD + 13.0
    } else {
        rect.left() + CARD_PAD
    };
    paint_one_line(
        &painter,
        pos2(text_x, rect.top() + 53.0 - 7.0),
        &mid_text,
        12.5,
        mid_color,
        rect.right() - 10.0 - text_x,
    );

    // ---- 「扫描 >」----
    // 参考图里扫描进行中这一行是空的，所以只在空闲 / 扫完才画。
    if matches!(phase, CardPhase::Idle | CardPhase::Done { .. }) {
        let link_rect = Rect::from_min_size(
            pos2(rect.left() + CARD_PAD, rect.top() + 67.0),
            vec2(76.0, 24.0),
        );
        let resp = ui.interact(
            link_rect,
            ui.id().with(("clean_card_link", index)),
            Sense::click(),
        );
        let color = if resp.hovered() {
            p.accent
        } else {
            p.accent_ink
        };
        painter.text(
            pos2(link_rect.left(), rect.top() + 79.0),
            Align2::LEFT_CENTER,
            "扫描 >",
            FontId::proportional(12.5),
            color,
        );
        if resp.clicked() {
            actions.push(Action::OpenGroup(spec.group));
        }
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
    }
}

/// 单行文字，超宽就**收尾省略号**截断。
///
/// 卡片只有 208px 宽、内边距各 18px，一行放不下 14 个字；直接画会溢出到隔壁卡片上
/// （第一版就溢出了）。这里用 `LayoutJob` 的 `max_rows = 1` + `overflow_character`
/// 让 egui 自己收尾，比手数字数可靠。
fn paint_one_line(
    painter: &egui::Painter,
    pos: egui::Pos2,
    text: &str,
    size: f32,
    color: Color32,
    max_w: f32,
) {
    let mut job = egui::text::LayoutJob::single_section(
        text.to_string(),
        egui::TextFormat {
            font_id: FontId::proportional(size),
            color,
            ..Default::default()
        },
    );
    job.wrap = egui::text::TextWrapping {
        max_width: max_w,
        max_rows: 1,
        break_anywhere: false,
        overflow_character: Some('…'),
    };
    let galley = painter.layout_job(job);
    painter.galley(pos, galley, color);
}

/// 参考图右上角那颗勾选框：绿色圆角方块 + 白勾；未勾选时是一块浅灰底。
fn paint_checkbox(painter: &egui::Painter, rect: Rect, checked: bool, p: &Palette, hovered: bool) {
    // 圆角、勾线长度都按 CHECK_SIDE=29 反推（参考图 24px 框配 7px 圆角、1.9px 勾线）。
    let r = CornerRadius::same((CHECK_SIDE * 0.29).round() as u8);
    if checked {
        let fill = if hovered {
            mix(p.accent, Color32::WHITE, 0.12)
        } else {
            p.accent
        };
        painter.rect_filled(rect, r, fill);
        let c = rect.center();
        let k = CHECK_SIDE / 24.0;
        let stroke = Stroke::new(1.9 * k, Color32::WHITE);
        painter.line_segment(
            [
                pos2(c.x - 5.6 * k, c.y + 0.2 * k),
                pos2(c.x - 1.8 * k, c.y + 4.0 * k),
            ],
            stroke,
        );
        painter.line_segment(
            [
                pos2(c.x - 1.8 * k, c.y + 4.0 * k),
                pos2(c.x + 5.8 * k, c.y - 4.2 * k),
            ],
            stroke,
        );
    } else {
        // 未勾选的底用 `border`(#D7E1EF) 而不是 `surface_alt`(#EEF3FA)：
        // 参考图里未勾选那颗是**比卡片底更暗**的灰方块 —— 量到 (215,224,238)，
        // 恰好就是本程序 border 的色值；用 surface_alt 会比卡片底还亮，
        // 等于看不见，"这里还有个开关"就传达不出去了。
        let fill = if hovered {
            mix(p.border, p.text_faint, 0.25)
        } else {
            p.border
        };
        painter.rect_filled(rect, r, fill);
    }
}

// ---------------------------------------------------------------------------
// 大圆按钮
// ---------------------------------------------------------------------------

/// 大圆按钮的三种状态：能做什么、上面写什么。
enum CtaKind {
    /// 还没扫描：点一下开始扫描
    Scan,
    /// 正在扫描：只能看进度，取消走下面那行小字
    Busy,
    /// 扫完了：点一下开始清理
    Clean,
    /// 扫完但没东西可清：点一下重新扫描
    Rescan,
}

fn cta_kind(app: &App) -> CtaKind {
    if app.scanning() {
        return CtaKind::Busy;
    }
    if app.summary().is_some() {
        let (_, _, bytes) = app.pending_selection();
        return if bytes > 0 {
            CtaKind::Clean
        } else {
            CtaKind::Rescan
        };
    }
    CtaKind::Scan
}

fn big_button(app: &App, ui: &mut egui::Ui, p: &Palette, actions: &mut Vec<Action>) {
    let full = ui.available_width();
    let kind = cta_kind(app);
    let busy = matches!(kind, CtaKind::Busy);

    // 外发光 / 雷达扫尾 / 脉冲波都会画出圆外最多 ~15px，容器要留出这个领空，
    // 否则会被下一行内容压住（egui 不裁剪画笔，但会跟下面的字打架）。
    ui.allocate_ui_with_layout(
        vec2(full, CIRCLE + 32.0),
        Layout::top_down(Align::Center),
        |ui| {
            let (rect, resp) = ui.allocate_exact_size(Vec2::splat(CIRCLE), Sense::click());
            let painter = ui.painter();
            let center = rect.center();
            let radius = CIRCLE / 2.0;

            // 底色：扫描中用更饱和的绿；平时悬停提亮一档。
            // 「一键清理」再往深里压一档 —— 它是真正要执行删除的那一下，
            // 颜色比"扫描"重一点才分得开。
            let base = match kind {
                CtaKind::Busy => p.cta_busy,
                CtaKind::Clean => mix(p.cta, p.accent, 0.35),
                CtaKind::Scan | CtaKind::Rescan => {
                    if resp.hovered() {
                        mix(p.cta, Color32::WHITE, 0.14)
                    } else {
                        p.cta
                    }
                }
            };

            // 球面渐变圆盘 + 外发光 + 玻璃高光
            let glow = if resp.hovered() || busy { 1.0 } else { 0.62 };
            paint_cta_disc(painter, center, radius, base, glow);

            if busy {
                // 扫描中：进度环 + 雷达扫尾 + 脉冲波，全部吃时间参数
                let t = ui.ctx().input(|i| i.time) as f32;
                paint_cta_radar(painter, center, radius, base, scan_ratio(app), t);
                // 下一次重绘越快动画越顺；只在扫描中花这个电
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(16));
            }

            match kind {
                CtaKind::Busy => {
                    let pct = scan_ratio(app) * 100.0;
                    draw_number_unit(
                        painter,
                        center,
                        &format!("{:.0}", pct),
                        "%",
                        30.0,
                        14.0,
                        p.cta_ink,
                    );
                    painter.text(
                        center + vec2(0.0, 26.0),
                        Align2::CENTER_CENTER,
                        "扫描中",
                        FontId::proportional(10.5),
                        cta_alpha(p.cta_ink, 150),
                    );
                }
                CtaKind::Scan => {
                    painter.text(
                        center,
                        Align2::CENTER_CENTER,
                        "扫描",
                        FontId::proportional(17.0),
                        p.cta_ink,
                    );
                }
                CtaKind::Clean | CtaKind::Rescan => {
                    let (_, _, bytes) = app.pending_selection();
                    if bytes > 0 {
                        let (n, unit) = split_size(bytes);
                        draw_number_unit(painter, center, &n, &unit, 27.0, 13.0, p.cta_ink);
                    } else {
                        painter.text(
                            center,
                            Align2::CENTER_CENTER,
                            "重新扫描",
                            FontId::proportional(16.0),
                            p.cta_ink,
                        );
                    }
                }
            }

            if resp.clicked() {
                match kind {
                    CtaKind::Scan | CtaKind::Rescan => actions.push(Action::Scan),
                    CtaKind::Clean => actions.push(Action::Clean),
                    CtaKind::Busy => {}
                }
            }
            if resp.hovered() && !busy {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
        },
    );
}

/// 同色不同透明度。
fn cta_alpha(c: Color32, a: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), a)
}

/// 大圆的主体：落影 + 侧壁厚度 + 球面渐变 + 镜面高光。
///
/// 立体感由四层叠出来（从下往上）：
/// 1. **落影** —— 向下偏移的一叠低透明度圆形，圆"离开页面"浮起来；
/// 2. **侧壁** —— 再往下一档的实心圆，只在底部露出一条深色月牙，等于厚度；
/// 3. **球面** —— 三角形网格做两段径向渐变（高光 → 中间调 → 暗边），
///    光心偏左上，比单段线性渐变更像球；
/// 4. **高光** —— 顶上一段偏白的镜面弧 + 左下一条反光弧，玻璃质感。
///
/// `Mesh` 是硬边光栅化（像素中心进轮廓就涂满），所以轮廓的柔和感全靠
/// 一圈 1px 描边接住，外发光的描边环把边缘往外羽化。
fn paint_cta_disc(
    painter: &egui::Painter,
    center: egui::Pos2,
    radius: f32,
    base: Color32,
    glow: f32,
) {
    // ---- 1) 落影：越靠内越实，整体压在圆的正下方 ----
    const SHADOW_STEPS: usize = 10;
    for i in 0..SHADOW_STEPS {
        let t = i as f32 / (SHADOW_STEPS - 1) as f32; // 0 = 最外圈
        let r = radius * (0.80 + 0.24 * t) + 2.0;
        let a = ((3.0 + t * 7.0) * if glow > 0.8 { 1.25 } else { 1.0 }) as u8;
        painter.circle_filled(
            center + vec2(0.0, 6.0 + t * 2.0),
            r,
            Color32::from_black_alpha(a),
        );
    }

    // ---- 2) 外发光：一圈圈往外淡出的细环，悬停 / 扫描中变强 ----
    let glow_col = mix(base, Color32::WHITE, 0.12);
    const GLOW_STEPS: usize = 9;
    for i in 0..GLOW_STEPS {
        let t = i as f32 / (GLOW_STEPS - 1) as f32;
        let r = radius + 2.0 + t * 10.0;
        let a = ((1.0 - t) * (1.0 - t) * 70.0 * glow) as u8;
        if a == 0 {
            continue;
        }
        painter.circle_stroke(center, r, Stroke::new(3.5, cta_alpha(glow_col, a)));
    }

    // ---- 3) 侧壁：往下偏移的实心圆，只在底部露出一道深色月牙 = 厚度 ----
    let dark = mix(base, Color32::BLACK, 0.34);
    painter.circle_filled(center + vec2(0.0, 4.0), radius + 1.0, dark);

    // ---- 4) 球面：两段径向渐变（光心 → 中间调 → 暗边） ----
    let hot = mix(base, Color32::WHITE, 0.46);
    let mid = mix(base, Color32::WHITE, 0.10);
    let rim = mix(base, Color32::BLACK, 0.24);
    const SEGMENTS: usize = 72;
    let dir = |i: usize| {
        let a = i as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
        vec2(a.cos(), a.sin())
    };
    let light = center + vec2(-radius * 0.28, -radius * 0.32);

    let mut mesh = egui::Mesh::default();
    // 光心
    mesh.colored_vertex(light, hot);
    // 内环（0.56r）与外环（r）
    for i in 0..=SEGMENTS {
        mesh.colored_vertex(center + dir(i) * (radius * 0.56), mid);
    }
    for i in 0..=SEGMENTS {
        mesh.colored_vertex(center + dir(i) * radius, rim);
    }
    let inner = |i: usize| 1 + i as u32;
    let outer = |i: usize| 1 + (SEGMENTS as u32 + 1) + i as u32;
    for i in 0..SEGMENTS as u32 {
        mesh.add_triangle(0, inner(i as usize), inner(i as usize + 1));
        mesh.add_triangle(inner(i as usize), outer(i as usize), outer(i as usize + 1));
        mesh.add_triangle(
            inner(i as usize),
            outer(i as usize + 1),
            inner(i as usize + 1),
        );
    }
    painter.add(egui::Shape::mesh(mesh));

    // ---- 5) 轮廓：一条略深的细边把硬边接住，也让圆在浅底上立得住 ----
    painter.circle_stroke(
        center,
        radius,
        Stroke::new(1.0, cta_alpha(mix(base, Color32::BLACK, 0.30), 190)),
    );

    // ---- 6) 高光：顶部镜面弧 + 底部反光弧 ----
    let arc = |from: f32, to: f32, r: f32, stroke: Stroke| {
        const N: usize = 28;
        let pts: Vec<egui::Pos2> = (0..=N)
            .map(|i| {
                let a = from + (to - from) * i as f32 / N as f32;
                center + vec2(a.cos() * r, a.sin() * r)
            })
            .collect();
        painter.add(egui::Shape::line(pts, stroke));
    };
    let hr = radius - 3.0;
    // 屏幕坐标 y 向下，负角是上半圆。宽而淡的一层 + 窄而亮的一层
    arc(
        -std::f32::consts::PI * 0.88,
        -std::f32::consts::PI * 0.12,
        hr,
        Stroke::new(5.0, cta_alpha(Color32::WHITE, 38)),
    );
    arc(
        -std::f32::consts::PI * 0.80,
        -std::f32::consts::PI * 0.28,
        hr,
        Stroke::new(2.0, cta_alpha(Color32::WHITE, 150)),
    );
    // 左下的一小段反光，让球面有"被环境照亮"的第二光源
    arc(
        std::f32::consts::PI * 0.30,
        std::f32::consts::PI * 0.62,
        hr,
        Stroke::new(2.2, cta_alpha(Color32::WHITE, 55)),
    );
    // 底部内阴影：压住下缘，球才"坐"得下去
    arc(
        std::f32::consts::PI * 0.12,
        std::f32::consts::PI * 0.88,
        radius - 5.0,
        Stroke::new(4.0, cta_alpha(mix(base, Color32::BLACK, 0.45), 40)),
    );
}

/// 扫描中的三件套：进度环、雷达扫尾、脉冲波。
///
/// `t` 是秒（egui 的 `input.time`），`ratio` 是 0..1 的扫描进度。
fn paint_cta_radar(
    painter: &egui::Painter,
    center: egui::Pos2,
    radius: f32,
    base: Color32,
    ratio: f32,
    t: f32,
) {
    let arc = |from: f32, to: f32, r: f32, stroke: Stroke| {
        let n = (((to - from).abs() / 0.12) as usize).clamp(2, 96);
        let pts: Vec<egui::Pos2> = (0..=n)
            .map(|i| {
                let a = from + (to - from) * i as f32 / n as f32;
                center + vec2(a.cos() * r, a.sin() * r)
            })
            .collect();
        painter.add(egui::Shape::line(pts, stroke));
    };

    // ---- 进度环：贴着圆盘内缘一圈，白轨道 + 白进度 ----
    let pr = radius - 9.0;
    painter.circle_stroke(center, pr, Stroke::new(4.0, cta_alpha(Color32::WHITE, 60)));
    if ratio > 0.003 {
        let start = -std::f32::consts::FRAC_PI_2;
        let end = start + std::f32::consts::TAU * ratio.clamp(0.01, 1.0);
        arc(
            start,
            end,
            pr,
            Stroke::new(4.0, cta_alpha(Color32::WHITE, 235)),
        );
    }

    // ---- 雷达扫尾：外缘一圈旋转的彗尾，越靠头部越亮 ----
    let spin = t * std::f32::consts::TAU * 0.85;
    let tail = 1.7;
    const TAIL_STEPS: usize = 16;
    for i in 0..TAIL_STEPS {
        let f0 = i as f32 / TAIL_STEPS as f32;
        let f1 = (i + 1) as f32 / TAIL_STEPS as f32;
        let a0 = spin - tail + tail * f0;
        let a1 = spin - tail + tail * f1;
        let p0 = center + vec2(a0.cos(), a0.sin()) * (radius + 4.5);
        let p1 = center + vec2(a1.cos(), a1.sin()) * (radius + 4.5);
        let a = (f0 * f0 * 140.0) as u8;
        if a == 0 {
            continue;
        }
        painter.add(egui::Shape::line(
            vec![p0, p1],
            Stroke::new(2.5, cta_alpha(Color32::WHITE, a)),
        ));
    }

    // ---- 脉冲波：从圆边往外扩散、边走边淡出的圆环 ----
    let pulse = (t / 1.6) % 1.0;
    let pr_r = radius + 2.0 + pulse * 11.0;
    let pa = ((1.0 - pulse) * (1.0 - pulse) * 80.0) as u8;
    if pa > 0 {
        painter.circle_stroke(center, pr_r, Stroke::new(1.5, cta_alpha(base, pa)));
    }
}

/// 把「12.4 GB」拆成数字和单位两段，圆按钮里数字大、单位小。
fn split_size(bytes: u64) -> (String, String) {
    let text = human_bytes(bytes);
    match text.split_once(' ') {
        Some((n, unit)) => (n.to_string(), unit.to_string()),
        None => (text, String::new()),
    }
}

/// 大数字 + 小单位，作为一个整体在圆心居中。
fn draw_number_unit(
    painter: &egui::Painter,
    center: egui::Pos2,
    number: &str,
    unit: &str,
    big: f32,
    small: f32,
    color: Color32,
) {
    let big_galley = painter.layout_no_wrap(number.to_string(), FontId::proportional(big), color);
    let small_galley = painter.layout_no_wrap(unit.to_string(), FontId::proportional(small), color);
    let gap = if unit.is_empty() { 0.0 } else { 2.0 };
    let total = big_galley.size().x + gap + small_galley.size().x;
    let left = center.x - total / 2.0;

    painter.galley(
        pos2(left, center.y - big_galley.size().y / 2.0),
        big_galley.clone(),
        color,
    );
    if !unit.is_empty() {
        // 单位贴着数字的基线坐，而不是垂直居中 —— 居中会显得"浮"起来
        painter.galley(
            pos2(
                left + big_galley.size().x + gap,
                center.y - small_galley.size().y / 2.0 + 6.0,
            ),
            small_galley,
            color,
        );
    }
}

// ---------------------------------------------------------------------------
// 圆按钮下面那行小字
// ---------------------------------------------------------------------------

fn hint_row(app: &App, ui: &mut egui::Ui, p: &Palette, actions: &mut Vec<Action>) {
    const SIZE: f32 = 12.5;
    let font = FontId::proportional(SIZE);

    // 三段：[前文][可点的绿字][后文]
    let parts: [(&str, Color32, bool); 3] = if app.scanning() {
        [
            ("点击 ", p.text_weak, false),
            ("取消扫描", p.accent_ink, true),
            (" 可重新选择扫描项", p.text_weak, false),
        ]
    } else if app.summary().is_some() {
        if matches!(cta_kind(app), CtaKind::Clean) {
            [
                ("点击圆圈开始一键清理 · ", p.text_weak, false),
                ("清理前会再确认一次", p.accent_ink, false),
                ("", p.text_weak, false),
            ]
        } else {
            [
                ("勾选一个清理类别后即可开始 · ", p.text_weak, false),
                ("扫描只读，不会删除", p.accent_ink, false),
                ("", p.text_weak, false),
            ]
        }
    } else {
        [
            ("扫描只读取文件信息，", p.text_weak, false),
            ("不会删除任何东西", p.accent_ink, false),
            ("", p.text_weak, false),
        ]
    };

    // 先量宽度再整体居中——`ui.horizontal` 在里面夹一个可点区域时宽度不好控，
    // 这里干脆自己排版，顺便拿到"取消扫描"那段的精确点击区。
    let galleys: Vec<_> = parts
        .iter()
        .map(|(text, color, _)| {
            ui.painter()
                .layout_no_wrap(text.to_string(), font.clone(), *color)
        })
        .collect();
    let total: f32 = galleys.iter().map(|g| g.size().x).sum();

    let full = ui.available_width();
    ui.allocate_ui_with_layout(
        vec2(full, SIZE + 8.0),
        Layout::top_down(Align::Center),
        |ui| {
            let (rect, _) = ui.allocate_exact_size(vec2(full, SIZE + 8.0), Sense::hover());
            let painter = ui.painter();
            let y = rect.center().y;
            let mut x = rect.center().x - total / 2.0;

            for (i, galley) in galleys.iter().enumerate() {
                let part = &parts[i];
                if part.2 {
                    let hit = Rect::from_min_max(
                        pos2(x, y - (SIZE + 6.0) / 2.0),
                        pos2(x + galley.size().x, y + (SIZE + 6.0) / 2.0),
                    );
                    let resp = ui.interact(hit, ui.id().with("clean_hint_cancel"), Sense::click());
                    if resp.clicked() {
                        actions.push(Action::Cancel);
                    }
                    if resp.hovered() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                }
                painter.galley(pos2(x, y - galley.size().y / 2.0), galley.clone(), part.1);
                x += galley.size().x;
            }
        },
    );
}

// ---------------------------------------------------------------------------
// 展开的那一组明细的标题行
// ---------------------------------------------------------------------------

fn detail_header(
    app: &App,
    ui: &mut egui::Ui,
    p: &Palette,
    group: Group,
    actions: &mut Vec<Action>,
) {
    let (bytes, files) = {
        let spec = CardSpec {
            group,
            extra: None,
            title: "",
            desc: "",
        };
        let mut bytes = 0;
        let mut files = 0;
        for t in card_targets(app, &spec) {
            if let Some(r) = app.result_for(t.id) {
                bytes += r.bytes;
                files += r.files;
            }
        }
        (bytes, files)
    };

    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!("{} 明细", group.title()))
                .size(14.0)
                .strong()
                .color(p.text),
        );
        if bytes > 0 {
            ui.label(
                RichText::new(format!(
                    "{} · {} 个文件",
                    human_bytes(bytes),
                    thousands(files)
                ))
                .size(12.5)
                .color(p.text_weak),
            );
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let resp = ui.add(
                egui::Button::new(RichText::new("收起").size(12.5).color(p.text_weak)).frame(false),
            );
            if resp.clicked() {
                actions.push(Action::OpenGroup(group));
            }
        });
    });
}

// ---------------------------------------------------------------------------
// 通用：这一轮扫描的进度
// ---------------------------------------------------------------------------

fn scan_ratio(app: &App) -> f32 {
    match &app.scan {
        ScanState::Running {
            finished, total, ..
        } if *total > 0 => *finished as f32 / *total as f32,
        _ => 0.0,
    }
}

// ---------------------------------------------------------------------------
// 分组与清理项
// ---------------------------------------------------------------------------

fn render_groups(app: &App, ui: &mut egui::Ui, p: &Palette, actions: &mut Vec<Action>) {
    let mut first_section = true;

    for group in Group::ALL {
        if let Some(filter) = app.group_filter
            && filter != group
        {
            continue;
        }

        let targets: Vec<&crate::model::CleanTarget> =
            app.targets.iter().filter(|t| t.group == group).collect();
        if targets.is_empty() {
            continue;
        }

        // 「只看已勾选」过滤
        let targets: Vec<&crate::model::CleanTarget> = if app.only_checked {
            targets
                .into_iter()
                .filter(|t| app.selected.contains(t.id))
                .collect()
        } else {
            targets
        };
        if targets.is_empty() {
            continue;
        }

        if !first_section {
            ui.add_space(14.0);
        }
        first_section = false;

        // ---- 分组头 ----
        let mut group_bytes = 0u64;
        let mut group_files = 0u64;
        let mut all_checked = true;
        for t in &targets {
            if !app.selected.contains(t.id) {
                all_checked = false;
            }
            if let Some(r) = app.result_for(t.id) {
                group_bytes += r.bytes;
                group_files += r.files;
            }
        }

        ui.horizontal(|ui| {
            ui.label(
                RichText::new(group.title())
                    .size(14.0)
                    .strong()
                    .color(p.text),
            );
            if group == Group::Advanced {
                pill(ui, "需要你明确确认", PillKind::Danger, p);
            }
            ui.add_space(6.0);
            if group_files > 0 {
                ui.label(
                    RichText::new(format!(
                        "{} 个文件 · {}",
                        thousands(group_files),
                        human_bytes(group_bytes)
                    ))
                    .size(11.5)
                    .color(p.text_weak),
                );
            }

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let mut checked = all_checked;
                let label = if all_checked {
                    "取消本组"
                } else {
                    "全选本组"
                };
                if ui
                    .checkbox(&mut checked, RichText::new(label).size(11.5))
                    .changed()
                {
                    actions.push(Action::GroupAll(group, !all_checked));
                }
            });
        });
        ui.add_space(6.0);

        for target in targets {
            target_card(app, ui, p, target, actions);
            ui.add_space(8.0);
        }
    }
}

fn target_card(
    app: &App,
    ui: &mut egui::Ui,
    p: &Palette,
    target: &crate::model::CleanTarget,
    actions: &mut Vec<Action>,
) {
    let checked = app.selected.contains(target.id);
    let expanded = app.expanded.contains(target.id);
    let result = app.result_for(target.id);

    let fill = if checked { p.surface } else { p.surface_alt };
    let border = if checked {
        p.accent.gamma_multiply(0.35)
    } else {
        p.border
    };

    egui::Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0, border))
        .corner_radius(CornerRadius::same(crate::theme::CARD_RADIUS))
        .inner_margin(egui::Margin::symmetric(16, 13))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());

            // ---- 第一行：勾选 / 名称 / 体积 ----
            ui.horizontal(|ui| {
                let mut c = checked;
                if ui
                    .add(egui::Checkbox::new(&mut c, ""))
                    .on_hover_text("勾选后才会被清理")
                    .changed()
                {
                    actions.push(Action::Toggle(target.id.to_string()));
                }

                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(target.name)
                                .size(14.5)
                                .strong()
                                .color(p.text),
                        );
                        risk_pill(ui, target.risk, p).on_hover_text(target.risk.hint());
                        if target.requires_admin && !app.admin {
                            pill(ui, "需管理员", PillKind::Caution, p);
                        }
                        if app.settings.dry_run && checked {
                            pill(ui, "演练", PillKind::Neutral, p);
                        }
                    });
                    ui.add_space(2.0);
                    ui.label(
                        RichText::new(target.desc)
                            .size(12.0)
                            .color(p.text_weak),
                    );
                });

                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    match &result {
                        Some(r) if r.files > 0 => {
                            ui.vertical(|ui| {
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    ui.label(widgets::size_text(r.bytes, p));
                                });
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    ui.label(
                                        RichText::new(widgets::count_pair(r.files, r.dirs))
                                            .size(11.0)
                                            .color(p.text_faint),
                                    );
                                });
                            });
                        }
                        Some(_) => {
                            ui.label(
                                RichText::new("无内容")
                                    .size(12.0)
                                    .color(p.text_faint),
                            );
                        }
                        None => {
                            ui.label(
                                RichText::new(if app.scanning() { "扫描中…" } else { "未扫描" })
                                    .size(12.0)
                                    .color(p.text_faint),
                            );
                        }
                    }
                });
            });

            // ---- 第二行：状态 + 操作 ----
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if let Some(r) = &result {
                    let (label, kind) = status_pill(r.status);
                    pill(ui, label, kind, p);
                    if r.skipped > 0 {
                        pill(
                            ui,
                            &format!("跳过 {}", thousands(r.skipped)),
                            PillKind::Neutral,
                            p,
                        );
                    }
                }
                if let Some(note) = target.note {
                    ui.add_space(4.0);
                    ui.label(RichText::new(note).size(11.0).color(p.text_faint));
                }

                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let can_expand = result.as_ref().map(|r| !r.preview.is_empty()).unwrap_or(false);
                    if can_expand {
                        let label = if expanded { "收起明细" } else { "查看明细" };
                        if widgets::ghost_button(ui, p, label).clicked() {
                            actions.push(Action::ToggleExpand(target.id.to_string()));
                        }
                    }
                    if let Some(r) = &result
                        && let Some(root) = r.resolved_roots.first()
                    {
                        if widgets::ghost_button(ui, p, "打开目录").clicked() {
                            actions.push(Action::Reveal(root.clone()));
                        }
                    }
                });
            });

            // ---- 明细 ----
            if expanded
                && let Some(r) = &result
                && !r.preview.is_empty()
            {
                ui.add_space(10.0);
                widgets::section(ui, p, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!(
                                "体积最大的 {} 个条目（共 {} 个）",
                                r.preview.len(),
                                thousands(r.files)
                            ))
                            .size(11.5)
                            .strong()
                            .color(p.text_weak),
                        );
                    });
                    ui.add_space(6.0);

                    egui::ScrollArea::vertical()
                        .id_salt(("preview", target.id))
                        .max_height(230.0)
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            for entry in &r.preview {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new(entry.path.to_string_lossy().to_string())
                                            .size(11.0)
                                            .color(p.text_weak),
                                    );
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        ui.label(
                                            RichText::new(human_bytes(entry.size))
                                                .size(11.0)
                                                .color(p.text)
                                                .monospace(),
                                        );
                                        ui.add_space(6.0);
                                        ui.label(
                                            RichText::new(crate::cleaner::format_time(entry.modified))
                                                .size(10.5)
                                                .color(p.text_faint),
                                        );
                                    });
                                });
                            }
                        });
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(
                            "明细按体积排序，仅用于确认；实际清理会重新遍历目录，因此可能包含更多同类型文件。",
                        )
                        .size(10.5)
                        .color(p.text_faint),
                    );
                });
            }
        });
}

fn status_pill(status: TargetStatus) -> (&'static str, PillKind) {
    let kind = match status {
        TargetStatus::Ok | TargetStatus::Partial => PillKind::Safe,
        TargetStatus::Empty | TargetStatus::Missing => PillKind::Neutral,
        TargetStatus::Denied => PillKind::Caution,
    };
    (status.label(), kind)
}

/// 上一次清理的结果横幅。
fn clean_report_banner(app: &App, ui: &mut egui::Ui, p: &Palette) {
    let CleanState::Done(report) = &app.clean else {
        return;
    };

    card(ui, p, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(if report.dry_run {
                    "上次演练结果"
                } else {
                    "上次清理结果"
                })
                .size(13.0)
                .strong()
                .color(p.text),
            );
            ui.add_space(6.0);
            pill(
                ui,
                &format!(
                    "{} · {} 个文件",
                    human_bytes(report.freed_bytes),
                    thousands(report.deleted_files)
                ),
                if report.dry_run {
                    PillKind::Neutral
                } else {
                    PillKind::Safe
                },
                p,
            );
            if report.skipped > 0 {
                pill(
                    ui,
                    &format!("跳过 {}", thousands(report.skipped)),
                    PillKind::Neutral,
                    p,
                );
            }
        });
        ui.add_space(4.0);
        ui.label(
            RichText::new("清理后各类别的体积已变化，建议重新扫描一次再决定下一步。")
                .size(11.5)
                .color(p.text_faint),
        );
    });
    ui.add_space(8.0);
}
