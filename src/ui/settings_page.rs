//! 设置页。

use eframe::egui::{self, Align, Layout, RichText};

use crate::app::{App, ToastKind};
use crate::cleaner::CleanMode;
use crate::settings::ThemePref;
use crate::theme::{Palette, PillKind};
use crate::ui::widgets::{self, card, pill};

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let p = Palette::of(ui);

    appearance(app, ui, &p);
    ui.add_space(10.0);
    strategy(app, ui, &p);
    ui.add_space(10.0);
    big_file_settings(app, ui, &p);
    ui.add_space(10.0);
    storage(app, ui, &p);
    ui.add_space(10.0);
    about(ui, &p);
    ui.add_space(10.0);
    reset(app, ui, &p);
}

fn appearance(app: &mut App, ui: &mut egui::Ui, p: &Palette) {
    card(ui, p, |ui| {
        ui.set_width(ui.available_width());
        widgets::section_title(ui, p, "外观");
        ui.add_space(4.0);
        ui.label(
            RichText::new("界面会跟随系统主题自动切换，也可以手动固定。")
                .size(12.0)
                .color(p.text_weak),
        );
        ui.add_space(10.0);

        ui.horizontal(|ui| {
            // 和顶栏右上角那种切换是同一个控件（`widgets::segmented`），
            // 同一件事在两处不能长成两个样子
            let labels: Vec<&str> = ThemePref::ALL.iter().map(|pref| pref.label()).collect();
            let selected = ThemePref::ALL
                .iter()
                .position(|pref| *pref == app.settings.theme)
                .unwrap_or(0);
            if let Some(index) =
                widgets::segmented(ui, p, &labels, selected, widgets::SEGMENTED_HEIGHT)
            {
                app.apply_theme(ThemePref::ALL[index], ui.ctx());
            }
        });
    });
}

fn strategy(app: &mut App, ui: &mut egui::Ui, p: &Palette) {
    card(ui, p, |ui| {
        ui.set_width(ui.available_width());
        widgets::section_title(ui, p, "清理策略");
        ui.add_space(10.0);

        ui.horizontal(|ui| {
            ui.label(RichText::new("默认删除方式").size(12.5).color(p.text_weak));
            for mode in [CleanMode::RecycleBin, CleanMode::Permanent] {
                let selected = app.settings.clean_mode == mode;
                let accent = if mode == CleanMode::Permanent {
                    p.danger
                } else {
                    p.accent
                };
                let btn =
                    egui::Button::new(RichText::new(mode.label()).size(12.5).color(if selected {
                        egui::Color32::WHITE
                    } else {
                        p.text
                    }))
                    .fill(if selected { accent } else { p.surface })
                    .stroke(egui::Stroke::new(
                        1.0,
                        if selected { accent } else { p.border },
                    ))
                    .corner_radius(egui::CornerRadius::same(crate::theme::CARD_RADIUS));
                if ui.add(btn).on_hover_text(mode.hint()).clicked() {
                    app.settings.clean_mode = mode;
                    app.touch_settings();
                }
            }

            if !crate::recycle::is_available() {
                ui.add_space(10.0);
                crate::ui::widgets::pill(ui, "回收站不可用", crate::theme::PillKind::Caution, p);
            }
        });

        if !crate::recycle::is_available() {
            ui.add_space(6.0);
            ui.label(
                RichText::new(
                    "本机回收站被禁用或不可用（已用探针文件实测）。选择「移入回收站」时会有大量文件被跳过。",
                )
                .size(11.5)
                .color(p.caution),
            );
        }

        ui.add_space(12.0);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("临时文件最小年龄")
                    .size(12.5)
                    .color(p.text_weak),
            );
            for preset in [0u32, 1, 3, 7, 30] {
                let selected = app.settings.min_age_days == preset;
                let label = if preset == 0 {
                    "全部".to_string()
                } else {
                    format!("{preset} 天")
                };
                let btn = egui::Button::new(RichText::new(label).size(12.5).color(if selected {
                    egui::Color32::WHITE
                } else {
                    p.text
                }))
                .fill(if selected { p.accent } else { p.surface })
                .stroke(egui::Stroke::new(
                    1.0,
                    if selected { p.accent } else { p.border },
                ))
                .corner_radius(egui::CornerRadius::same(crate::theme::CARD_RADIUS));
                if ui.add(btn).clicked() {
                    app.settings.min_age_days = preset;
                    app.touch_settings();
                }
            }
        });
        ui.add_space(4.0);
        ui.label(
            RichText::new(
                "只影响标记了「临时文件」性质的目录（如 %TEMP%、系统临时目录、更新缓存）。",
            )
            .size(11.5)
            .color(p.text_faint),
        );

        ui.add_space(12.0);
        let mut dry = app.settings.dry_run;
        if ui
            .checkbox(
                &mut dry,
                RichText::new("演练模式（只统计，不删除）").size(12.5),
            )
            .on_hover_text("开启后所有清理操作只统计体积与数量，不真正删除文件")
            .changed()
        {
            app.settings.dry_run = dry;
            app.touch_settings();
        }
        if app.settings.dry_run {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                pill(ui, "当前为演练模式", PillKind::Caution, p);
                ui.label(
                    RichText::new("适合第一次使用前先看看会删掉什么")
                        .size(11.5)
                        .color(p.text_faint),
                );
            });
        }
    });
}

fn big_file_settings(app: &mut App, ui: &mut egui::Ui, p: &Palette) {
    card(ui, p, |ui| {
        ui.set_width(ui.available_width());
        widgets::section_title(ui, p, "大文件");
        ui.add_space(4.0);
        ui.label(
            RichText::new("大文件页的默认体积阈值，以及默认扫描目录。")
                .size(12.0)
                .color(p.text_weak),
        );
        ui.add_space(10.0);

        ui.horizontal(|ui| {
            ui.label(RichText::new("体积阈值").size(12.5).color(p.text_weak));
            let mut mb = app.big_min_mb as f32;
            if ui
                .add(
                    egui::Slider::new(&mut mb, 10.0..=4096.0)
                        .suffix(" MB")
                        .logarithmic(true),
                )
                .changed()
            {
                app.big_min_mb = mb.round() as u64;
                app.touch_settings();
            }
        });

        ui.add_space(8.0);
        ui.label(RichText::new("默认扫描目录").size(12.5).color(p.text_weak));
        ui.add_space(4.0);
        for root in &app.big_roots {
            ui.label(
                RichText::new(format!("· {}", root.to_string_lossy()))
                    .size(11.5)
                    .color(p.text_weak),
            );
        }
        ui.add_space(4.0);
        ui.label(
            RichText::new("目录列表可以到「大文件」页里增删。")
                .size(11.0)
                .color(p.text_faint),
        );
    });
}

fn storage(app: &mut App, ui: &mut egui::Ui, p: &Palette) {
    card(ui, p, |ui| {
        ui.set_width(ui.available_width());
        widgets::section_title(ui, p, "数据与日志");
        ui.add_space(8.0);

        widgets::kv_row(
            ui,
            p,
            "配置文件",
            &crate::sysutil::settings_path().to_string_lossy(),
        );
        ui.add_space(4.0);
        widgets::kv_row(
            ui,
            p,
            "日志目录",
            &crate::sysutil::logs_dir().to_string_lossy(),
        );
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if widgets::ghost_button(ui, p, "打开日志目录").clicked() {
                let dir = crate::sysutil::logs_dir();
                crate::sysutil::ensure_dir(&dir);
                crate::sysutil::open_path(&dir);
            }
            if widgets::ghost_button(ui, p, "立即保存设置").clicked() {
                app.touch_settings();
                app.settings.selected_targets =
                    Some(app.selected.iter().cloned().collect::<Vec<_>>());
                app.settings.big_file_min_mb = app.big_min_mb;
                app.settings.big_file_roots = app
                    .big_roots
                    .iter()
                    .map(|p| p.to_string_lossy().to_string())
                    .collect();
                match app.settings.save() {
                    Ok(()) => app.toast("设置已保存", ToastKind::Info),
                    Err(e) => app.toast(format!("保存失败：{e}"), ToastKind::Error),
                }
            }
        });
        ui.add_space(6.0);
        ui.label(
            RichText::new("每次清理都会在日志目录写入一份明细日志，记录删了什么、跳过了什么。")
                .size(11.5)
                .color(p.text_faint),
        );
    });
}

/// 「关于」卡片。
///
/// 这里集中放原先挂在侧栏底部的那几行说明（版本号、运行方式），
/// 侧栏只负责导航，不再堆放文字。
fn about(ui: &mut egui::Ui, p: &Palette) {
    card(ui, p, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                widgets::section_title(ui, p, "关于");
                ui.add_space(2.0);
                ui.label(
                    RichText::new("本地运行 · 不联网 · 不上传任何数据")
                        .size(12.0)
                        .color(p.text_weak),
                );
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                pill(
                    ui,
                    &format!("v{}", env!("CARGO_PKG_VERSION")),
                    PillKind::Neutral,
                    p,
                );
            });
        });

        ui.add_space(10.0);
        widgets::kv_row(ui, p, "运行平台", crate::sysutil::PLATFORM.label());
        ui.add_space(4.0);
        widgets::kv_row(ui, p, "删除去向", crate::recycle::action_label());
        ui.add_space(4.0);
        widgets::kv_row(
            ui,
            p,
            "当前权限",
            if crate::sysutil::is_admin() {
                crate::sysutil::privilege_label()
            } else {
                "普通用户"
            },
        );
        ui.add_space(4.0);
        widgets::kv_row(
            ui,
            p,
            "数据目录",
            &crate::sysutil::app_data_dir().to_string_lossy(),
        );
    });
}

fn reset(app: &mut App, ui: &mut egui::Ui, p: &Palette) {
    card(ui, p, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                widgets::section_title(ui, p, "恢复默认设置");
                ui.add_space(4.0);
                ui.label(
                    RichText::new("把删除方式、年龄阈值、主题、扫描目录等全部还原为初始状态。")
                        .size(12.0)
                        .color(p.text_weak),
                );
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if widgets::ghost_button(ui, p, "恢复默认").clicked() {
                    let theme = app.settings.theme;
                    app.settings = crate::settings::Settings {
                        theme,
                        ..Default::default()
                    };
                    app.big_min_mb = app.settings.big_file_min_mb;
                    app.big_roots = app.settings.big_file_roots();
                    app.selected = app
                        .targets
                        .iter()
                        .filter(|t| t.default_on)
                        .map(|t| t.id.to_string())
                        .collect();
                    app.touch_settings();
                    app.toast("已恢复默认设置", ToastKind::Success);
                }
            });
        });
    });
}
