//! 图标：绝大多数用 egui 的图元直接画出来，微信 / QQ 例外 —— 这两个是品牌标识，
//! 用 `assets/` 里的 SVG 原样光栅化（见 `ui::svg_icon`），保留各自的配色。
//!
//! 为什么不引图标字体：
//!
//! * **单文件分发**是这个项目的一条底线 —— 多一个字体文件就多一个可能丢失的
//!   附件；SVG 用 `include_bytes!` / `include_str!` 编进可执行文件，同样不带附件。
//! * 线框图标形状都很简单，手画比维护一套字体子集划算。
//! * 线框图标能直接跟着主题色、描边粗细走，深浅两套主题不用各备一套资源。
//!
//! 每个图标都在**归一化的 0..1 方格**里描述，再映射到传入的 `Rect`，
//! 所以调用方给多大就画多大，比例永远不变。
//!
//! 画法是 **duotone（双色调）**：主体先用同一主题色的半透明版填一层底，
//! 再叠全色描边 —— 比"一根根细线"有体积感，17px 的侧栏小图标也不会糊成一团；
//! 而且底色由传入的 `color` 推出来，选中态的绿色、未选态的灰色、彩色入口卡
//! 上的前景色都自动成立，深浅主题不用各备一套。

use eframe::egui::{self, Color32, Pos2, Rect, Shape, Stroke, pos2};

/// 一套线框图标。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    /// 概览 —— 三层堆叠
    Overview,
    /// 清理 —— 垃圾桶
    Clean,
    /// 大文件 —— 折角文档
    BigFiles,
    /// 软件卸载 —— 立方体
    Apps,
    /// 设置 —— 三条滑杆
    Settings,
    /// 安全说明 —— 盾牌 + 勾
    Safety,
    /// 扫描 —— 放大镜
    Scan,
    /// 重复文件 —— 前后两张叠着的文档
    Duplicates,
    /// 系统设置 —— 显示器（屏幕 + 支架 + 底座）
    System,
    /// 微信 —— 用 assets/wechat.svg 的品牌图标（多色，不跟随主题色）
    Wechat,
    /// QQ —— 用 assets/qq.svg 的企鹅图标（多色，不跟随主题色）
    Qq,
}

/// 把一个图标画进 `rect`（取短边为准，居中）。
///
/// `width` 是线宽，`color` 是线的颜色。微信 / QQ 是彩色品牌图标，这两个参数对它们
/// 不生效（画出来就是 SVG 里的原色）。
pub fn paint(painter: &egui::Painter, rect: Rect, icon: Icon, color: Color32, width: f32) {
    // 取正方形区域，避免调用方给一个长方形时图标被拉扁
    let side = rect.width().min(rect.height());
    let box_ = Rect::from_center_size(rect.center(), egui::Vec2::splat(side));
    let stroke = Stroke::new(width, color);

    // 归一化坐标 → 屏幕坐标
    let p = |x: f32, y: f32| -> Pos2 {
        pos2(
            box_.left() + box_.width() * x,
            box_.top() + box_.height() * y,
        )
    };
    let line = |pts: &[(f32, f32)]| -> Vec<Pos2> { pts.iter().map(|(x, y)| p(*x, *y)).collect() };

    // duotone 底色：同一主题色降透明度（egui 的 Color32 是预乘 alpha，
    // gamma_multiply 是预乘的正确做法，直接改 a 通道会把颜色洗白）。
    let tint = |a: f32| color.gamma_multiply(a);
    // 底色填充 + 描边一次画完（多边形必须是凸的 —— 下面的形状都保证凸）
    let fill_stroke =
        |pts: Vec<Pos2>, a: f32| painter.add(Shape::convex_polygon(pts, tint(a), stroke));

    match icon {
        // 概览 —— 三层堆叠：顶层菱形上底色，下面两道"折线"
        Icon::Overview => {
            fill_stroke(
                line(&[(0.50, 0.08), (0.90, 0.28), (0.50, 0.48), (0.10, 0.28)]),
                0.26,
            );
            painter.add(Shape::line(
                line(&[(0.10, 0.54), (0.50, 0.72), (0.90, 0.54)]),
                stroke,
            ));
            painter.add(Shape::line(
                line(&[(0.10, 0.74), (0.50, 0.92), (0.90, 0.74)]),
                stroke,
            ));
        }

        // 垃圾桶：盖 + 提手 + 桶身（上底色）+ 两道竖纹
        Icon::Clean => {
            // 提手（U 形，骑在盖上方）
            painter.add(Shape::line(
                line(&[(0.40, 0.26), (0.40, 0.14), (0.60, 0.14), (0.60, 0.26)]),
                stroke,
            ));
            // 盖 —— 比桶口宽，经典垃圾桶比例
            painter.line_segment([p(0.12, 0.30), p(0.88, 0.30)], stroke);
            // 桶身梯形（收底）
            fill_stroke(
                line(&[(0.24, 0.38), (0.76, 0.38), (0.70, 0.90), (0.30, 0.90)]),
                0.22,
            );
            // 两道竖纹
            painter.line_segment([p(0.43, 0.50), p(0.44, 0.78)], stroke);
            painter.line_segment([p(0.57, 0.50), p(0.56, 0.78)], stroke);
        }

        // 折角文档 + 两道内容线（大文件）
        Icon::BigFiles => {
            fill_stroke(
                line(&[
                    (0.26, 0.08),
                    (0.60, 0.08),
                    (0.80, 0.28),
                    (0.80, 0.92),
                    (0.26, 0.92),
                ]),
                0.20,
            );
            painter.add(Shape::line(
                line(&[(0.60, 0.08), (0.60, 0.28), (0.80, 0.28)]),
                stroke,
            ));
            painter.line_segment([p(0.38, 0.52), p(0.68, 0.52)], stroke);
            painter.line_segment([p(0.38, 0.68), p(0.60, 0.68)], stroke);
        }

        // 软件卸载 —— 立方体包装盒：顶面上底色，棱线描边
        Icon::Apps => {
            fill_stroke(
                line(&[(0.50, 0.08), (0.88, 0.28), (0.50, 0.48), (0.12, 0.28)]),
                0.30,
            );
            painter.add(Shape::line(
                line(&[
                    (0.12, 0.28),
                    (0.12, 0.70),
                    (0.50, 0.92),
                    (0.88, 0.70),
                    (0.88, 0.28),
                ]),
                stroke,
            ));
            painter.line_segment([p(0.50, 0.48), p(0.50, 0.92)], stroke);
        }

        // 三条滑杆：线在圆钮处断开，圆钮画成实心 —— 实心钮是"滑杆"语义的关键，
        // 空心圆在 17px 下会被误读成三个圈圈
        Icon::Settings => {
            for (y, knob) in [(0.24_f32, 0.34_f32), (0.50, 0.66), (0.76, 0.44)] {
                let r = 0.105;
                painter.line_segment([p(0.12, y), p(knob - r, y)], stroke);
                painter.line_segment([p(knob + r, y), p(0.88, y)], stroke);
                painter.circle(p(knob, y), box_.width() * r, color, Stroke::NONE);
            }
        }

        // 盾牌 + 勾：盾面上底色，勾全色压在上面
        Icon::Safety => {
            fill_stroke(
                line(&[
                    (0.50, 0.08),
                    (0.85, 0.22),
                    (0.85, 0.50),
                    (0.50, 0.92),
                    (0.15, 0.50),
                    (0.15, 0.22),
                ]),
                0.22,
            );
            painter.add(Shape::line(
                line(&[(0.33, 0.48), (0.45, 0.62), (0.68, 0.36)]),
                stroke,
            ));
        }

        // 放大镜：镜片上底色 + 斜柄
        Icon::Scan => {
            let r = box_.width() * 0.27;
            painter.circle(p(0.42, 0.42), r, tint(0.22), stroke);
            painter.line_segment([p(0.62, 0.62), p(0.88, 0.88)], stroke);
        }

        // 重复文件：后面一张折角文档错位叠着，前面一张上底色
        Icon::Duplicates => {
            // 后一张（往右上偏）
            painter.add(Shape::line(
                line(&[(0.36, 0.08), (0.66, 0.08), (0.82, 0.24), (0.82, 0.70)]),
                stroke,
            ));
            // 前一张
            fill_stroke(
                line(&[
                    (0.18, 0.26),
                    (0.50, 0.26),
                    (0.66, 0.42),
                    (0.66, 0.92),
                    (0.18, 0.92),
                ]),
                0.20,
            );
            painter.add(Shape::line(
                line(&[(0.50, 0.26), (0.50, 0.42), (0.66, 0.42)]),
                stroke,
            ));
            painter.line_segment([p(0.30, 0.64), p(0.54, 0.64)], stroke);
            painter.line_segment([p(0.30, 0.78), p(0.48, 0.78)], stroke);
        }

        // 显示器：屏幕 + 立柱 + 底座横线，语义是"系统/这台电脑"，
        // 和「设置」页的滑杆图标区分开（齿轮容易被读成设置）。
        // 屏幕上底色后就不再画里面的"面板线" —— 17px 下会糊。
        Icon::System => {
            let screen = Rect::from_min_max(p(0.10, 0.14), p(0.90, 0.64));
            painter.rect_filled(screen, egui::CornerRadius::same(3), tint(0.22));
            painter.rect_stroke(
                screen,
                egui::CornerRadius::same(3),
                stroke,
                egui::StrokeKind::Middle,
            );
            painter.line_segment([p(0.50, 0.64), p(0.50, 0.80)], stroke);
            painter.line_segment([p(0.28, 0.87), p(0.72, 0.87)], stroke);
        }

        // 微信 / QQ：直接画 assets 里的品牌 SVG。
        // 深色主题下补一块浅色圆角底板 —— 企鹅的身体是纯黑填充，直接放在深色
        // 侧栏上会整个融进背景；浅色主题不画，保持和参考稿一致的"裸图标"。
        Icon::Wechat | Icon::Qq => {
            if painter.ctx().theme() == egui::Theme::Dark {
                let plate = box_.expand(side * 0.11);
                painter.rect_filled(
                    plate,
                    egui::CornerRadius::same((side * 0.24).round() as u8),
                    Color32::from_rgba_unmultiplied(247, 249, 252, 238),
                );
            }
            let (name, svg) = match icon {
                Icon::Wechat => ("wechat", crate::ui::svg_icon::WECHAT_SVG),
                _ => ("qq", crate::ui::svg_icon::QQ_SVG),
            };
            crate::ui::svg_icon::paint(painter, box_, name, svg);
        }
    }
}

/// 画一个「浅色圆角底 + 线性图标」的小方块，用作列表项或入口卡的图标位。
pub fn paint_chip(
    painter: &egui::Painter,
    rect: Rect,
    icon: Icon,
    bg: Color32,
    fg: Color32,
    radius: f32,
) {
    painter.rect_filled(rect, egui::CornerRadius::same(radius.round() as u8), bg);
    // 图标占方块的一半多一点，四周留出呼吸边
    let inner = rect.shrink(rect.width() * 0.26);
    paint(painter, inner, icon, fg, (rect.width() * 0.072).max(1.3));
}
