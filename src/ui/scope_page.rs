//! 专项清理页（系统设置 / 微信 / QQ 共用）。
//!
//! 三页长得一样、只是扫描范围（[`Page::scope`]）不同：一张说明卡、
//! 一个扫描按钮、按分组排列的目标清单、底部一颗「一键清理」。
//! 复用全局的 `scan` / `clean` 状态机 —— 扫描结果按分组过滤展示，
//! 确认弹窗也只提交本页范围内的目标。

use eframe::egui::{self, Align, Color32, Layout, RichText, Sense, Vec2};

use crate::app::{App, Page, ScanState};
use crate::model::{CleanTarget, human_bytes};
use crate::theme::{Palette, PillKind};
use crate::ui::icons;
use crate::ui::widgets::{self, card, pill};

/// 页面对应的图标章底色。
fn tint(page: Page) -> Color32 {
    match page {
        Page::System => crate::ui::overview::TINT_VIOLET,
        Page::Wechat => Color32::from_rgb(0x2A, 0xA8, 0x5C),
        Page::Qq => Color32::from_rgb(0x2B, 0x9E, 0xD8),
        _ => crate::ui::overview::TINT_SLATE,
    }
}

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let p = Palette::of(ui);
    let page = app.page;
    let Some(groups) = page.scope() else {
        return;
    };

    // 本页范围内的目标（按分组稳定排列）
    let targets: Vec<CleanTarget> = app
        .targets
        .iter()
        .filter(|t| groups.contains(&t.group))
        .cloned()
        .collect();

    // ---- 说明卡 ----
    card(ui, &p, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(44.0), Sense::hover());
            icons::paint_chip(
                ui.painter(),
                rect,
                icon_for(page),
                crate::theme::mix(p.surface, tint(page), 0.16),
                tint(page),
                13.0,
            );
            ui.add_space(12.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.label(
                    RichText::new(page.title())
                        .size(15.0)
                        .strong()
                        .color(p.text),
                );
                ui.label(RichText::new(page.subtitle()).size(12.0).color(p.text_weak));
            });
        });
    });
    ui.add_space(12.0);

    let scanning = app.scanning();

    // ---- 空态：本机没有对应目录 ----
    if targets.is_empty() {
        card(ui, &p, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                RichText::new("本机没有找到可清理的目录")
                    .size(13.5)
                    .strong()
                    .color(p.text),
            );
            ui.add_space(4.0);
            ui.label(
                RichText::new("可能没有安装，或数据目录被自定义到了其它位置。")
                    .size(12.0)
                    .color(p.text_weak),
            );
        });
        return;
    }

    // ---- 目标清单（按分组分节） ----
    card(ui, &p, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 8.0;

        let total_targets = targets.len();
        for (i, target) in targets.iter().enumerate() {
            if i > 0 {
                widgets::divider(ui, &p);
            }
            // 分组标题：只有换组时才画
            if i == 0 || targets[i - 1].group != target.group {
                ui.label(
                    RichText::new(target.group.title())
                        .size(12.0)
                        .strong()
                        .color(p.text_weak),
                );
            }
            target_row(app, ui, &p, target, i == total_targets - 1);
        }
    });
    ui.add_space(14.0);

    // ---- 底部动作条 ----
    let (_, _, bytes) = app.pending_selection_in(groups);
    let done = matches!(app.scan, ScanState::Done(_));

    ui.horizontal(|ui| {
        // 左：状态说明
        if scanning {
            ui.label(RichText::new("正在扫描…").size(12.5).color(p.text_weak));
        } else if done {
            ui.label(
                RichText::new(format!(
                    "可清理 {}（勾选 {} 项）",
                    human_bytes(bytes),
                    app.selected
                        .iter()
                        .filter(|id| targets.iter().any(|t| &t.id == *id))
                        .count()
                ))
                .size(12.5)
                .color(p.text_weak),
            );
        } else {
            ui.label(
                RichText::new("扫描只读取文件信息，不会删除任何东西")
                    .size(12.5)
                    .color(p.text_faint),
            );
        }

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let label = if scanning {
                "取消扫描"
            } else if done {
                "重新扫描"
            } else {
                "开始扫描"
            };
            if scanning {
                if widgets::ghost_button(ui, &p, label).clicked() {
                    app.cancel_scan();
                }
            } else if widgets::primary_button(ui, &p, label, true).clicked() {
                app.start_scan_scoped(groups);
            }
            ui.add_space(8.0);
            if done {
                if widgets::primary_button(ui, &p, "一键清理", bytes > 0).clicked() {
                    app.request_clean_scoped(groups);
                }
            }
        });
    });
}

/// 一行清理目标：勾选框 + 名称/说明 + 右侧状态（体积或"未扫描"）。
fn target_row(
    app: &mut App,
    ui: &mut egui::Ui,
    p: &Palette,
    target: &CleanTarget,
    _last: bool,
) -> egui::Response {
    ui.horizontal(|ui| {
        let mut checked = app.selected.contains(target.id);
        if ui
            .checkbox(
                &mut checked,
                RichText::new(target.name).size(13.0).color(p.text),
            )
            .changed()
        {
            app.toggle_target(target.id);
        }
        ui.add_space(6.0);
        ui.label(RichText::new(target.desc).size(11.0).color(p.text_faint));

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            match app.result_for(target.id) {
                Some(r) if r.files > 0 => {
                    let kind = risk_kind(target.risk);
                    pill(
                        ui,
                        &format!("{} · {} 个文件", human_bytes(r.bytes), r.files),
                        kind,
                        p,
                    );
                }
                Some(_) => {
                    pill(ui, "没有可清理的内容", PillKind::Neutral, p);
                }
                None => {
                    pill(ui, "未扫描", PillKind::Neutral, p);
                }
            }
        });
    })
    .response
}

fn icon_for(page: Page) -> icons::Icon {
    match page {
        Page::Wechat => icons::Icon::Wechat,
        Page::Qq => icons::Icon::Qq,
        _ => icons::Icon::System,
    }
}

/// 风险等级 → 胶囊色（体积标签跟着风险走，红的显眼）。
fn risk_kind(risk: crate::model::RiskLevel) -> PillKind {
    match risk {
        crate::model::RiskLevel::Safe => PillKind::Safe,
        crate::model::RiskLevel::Caution => PillKind::Caution,
        crate::model::RiskLevel::Advanced => PillKind::Danger,
    }
}
