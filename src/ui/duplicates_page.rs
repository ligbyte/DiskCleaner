//! 重复文件页：扫描 → 按组列出完全相同的文件 → 勾选多余副本 → 移入回收站。
//!
//! 扫描由 [`crate::duplicates`] 完成（只读）；删除走
//! [`crate::bigfiles::recycle_one`] 同一套安全校验，必须经过确认弹窗。

use eframe::egui::{self, Align, Layout, RichText};

use crate::app::{App, DupState};
use crate::model::human_bytes;
use crate::theme::Palette;
use crate::ui::widgets::{self, card, pill};

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let p = Palette::of(ui);

    // 先把状态数据**拷出来**再渲染：渲染过程中要可变借用 app（勾选、起扫），
    // 直接 `match &app.dup` 会把不可变借用一直吊着。
    enum DupView {
        Idle,
        Running {
            scanned: u64,
        },
        Done {
            groups: Vec<crate::duplicates::DupGroup>,
            scanned: u64,
            elapsed_ms: u64,
            canceled: bool,
        },
    }
    let view = match &app.dup {
        DupState::Idle => DupView::Idle,
        DupState::Running { scanned, .. } => DupView::Running { scanned: *scanned },
        DupState::Done {
            groups,
            scanned,
            elapsed_ms,
            canceled,
        } => DupView::Done {
            groups: groups.clone(),
            scanned: *scanned,
            elapsed_ms: *elapsed_ms,
            canceled: *canceled,
        },
    };

    match view {
        DupView::Idle => {
            intro_card(app, ui, &p);
        }
        DupView::Running { scanned } => {
            running_card(app, ui, &p, scanned);
        }
        DupView::Done {
            groups,
            scanned,
            elapsed_ms,
            canceled,
        } => {
            intro_card(app, ui, &p);
            ui.add_space(12.0);
            if groups.is_empty() {
                card(ui, &p, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(
                        RichText::new(if canceled {
                            "扫描已取消，没有可显示的结果"
                        } else {
                            "没有发现重复文件"
                        })
                        .size(13.5)
                        .strong()
                        .color(p.text),
                    );
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(format!(
                            "共检查了 {} 个 10MB 以上的文件，耗时 {}",
                            scanned,
                            format_elapsed(elapsed_ms)
                        ))
                        .size(12.0)
                        .color(p.text_weak),
                    );
                });
            } else {
                result_list(app, ui, &p, &groups);
                ui.add_space(14.0);
                footer(app, ui, &p);
                ui.add_space(4.0);
                ui.label(
                    RichText::new(format!(
                        "共检查 {} 个文件 · 耗时 {} · 每组默认保留最早的一份",
                        scanned,
                        format_elapsed(elapsed_ms)
                    ))
                    .size(11.5)
                    .color(p.text_faint),
                );
            }
        }
    }
}

/// 顶部说明卡：这个功能做什么 + 开始按钮。
fn intro_card(app: &mut App, ui: &mut egui::Ui, p: &Palette) {
    card(ui, &p, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 3.0;
                ui.label(
                    RichText::new("找出内容完全相同的文件")
                        .size(15.0)
                        .strong()
                        .color(p.text),
                );
                ui.label(
                    RichText::new(format!(
                        "在「大文件」页设置的扫描目录里，把 10MB 以上、内容一模一样的文件归成一组；\
                         每组保留一份，多余的可以移入回收站。"
                    ))
                    .size(12.0)
                    .color(p.text_weak),
                );
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if widgets::primary_button(ui, p, "开始扫描", true).clicked() {
                    app.start_dup_scan();
                }
            });
        });
    });
}

/// 扫描进行中的卡片。
fn running_card(app: &mut App, ui: &mut egui::Ui, p: &Palette, scanned: u64) {
    card(ui, &p, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 3.0;
                ui.label(
                    RichText::new("正在扫描重复文件…")
                        .size(15.0)
                        .strong()
                        .color(p.text),
                );
                ui.label(
                    RichText::new(format!(
                        "已检查 {scanned} 个文件。逐字节比较可能需要一点时间。"
                    ))
                    .size(12.0)
                    .color(p.text_weak),
                );
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if widgets::ghost_button(ui, p, "取消").clicked() {
                    app.cancel_dup_scan();
                }
            });
        });
    });
}

/// 结果列表：每组一张卡。
fn result_list(
    app: &mut App,
    ui: &mut egui::Ui,
    p: &Palette,
    groups: &[crate::duplicates::DupGroup],
) {
    for (index, group) in groups.iter().enumerate() {
        let keep = group.keep_index();
        let all_checked = group
            .files
            .iter()
            .enumerate()
            .all(|(i, f)| i == keep || app.dup_selected.contains(&f.path));

        card(ui, &p, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let mut group_checked = all_checked;
                if ui
                    .checkbox(
                        &mut group_checked,
                        RichText::new(format!(
                            "第 {} 组 · {} 份副本",
                            index + 1,
                            group.files.len()
                        ))
                        .size(13.5)
                        .strong()
                        .color(p.text),
                    )
                    .changed()
                {
                    app.toggle_dup_group(group, group_checked);
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    pill(
                        ui,
                        &format!("可释放 {}", human_bytes(group.reclaimable())),
                        crate::theme::PillKind::Accent,
                        p,
                    );
                });
            });
            ui.add_space(8.0);

            for (i, file) in group.files.iter().enumerate() {
                let checked = app.dup_selected.contains(&file.path);
                ui.horizontal(|ui| {
                    let keep_label = i == keep;
                    let mut row_checked = checked;
                    if ui.checkbox(&mut row_checked, "").changed() {
                        app.toggle_dup(&file.path);
                    }
                    ui.label(
                        RichText::new(short_path(&file.path.to_string_lossy()))
                            .size(12.0)
                            .color(if keep_label { p.text_weak } else { p.text }),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if keep_label {
                            pill(ui, "保留", crate::theme::PillKind::Safe, p);
                        }
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new(human_bytes(file.size))
                                .size(11.5)
                                .color(p.text_faint),
                        );
                    });
                });
            }
        });
        ui.add_space(10.0);
    }
}

/// 底部动作条：汇总 + 确认按钮。
fn footer(app: &mut App, ui: &mut egui::Ui, p: &Palette) {
    let count = app.dup_selected.len();
    let bytes: u64 = app
        .dup_selected
        .iter()
        .filter_map(|path| std::fs::metadata(path).ok())
        .map(|m| m.len())
        .sum();

    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!(
                "已勾选 {count} 个副本，将释放 {}",
                human_bytes(bytes)
            ))
            .size(12.5)
            .color(p.text_weak),
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if widgets::primary_button(ui, p, "移入回收站", count > 0).clicked() {
                app.request_recycle_duplicates();
            }
        });
    });
}

/// 路径太长只保留头尾。
fn short_path(text: &str) -> String {
    const MAX_CHARS: usize = 72;
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= MAX_CHARS {
        return text.to_string();
    }
    let keep = (MAX_CHARS - 1) / 2;
    let head: String = chars[..keep].iter().collect();
    let tail: String = chars[chars.len() - keep..].iter().collect();
    format!("{head}…{tail}")
}

fn format_elapsed(ms: u64) -> String {
    let secs = ms / 1000;
    if secs >= 60 {
        format!("{} 分 {} 秒", secs / 60, secs % 60)
    } else {
        format!("{secs} 秒")
    }
}
