//! 线性图标：直接用 egui 的图元画出来。
//!
//! 为什么不引图标字体或 PNG 资源：
//!
//! * **单文件分发**是这个项目的一条底线 —— 多一个字体文件就多一个可能丢失的
//!   附件；用 painter 画则一个字节都不用带。
//! * 需要的图标只有 6 个，形状又都很简单，手画比维护一套字体子集划算。
//! * 图标能直接跟着主题色、描边粗细走，深浅两套主题不用各备一套资源。
//!
//! 每个图标都在**归一化的 0..1 方格**里描述，再映射到传入的 `Rect`，
//! 所以调用方给多大就画多大，比例永远不变。

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
}

/// 把一个图标画进 `rect`（取短边为准，居中）。
///
/// `width` 是线宽，`color` 是线的颜色。
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

    match icon {
        // 三层菱形堆叠：上面一个完整的菱形，下面两道"折线"
        Icon::Overview => {
            let diamond = line(&[(0.50, 0.10), (0.90, 0.30), (0.50, 0.50), (0.10, 0.30)]);
            painter.add(Shape::closed_line(diamond, stroke));
            painter.add(Shape::line(
                line(&[(0.10, 0.50), (0.50, 0.68), (0.90, 0.50)]),
                stroke,
            ));
            painter.add(Shape::line(
                line(&[(0.10, 0.68), (0.50, 0.86), (0.90, 0.68)]),
                stroke,
            ));
        }

        // 垃圾桶：盖 + 提手 + 桶身 + 两道竖纹
        Icon::Clean => {
            painter.line_segment([p(0.13, 0.28), p(0.87, 0.28)], stroke);
            painter.add(Shape::line(
                line(&[(0.36, 0.28), (0.36, 0.17), (0.64, 0.17), (0.64, 0.28)]),
                stroke,
            ));
            painter.add(Shape::line(
                line(&[(0.21, 0.28), (0.27, 0.87), (0.73, 0.87), (0.79, 0.28)]),
                stroke,
            ));
            painter.line_segment([p(0.42, 0.42), p(0.44, 0.75)], stroke);
            painter.line_segment([p(0.58, 0.42), p(0.56, 0.75)], stroke);
        }

        // 折角文档 + 两道内容线
        Icon::BigFiles => {
            painter.add(Shape::closed_line(
                line(&[
                    (0.24, 0.10),
                    (0.60, 0.10),
                    (0.78, 0.30),
                    (0.78, 0.90),
                    (0.24, 0.90),
                ]),
                stroke,
            ));
            painter.add(Shape::line(
                line(&[(0.60, 0.10), (0.60, 0.30), (0.78, 0.30)]),
                stroke,
            ));
            painter.line_segment([p(0.37, 0.52), p(0.66, 0.52)], stroke);
            painter.line_segment([p(0.37, 0.68), p(0.58, 0.68)], stroke);
        }

        // 立方体
        Icon::Apps => {
            painter.add(Shape::closed_line(
                line(&[(0.50, 0.09), (0.87, 0.29), (0.50, 0.49), (0.13, 0.29)]),
                stroke,
            ));
            painter.add(Shape::line(
                line(&[
                    (0.13, 0.29),
                    (0.13, 0.71),
                    (0.50, 0.91),
                    (0.87, 0.71),
                    (0.87, 0.29),
                ]),
                stroke,
            ));
            painter.line_segment([p(0.50, 0.49), p(0.50, 0.91)], stroke);
        }

        // 三条滑杆：线在圆钮处断开，再画一个圆钮
        Icon::Settings => {
            for (y, knob) in [(0.24_f32, 0.34_f32), (0.50, 0.66), (0.76, 0.44)] {
                let r = 0.10;
                painter.line_segment([p(0.12, y), p(knob - r, y)], stroke);
                painter.line_segment([p(knob + r, y), p(0.88, y)], stroke);
                painter.circle_stroke(p(knob, y), box_.width() * r, stroke);
            }
        }

        // 盾牌 + 勾
        Icon::Safety => {
            painter.add(Shape::closed_line(
                line(&[
                    (0.50, 0.08),
                    (0.85, 0.22),
                    (0.85, 0.50),
                    (0.50, 0.92),
                    (0.15, 0.50),
                    (0.15, 0.22),
                ]),
                stroke,
            ));
            painter.add(Shape::line(
                line(&[(0.34, 0.48), (0.45, 0.61), (0.67, 0.36)]),
                stroke,
            ));
        }

        // 放大镜：镜片 + 斜柄，镜片里一道小横线
        Icon::Scan => {
            painter.circle_stroke(p(0.42, 0.42), box_.width() * 0.27, stroke);
            painter.line_segment([p(0.62, 0.62), p(0.88, 0.88)], stroke);
            painter.line_segment([p(0.30, 0.42), p(0.54, 0.42)], stroke);
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
