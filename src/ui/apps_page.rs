//! 已安装软件页：统计占用空间、搜索排序、一键卸载。

use std::path::PathBuf;

use eframe::egui::{self, Align, Color32, CornerRadius, Layout, RichText, Stroke};

use crate::app::{App, AppScanState, AppsSort};
use crate::model::human_bytes;
use crate::theme::{CARD_RADIUS, Palette, PillKind};
use crate::ui::widgets::{self, card, ghost_button, pill, primary_button};

/// 每帧收集的交互动作，避免在遍历列表时可变借用 `app`。
enum Action {
    Scan,
    Cancel,
    Uninstall(usize),
    Reveal(PathBuf),
}

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let p = Palette::of(ui);
    let mut actions: Vec<Action> = Vec::new();

    toolbar(app, ui, &p, &mut actions);
    ui.add_space(10.0);

    if app.apps_scanning() {
        scan_progress(app, ui, &p);
        ui.add_space(10.0);
    }

    list(app, ui, &p, &mut actions);

    // ---- 应用动作 ----
    for action in actions {
        match action {
            Action::Scan => app.start_apps_scan(),
            Action::Cancel => app.cancel_apps_scan(),
            Action::Uninstall(index) => app.request_uninstall(index),
            Action::Reveal(path) => crate::sysutil::reveal_in_file_manager(&path),
        }
    }
}

// ---------------------------------------------------------------------------
// 顶部操作区
// ---------------------------------------------------------------------------

fn toolbar(app: &mut App, ui: &mut egui::Ui, p: &Palette, actions: &mut Vec<Action>) {
    card(ui, p, |ui| {
        ui.set_width(ui.available_width());

        ui.horizontal(|ui| {
            let scanning = app.apps_scanning();
            let label = if scanning {
                "统计中…"
            } else if app.apps.is_empty() {
                "扫描已安装软件"
            } else {
                "重新扫描"
            };
            if primary_button(ui, p, label, !scanning && !app.busy()).clicked() {
                actions.push(Action::Scan);
            }

            if scanning && ghost_button(ui, p, "取消").clicked() {
                actions.push(Action::Cancel);
            }

            ui.add_space(4.0);
            ui.add(
                egui::TextEdit::singleline(&mut app.apps_query)
                    .hint_text("搜索软件名称 / 发布者")
                    .desired_width(220.0),
            );

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.checkbox(
                    &mut app.apps_only_uninstallable,
                    RichText::new("只看可卸载").size(12.0),
                );
            });
        });

        ui.add_space(10.0);
        widgets::divider(ui, p);
        ui.add_space(2.0);

        ui.horizontal(|ui| {
            ui.label(RichText::new("排序").size(12.0).color(p.text_weak));
            for sort in AppsSort::ALL {
                let selected = app.apps_sort == sort;
                let btn =
                    egui::Button::new(RichText::new(sort.label()).size(12.0).color(if selected {
                        Color32::WHITE
                    } else {
                        p.text
                    }))
                    .fill(if selected { p.accent } else { p.surface })
                    .stroke(Stroke::new(1.0, if selected { p.accent } else { p.border }))
                    .corner_radius(CornerRadius::same(CARD_RADIUS));
                if ui.add(btn).clicked() {
                    app.apps_sort = sort;
                }
            }

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if let AppScanState::Done {
                    elapsed_ms,
                    canceled,
                } = &app.apps_scan
                {
                    if *canceled {
                        pill(ui, "统计已取消", PillKind::Caution, p);
                    }
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(format!("用时 {:.1}s", *elapsed_ms as f32 / 1000.0))
                            .size(11.5)
                            .color(p.text_faint),
                    );
                }
                if !app.apps.is_empty() {
                    let (count, total) = app.apps_summary();
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new(format!(
                            "{} 款软件 · 合计 {}",
                            crate::model::thousands(count),
                            human_bytes(total)
                        ))
                        .size(12.0)
                        .color(p.text_weak),
                    );
                }
            });
        });
    });
}

// ---------------------------------------------------------------------------
// 扫描进度
// ---------------------------------------------------------------------------

fn scan_progress(app: &App, ui: &mut egui::Ui, p: &Palette) {
    let (phase, measured, total, started) = match &app.apps_scan {
        AppScanState::Running {
            phase,
            measured,
            total,
            started,
            ..
        } => (phase.clone(), *measured, *total, *started),
        _ => return,
    };

    card(ui, p, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("正在统计占用空间")
                    .size(13.5)
                    .strong()
                    .color(p.text),
            );
            ui.add_space(4.0);
            ui.label(
                RichText::new(if total == 0 {
                    phase.clone()
                } else {
                    format!("{measured} / {total} 款")
                })
                .size(12.0)
                .color(p.text_weak),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(
                    RichText::new(format!("{:.1}s", started.elapsed().as_secs_f32()))
                        .size(12.0)
                        .color(p.text_faint),
                );
            });
        });
        ui.add_space(8.0);
        let ratio = if total == 0 {
            0.0
        } else {
            measured as f32 / total as f32
        };
        widgets::progress_bar(ui, p, ratio, 8.0, p.accent);
        ui.add_space(6.0);
        ui.label(
            RichText::new(crate::apps::size_note())
                .size(11.5)
                .color(p.text_faint),
        );
    });
}

// ---------------------------------------------------------------------------
// 列表
// ---------------------------------------------------------------------------

fn list(app: &App, ui: &mut egui::Ui, p: &Palette, actions: &mut Vec<Action>) {
    if app.apps.is_empty() {
        empty_state(app, ui, p);
        return;
    }

    let order = app.apps_view();
    if order.is_empty() {
        card(ui, p, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                RichText::new("没有符合条件的软件，换个关键词试试。")
                    .size(12.5)
                    .color(p.text_weak),
            );
        });
        return;
    }

    for index in order {
        app_row(app, ui, p, index, actions);
        ui.add_space(8.0);
    }
}

fn app_row(app: &App, ui: &mut egui::Ui, p: &Palette, index: usize, actions: &mut Vec<Action>) {
    let Some(item) = app.apps.get(index) else {
        return;
    };

    card(ui, p, |ui| {
        ui.set_width(ui.available_width());

        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&item.name).size(14.0).strong().color(p.text));
                    if !item.version.is_empty() {
                        pill(ui, &item.version, PillKind::Neutral, p);
                    }
                    if item.needs_elevation() {
                        pill(
                            ui,
                            &format!("需{}", crate::sysutil::privilege_label()),
                            PillKind::Caution,
                            p,
                        );
                    }
                    if !item.can_uninstall() {
                        pill(ui, "不可卸载", PillKind::Neutral, p);
                    }
                });
                if !item.publisher.is_empty() {
                    ui.add_space(2.0);
                    ui.label(RichText::new(&item.publisher).size(11.5).color(p.text_weak));
                }
            });

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                match item.size_bytes() {
                    Some(bytes) => {
                        let text = if item.size_is_estimated() {
                            format!("≈ {}", human_bytes(bytes))
                        } else {
                            human_bytes(bytes)
                        };
                        ui.label(RichText::new(text).size(17.0).strong().color(p.text));
                        if item.size_is_estimated() {
                            pill(ui, "估算", PillKind::Neutral, p);
                        }
                    }
                    None => {
                        ui.label(RichText::new("—").size(17.0).strong().color(p.text_faint))
                            .on_hover_text("该软件与其它软件共用安装目录，无法单独统计体积");
                    }
                }
            });
        });

        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let location = item
                .install_location
                .as_ref()
                .map(|path| shorten_path(path, 58))
                .unwrap_or_else(|| "（未登记安装目录）".to_string());
            ui.label(
                RichText::new(location)
                    .size(11.5)
                    .monospace()
                    .color(p.text_faint),
            );

            if let Some(date) = item.install_date_label() {
                ui.add_space(8.0);
                ui.label(
                    RichText::new(format!("安装于 {date}"))
                        .size(11.5)
                        .color(p.text_faint),
                );
            }

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if item.can_uninstall() {
                    let btn = egui::Button::new(
                        RichText::new("卸载")
                            .size(12.5)
                            .strong()
                            .color(Color32::WHITE),
                    )
                    .fill(p.danger)
                    .stroke(Stroke::NONE)
                    .corner_radius(CornerRadius::same(CARD_RADIUS))
                    .min_size(egui::vec2(64.0, 28.0));
                    if ui
                        .add(btn)
                        .on_hover_text(item.uninstall.short_label())
                        .clicked()
                    {
                        actions.push(Action::Uninstall(index));
                    }
                }

                if let Some(location) = &item.install_location {
                    if location.is_dir() {
                        let btn =
                            egui::Button::new(RichText::new("打开目录").size(12.0).color(p.text))
                                .fill(p.surface)
                                .stroke(Stroke::new(1.0, p.border))
                                .corner_radius(CornerRadius::same(CARD_RADIUS))
                                .min_size(egui::vec2(72.0, 28.0));
                        if ui.add(btn).clicked() {
                            actions.push(Action::Reveal(location.clone()));
                        }
                    }
                }
            });
        });
    });
}

fn empty_state(app: &App, ui: &mut egui::Ui, p: &Palette) {
    card(ui, p, |ui| {
        ui.set_width(ui.available_width());
        ui.vertical_centered(|ui| {
            ui.add_space(14.0);
            ui.label(
                RichText::new("还没有扫描")
                    .size(15.0)
                    .strong()
                    .color(p.text),
            );
            ui.add_space(6.0);
            let text = if app.apps_scanning() {
                "正在读取系统里的软件清单，请稍候…".to_string()
            } else {
                format!(
                    "点上面的「扫描已安装软件」，会列出系统里已安装的程序与它们占用的空间。\n\
                     扫描只读取{}与目录元数据，不会修改或卸载任何东西。",
                    crate::apps::source_label()
                )
            };
            ui.label(RichText::new(text).size(12.5).color(p.text_weak));
            ui.add_space(14.0);
        });
    });
}

// ---------------------------------------------------------------------------
// 小工具
// ---------------------------------------------------------------------------

/// 路径太长时保留首尾，中间用 `…` 代替。
fn shorten_path(path: &std::path::Path, max_chars: usize) -> String {
    let text = path.to_string_lossy().to_string();
    let count = text.chars().count();
    if count <= max_chars {
        return text;
    }

    let head: String = text.chars().take(max_chars / 2).collect();
    let tail: String = text
        .chars()
        .skip(count - (max_chars / 2).saturating_sub(1))
        .collect();
    format!("{head}…{tail}")
}
