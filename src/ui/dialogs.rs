//! 模态弹窗：清理确认、结果报告、普通提示。

use eframe::egui::{self, Align, Color32, CornerRadius, Layout, Margin, RichText, Shadow, Stroke};

use crate::app::{App, CleanConfirm, Dialog, ToastKind};
use crate::cleaner::CleanMode;
use crate::model::human_bytes;
use crate::theme::{Palette, PillKind};
use crate::ui::widgets::{self, pill};

fn modal_frame(p: &Palette) -> egui::Frame {
    egui::Frame::new()
        .fill(p.surface)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(crate::theme::CARD_RADIUS))
        .inner_margin(Margin::same(22))
        .shadow(Shadow {
            offset: [0, 12],
            blur: 34,
            spread: 0,
            color: Color32::from_black_alpha(if p.is_dark() { 140 } else { 60 }),
        })
}

pub fn show(app: &mut App, ctx: &egui::Context) {
    let Some(dialog) = app.dialog.take() else {
        return;
    };
    let p = Palette::of_ctx(ctx);
    let escaped = ctx.input(|i| i.key_pressed(egui::Key::Escape));

    match dialog {
        Dialog::ConfirmUninstall(app_item) => {
            let mut outcome = if escaped {
                Outcome::Cancel
            } else {
                Outcome::KeepOpen
            };

            let size = ModalSize::new(ctx, 580.0);
            egui::Modal::new(egui::Id::new("cleanc_uninstall"))
                .frame(modal_frame(&p))
                .show(ctx, |ui| {
                    confirm_uninstall_body(ui, &p, &app_item, &mut outcome, &size);
                });

            match outcome {
                Outcome::Confirm => app.run_uninstall(*app_item),
                Outcome::Cancel => {}
                Outcome::KeepOpen => {
                    app.dialog = Some(Dialog::ConfirmUninstall(app_item));
                }
            }
        }

        Dialog::ConfirmRecycleFolder(folder) => {
            let mut outcome = if escaped {
                Outcome::Cancel
            } else {
                Outcome::KeepOpen
            };

            let size = ModalSize::new(ctx, 600.0);
            egui::Modal::new(egui::Id::new("cleanc_recycle_folder"))
                .frame(modal_frame(&p))
                .show(ctx, |ui| {
                    confirm_recycle_folder_body(ui, &p, &folder, &mut outcome, &size);
                });

            match outcome {
                Outcome::Confirm => app.recycle_big_folder(folder.path.clone()),
                Outcome::Cancel => {}
                Outcome::KeepOpen => {
                    app.dialog = Some(Dialog::ConfirmRecycleFolder(folder));
                }
            }
        }

        Dialog::ConfirmClean(mut confirm) => {
            let mut outcome = if escaped {
                Outcome::Cancel
            } else {
                Outcome::KeepOpen
            };

            let size = ModalSize::new(ctx, 560.0);
            egui::Modal::new(egui::Id::new("cleanc_confirm"))
                .frame(modal_frame(&p))
                .show(ctx, |ui| {
                    confirm_clean_body(ui, &p, &mut confirm, &mut outcome, &size);
                });

            match outcome {
                Outcome::Confirm => {
                    let CleanConfirm { targets, opts, .. } = *confirm;
                    app.start_clean(targets, opts);
                }
                Outcome::Cancel => {}
                Outcome::KeepOpen => {
                    app.dialog = Some(Dialog::ConfirmClean(confirm));
                }
            }
        }

        Dialog::Report(report) => {
            let mut close = false;
            let mut open_log: Option<std::path::PathBuf> = None;

            let size = ModalSize::new(ctx, 560.0);
            egui::Modal::new(egui::Id::new("cleanc_report"))
                .frame(modal_frame(&p))
                .show(ctx, |ui| {
                    report_body(ui, &p, &report, &mut close, &mut open_log, &size);
                });

            if let Some(path) = open_log {
                crate::sysutil::reveal_in_file_manager(&path);
            }
            if !close {
                app.dialog = Some(Dialog::Report(report));
            }
        }

        Dialog::Message { title, body, kind } => {
            let mut close = false;
            let accent = match kind {
                ToastKind::Info => p.accent,
                ToastKind::Success => p.safe,
                ToastKind::Warn => p.caution,
                ToastKind::Error => p.danger,
            };
            let size = ModalSize::new(ctx, 460.0);
            egui::Modal::new(egui::Id::new("cleanc_message"))
                .frame(modal_frame(&p))
                .show(ctx, |ui| {
                    message_body(ui, &p, &title, &body, accent, &mut close, &size);
                });
            if !close && !escaped {
                app.dialog = Some(Dialog::Message { title, body, kind });
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 弹窗尺寸：让标题和按钮在任何窗口尺寸下都留在屏幕里
// ---------------------------------------------------------------------------

/// 标题区（标题行 + 一行说明）大致占的高度，含与内容区的间距。
const MODAL_HEADER_H: f32 = 104.0;
/// 底部按钮区大致占的高度，含与内容区的间距。
const MODAL_FOOTER_H: f32 = 64.0;

/// 弹窗的尺寸预算：宽度固定，高度跟着窗口走。
struct ModalSize {
    width: f32,
    max_height: f32,
}

impl ModalSize {
    /// 上下各留 28px 余量，避免弹窗贴到屏幕边缘；再小也给 300px，不然内容没法看。
    fn new(ctx: &egui::Context, width: f32) -> Self {
        // 用 viewport_rect 而不是 content_rect：弹窗是浮层，能用的就是整个窗口
        let max_height = (ctx.viewport_rect().height() - 56.0).max(300.0);
        Self { width, max_height }
    }

    /// 滚动区能用的高度：总高减掉标题与按钮两段。
    fn content_height(&self) -> f32 {
        (self.max_height - MODAL_HEADER_H - MODAL_FOOTER_H).max(120.0)
    }
}

/// 三段式弹窗的开头：先把整体尺寸限制住。
///
/// 之前弹窗是一整块直接铺出来，小窗口下总高度超过屏幕，`egui::Modal`
/// 会把超出的部分裁掉，标题和「完成 / 打开日志」按钮就一起消失了。
/// 现在每个弹窗都按「标题固定 → 中间滚动 → 按钮固定」排布。
fn modal_begin(ui: &mut egui::Ui, size: &ModalSize) {
    ui.set_max_width(size.width);
    ui.set_max_height(size.max_height);
}

/// 三段式弹窗的中间那段：高度受限的滚动区。
fn modal_scroll(
    ui: &mut egui::Ui,
    size: &ModalSize,
    scroll_id: &str,
    content: impl FnOnce(&mut egui::Ui),
) {
    egui::ScrollArea::vertical()
        .id_salt(scroll_id)
        .max_height(size.content_height())
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            content(ui);
        });
}

// ---------------------------------------------------------------------------
// 各弹窗的三段式排布
// ---------------------------------------------------------------------------

fn message_body(
    ui: &mut egui::Ui,
    p: &Palette,
    title: &str,
    body: &str,
    accent: Color32,
    close: &mut bool,
    size: &ModalSize,
) {
    modal_begin(ui, size);
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
        ui.painter().circle_filled(rect.center(), 5.0, accent);
        ui.add_space(4.0);
        ui.label(RichText::new(title).size(16.0).strong().color(p.text));
    });
    ui.add_space(14.0);
    modal_scroll(ui, size, "cleanc_message_scroll", |ui| {
        ui.label(RichText::new(body).size(12.5).color(p.text_weak));
    });
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if widgets::primary_button(ui, p, "知道了", true).clicked() {
                *close = true;
            }
        });
    });
}

fn confirm_uninstall_body(
    ui: &mut egui::Ui,
    p: &Palette,
    app: &crate::apps::InstalledApp,
    outcome: &mut Outcome,
    size: &ModalSize,
) {
    modal_begin(ui, size);
    confirm_uninstall_header(ui, p, app);
    ui.add_space(14.0);
    modal_scroll(ui, size, "cleanc_uninstall_scroll", |ui| {
        confirm_uninstall_content(ui, p, app);
    });
    ui.add_space(12.0);
    confirm_uninstall_footer(ui, p, outcome);
}

fn confirm_recycle_folder_body(
    ui: &mut egui::Ui,
    p: &Palette,
    folder: &crate::model::BigFolder,
    outcome: &mut Outcome,
    size: &ModalSize,
) {
    modal_begin(ui, size);
    confirm_recycle_folder_header(ui, p, folder);
    ui.add_space(14.0);
    modal_scroll(ui, size, "cleanc_recycle_folder_scroll", |ui| {
        confirm_recycle_folder_content(ui, p, folder);
    });
    ui.add_space(12.0);
    confirm_recycle_folder_footer(ui, p, outcome);
}

fn confirm_clean_body(
    ui: &mut egui::Ui,
    p: &Palette,
    confirm: &mut CleanConfirm,
    outcome: &mut Outcome,
    size: &ModalSize,
) {
    modal_begin(ui, size);
    confirm_clean_header(ui, p, confirm);
    ui.add_space(14.0);
    modal_scroll(ui, size, "cleanc_confirm_scroll", |ui| {
        confirm_clean_content(ui, p, confirm);
    });
    ui.add_space(12.0);
    confirm_clean_footer(ui, p, confirm, outcome);
}

fn report_body(
    ui: &mut egui::Ui,
    p: &Palette,
    report: &crate::model::CleanReport,
    close: &mut bool,
    open_log: &mut Option<std::path::PathBuf>,
    size: &ModalSize,
) {
    modal_begin(ui, size);
    report_header(ui, p, report);
    ui.add_space(14.0);
    modal_scroll(ui, size, "cleanc_report_scroll", |ui| {
        report_content(ui, p, report);
    });
    ui.add_space(12.0);
    report_footer(ui, p, report, close, open_log);
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Outcome {
    KeepOpen,
    Confirm,
    Cancel,
}

// ---------------------------------------------------------------------------
// 确认清理
// ---------------------------------------------------------------------------

fn confirm_clean_header(ui: &mut egui::Ui, p: &Palette, confirm: &CleanConfirm) {
    ui.horizontal(|ui| {
        ui.label(RichText::new("确认清理").size(18.0).strong().color(p.text));
        ui.add_space(6.0);
        if confirm.opts.dry_run {
            pill(ui, "演练模式 · 不会删除", PillKind::Caution, p);
        } else if confirm.opts.mode == CleanMode::Permanent {
            pill(ui, "永久删除", PillKind::Danger, p);
        } else {
            pill(ui, "移入回收站", PillKind::Safe, p);
        }
    });
    ui.add_space(4.0);
    ui.label(
        RichText::new("请再确认一次下面的内容。清理过程中被占用的文件会被自动跳过。")
            .size(12.0)
            .color(p.text_weak),
    );
}

fn confirm_clean_content(ui: &mut egui::Ui, p: &Palette, confirm: &mut CleanConfirm) {
    // 概要
    widgets::section(ui, p, |ui| {
        ui.set_width(ui.available_width());
        ui.columns(3, |cols| {
            summary_cell(
                &mut cols[0],
                p,
                "清理类别",
                &format!("{} 项", confirm.targets.len()),
            );
            summary_cell(
                &mut cols[1],
                p,
                "文件数量",
                &format!("{} 个", crate::model::thousands(confirm.files)),
            );
            summary_cell(&mut cols[2], p, "预计释放", &human_bytes(confirm.bytes));
        });
    });

    ui.add_space(14.0);
    ui.label(RichText::new("删除方式").size(12.5).color(p.text_weak));
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        for mode in [CleanMode::RecycleBin, CleanMode::Permanent] {
            let selected = confirm.opts.mode == mode;
            let accent = if mode == CleanMode::Permanent {
                p.danger
            } else {
                p.accent
            };
            let btn =
                egui::Button::new(RichText::new(mode.label()).size(12.5).color(if selected {
                    Color32::WHITE
                } else {
                    p.text
                }))
                .fill(if selected { accent } else { p.surface })
                .stroke(Stroke::new(1.0, if selected { accent } else { p.border }))
                .corner_radius(CornerRadius::same(crate::theme::CARD_RADIUS));
            if ui.add(btn).on_hover_text(mode.hint()).clicked() {
                confirm.opts.mode = mode;
            }
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let mut dry = confirm.opts.dry_run;
            if ui
                .checkbox(&mut dry, RichText::new("演练模式").size(12.0))
                .changed()
            {
                confirm.opts.dry_run = dry;
            }
        });
    });

    ui.add_space(12.0);

    // 风险提示
    if confirm.opts.dry_run {
        notice(
            ui,
            p,
            p.accent,
            "演练模式已开启：本次只统计体积，不会真正删除任何文件。",
        );
    } else if confirm.opts.mode == CleanMode::Permanent {
        notice(
            ui,
            p,
            p.danger,
            "永久删除无法恢复。这些缓存文件之后会被系统或程序重新生成，但已有的数据不会进入回收站。",
        );
    } else if !crate::recycle::is_available() {
        notice(
            ui,
            p,
            p.caution,
            "当前系统的回收站不可用，「移入回收站」会大量跳过文件。建议切换为「永久删除」。",
        );
    } else if confirm.files > 5000 {
        notice(
            ui,
            p,
            p.caution,
            "文件数量较多，移入回收站会分批调用系统接口，可能需要十几秒到一分钟。",
        );
    } else {
        notice(ui, p, p.safe, "文件会移入回收站，误删可以直接还原。");
    }

    // 类别明细（外层弹窗已经有滚动区，这里不再套一层）
    ui.add_space(12.0);
    ui.label(
        RichText::new(format!("将处理以下 {} 个类别", confirm.targets.len()))
            .size(12.0)
            .color(p.text_weak),
    );
    ui.add_space(6.0);

    for target in &confirm.targets {
        ui.horizontal(|ui| {
            ui.label(RichText::new(target.name).size(12.5).color(p.text));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(
                    RichText::new(target.risk.label())
                        .size(11.0)
                        .color(match target.risk {
                            crate::model::RiskLevel::Safe => p.safe,
                            crate::model::RiskLevel::Caution => p.caution,
                            crate::model::RiskLevel::Advanced => p.danger,
                        }),
                );
            });
        });
    }
}

fn confirm_clean_footer(
    ui: &mut egui::Ui,
    p: &Palette,
    confirm: &CleanConfirm,
    outcome: &mut Outcome,
) {
    ui.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let confirm_label = if confirm.opts.dry_run {
                "开始演练"
            } else if confirm.opts.mode == CleanMode::Permanent {
                "确认永久删除"
            } else {
                "确认清理"
            };
            let enabled = confirm.files > 0;
            let clicked = if confirm.opts.mode == CleanMode::Permanent && !confirm.opts.dry_run {
                widgets::danger_button(ui, p, confirm_label, enabled).clicked()
            } else {
                widgets::primary_button(ui, p, confirm_label, enabled).clicked()
            };
            if clicked {
                *outcome = Outcome::Confirm;
            }
            ui.add_space(6.0);
            if widgets::ghost_button(ui, p, "取消").clicked() {
                *outcome = Outcome::Cancel;
            }
        });
    });
}

// ---------------------------------------------------------------------------
// 卸载确认
// ---------------------------------------------------------------------------

fn confirm_uninstall_header(ui: &mut egui::Ui, p: &Palette, app: &crate::apps::InstalledApp) {
    ui.horizontal(|ui| {
        ui.label(RichText::new("卸载软件").size(18.0).strong().color(p.text));
        ui.add_space(6.0);
        pill(ui, &app.uninstall.short_label(), PillKind::Caution, p);
    });
    ui.add_space(4.0);
    ui.label(
        RichText::new(app.uninstall.description())
            .size(12.0)
            .color(p.text_weak),
    );
}

fn confirm_uninstall_content(ui: &mut egui::Ui, p: &Palette, app: &crate::apps::InstalledApp) {
    // 概要
    widgets::section(ui, p, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(RichText::new(&app.name).size(15.0).strong().color(p.text));
            if !app.version.is_empty() {
                pill(ui, &app.version, PillKind::Neutral, p);
            }
        });
        ui.add_space(8.0);
        widgets::kv_row(
            ui,
            p,
            "发布者",
            if app.publisher.is_empty() {
                "—"
            } else {
                &app.publisher
            },
        );
        let size = match app.size_bytes() {
            Some(bytes) => {
                if app.size_is_estimated() {
                    format!("≈ {}", human_bytes(bytes))
                } else {
                    human_bytes(bytes)
                }
            }
            None => "无法单独统计".to_string(),
        };
        widgets::kv_row(ui, p, "占用空间", &size);
        widgets::kv_row(
            ui,
            p,
            "安装位置",
            &app.install_location
                .as_ref()
                .map(|path| path.to_string_lossy().to_string())
                .unwrap_or_else(|| "（未登记）".to_string()),
        );
        if let Some(date) = app.install_date_label() {
            widgets::kv_row(ui, p, "安装时间", &date);
        }
    });

    ui.add_space(12.0);
    ui.label(
        RichText::new("将要执行的动作")
            .size(12.0)
            .color(p.text_weak),
    );
    ui.add_space(6.0);
    let command = app.uninstall_display();
    egui::Frame::new()
        .fill(p.inset)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(crate::theme::CARD_RADIUS))
        .inner_margin(Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                RichText::new(command)
                    .size(11.5)
                    .monospace()
                    .color(p.text_weak),
            );
        });

    ui.add_space(12.0);
    if app.needs_elevation() {
        notice(ui, p, p.caution, &elevation_notice(app));
    } else {
        notice(
            ui,
            p,
            p.danger,
            "卸载会移除这款软件及其相关文件。建议先关闭该软件，并确认重要资料已经备份。",
        );
    }
}

/// 提权提示按平台和动作分叉：Windows 走 UAC，Linux 走 pkexec，
/// macOS 则是"装在系统目录里的 App 移入废纸篓要授权"。
fn elevation_notice(app: &crate::apps::InstalledApp) -> String {
    if app.uninstall.always_needs_root() {
        return "卸载软件包需要超级用户权限，稍后会弹出授权窗口，请输入密码确认。".to_string();
    }
    #[cfg(windows)]
    {
        let _ = app;
        "这条记录来自全机器范围（HKLM），需要管理员权限。稍后会弹出 UAC 提示，请选择「是」。"
            .to_string()
    }
    #[cfg(target_os = "macos")]
    {
        let _ = app;
        "这款软件装在系统范围的目录里，移入废纸篓时可能需要输入管理员密码。".to_string()
    }
    #[cfg(target_os = "linux")]
    {
        let _ = app;
        "这款软件装在系统范围的目录里，卸载时需要超级用户权限。".to_string()
    }
    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
    {
        let _ = app;
        "这款软件需要超级用户权限才能卸载。".to_string()
    }
}

fn confirm_uninstall_footer(ui: &mut egui::Ui, p: &Palette, outcome: &mut Outcome) {
    ui.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if widgets::danger_button(ui, p, "开始卸载", true).clicked() {
                *outcome = Outcome::Confirm;
            }
            ui.add_space(6.0);
            if widgets::ghost_button(ui, p, "取消").clicked() {
                *outcome = Outcome::Cancel;
            }
        });
    });
}

// ---------------------------------------------------------------------------
// 删除整个文件夹的确认
// ---------------------------------------------------------------------------

fn confirm_recycle_folder_header(ui: &mut egui::Ui, p: &Palette, folder: &crate::model::BigFolder) {
    let kind = crate::advice::for_dir(&folder.path);

    ui.horizontal(|ui| {
        ui.label(
            RichText::new("删除整个文件夹")
                .size(18.0)
                .strong()
                .color(p.text),
        );
        ui.add_space(6.0);
        pill(ui, "移入回收站", PillKind::Safe, p);
        pill(ui, kind.short(), widgets::advice_pill(kind), p);
    });
    ui.add_space(4.0);
    ui.label(
        RichText::new(
            "这会连同里面所有文件一起移入回收站，比删单个文件重得多，请先确认路径没有问题。",
        )
        .size(12.0)
        .color(p.text_weak),
    );
}

fn confirm_recycle_folder_content(
    ui: &mut egui::Ui,
    p: &Palette,
    folder: &crate::model::BigFolder,
) {
    let kind = crate::advice::for_dir(&folder.path);
    let name = folder
        .path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| folder.path.to_string_lossy().to_string());

    widgets::section(ui, p, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new(&name).size(15.0).strong().color(p.text));
        ui.add_space(8.0);
        widgets::kv_row(ui, p, "完整路径", &folder.path.to_string_lossy());
        widgets::kv_row(ui, p, "占用空间", &human_bytes(folder.size));
        widgets::kv_row(
            ui,
            p,
            "文件数量",
            &format!("{} 个", crate::model::thousands(folder.files)),
        );
        widgets::kv_row(
            ui,
            p,
            "最近修改",
            &crate::cleaner::format_time(folder.modified),
        );
    });

    ui.add_space(12.0);
    ui.label(RichText::new("处置建议").size(12.0).color(p.text_weak));
    ui.add_space(6.0);
    notice(
        ui,
        p,
        widgets::advice_color(kind, p),
        &format!("{} · {}", kind.label(), kind.advice()),
    );

    ui.add_space(10.0);
    notice(
        ui,
        p,
        p.caution,
        "注意：内容进入回收站后依然占用磁盘空间，需要清空回收站才能真正腾出容量。",
    );
}

fn confirm_recycle_folder_footer(ui: &mut egui::Ui, p: &Palette, outcome: &mut Outcome) {
    ui.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if widgets::danger_button(ui, p, "确认移入回收站", true).clicked() {
                *outcome = Outcome::Confirm;
            }
            ui.add_space(6.0);
            if widgets::ghost_button(ui, p, "取消").clicked() {
                *outcome = Outcome::Cancel;
            }
        });
    });
}

fn summary_cell(ui: &mut egui::Ui, p: &Palette, label: &str, value: &str) {
    ui.vertical(|ui| {
        ui.label(RichText::new(label).size(11.5).color(p.text_weak));
        ui.add_space(2.0);
        ui.label(RichText::new(value).size(16.0).strong().color(p.text));
    });
}

fn notice(ui: &mut egui::Ui, p: &Palette, color: Color32, text: &str) {
    let alpha = if p.is_dark() { 52 } else { 26 };
    let fill = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha);
    egui::Frame::new()
        .fill(fill)
        .stroke(Stroke::new(
            1.0,
            Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 110),
        ))
        .corner_radius(CornerRadius::same(crate::theme::CARD_RADIUS))
        .inner_margin(Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(text).size(12.0).color(color));
        });
}

// ---------------------------------------------------------------------------
// 结果报告
// ---------------------------------------------------------------------------

fn report_header(ui: &mut egui::Ui, p: &Palette, report: &crate::model::CleanReport) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(if report.dry_run {
                "演练结果"
            } else {
                "清理完成"
            })
            .size(18.0)
            .strong()
            .color(p.text),
        );
        ui.add_space(6.0);
        if report.canceled {
            pill(ui, "被中断", PillKind::Caution, p);
        } else if report.blocked > 0 {
            pill(ui, "有拦截记录", PillKind::Caution, p);
        } else {
            pill(ui, "一切正常", PillKind::Safe, p);
        }
    });
}

fn report_content(ui: &mut egui::Ui, p: &Palette, report: &crate::model::CleanReport) {
    widgets::section(ui, p, |ui| {
        ui.set_width(ui.available_width());
        ui.columns(2, |cols| {
            summary_cell(
                &mut cols[0],
                p,
                if report.dry_run {
                    "可释放空间"
                } else {
                    "已释放空间"
                },
                &human_bytes(report.freed_bytes),
            );
            summary_cell(
                &mut cols[1],
                p,
                "处理文件数",
                &crate::model::thousands(report.deleted_files),
            );
        });
        ui.add_space(10.0);
        ui.columns(3, |cols| {
            summary_cell(
                &mut cols[0],
                p,
                "删除空目录",
                &crate::model::thousands(report.deleted_dirs),
            );
            summary_cell(
                &mut cols[1],
                p,
                "跳过（占用/权限）",
                &crate::model::thousands(report.skipped),
            );
            summary_cell(
                &mut cols[2],
                p,
                "安全拦截",
                &crate::model::thousands(report.blocked),
            );
        });
    });

    ui.add_space(12.0);

    for entry in &report.entries {
        ui.horizontal(|ui| {
            ui.label(RichText::new(&entry.name).size(12.5).color(p.text))
                .on_hover_text(entry.id);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(
                    RichText::new(format!("{} 个 / {}", entry.files, human_bytes(entry.bytes)))
                        .size(11.5)
                        .color(p.text_weak),
                );
            });
        });
        if entry.skipped > 0 || entry.blocked > 0 {
            ui.horizontal(|ui| {
                ui.add_space(12.0);
                ui.label(
                    RichText::new(format!("跳过 {} · 拦截 {}", entry.skipped, entry.blocked))
                        .size(10.5)
                        .color(p.text_faint),
                );
            });
        }
    }

    if !report.failures.is_empty() {
        ui.add_space(10.0);
        ui.label(
            RichText::new("部分被拦截的条目（正常情况下不应出现）：")
                .size(11.5)
                .color(p.caution),
        );
        for line in report.failures.iter().take(20) {
            ui.label(
                RichText::new(line)
                    .size(10.5)
                    .color(p.text_faint)
                    .monospace(),
            );
        }
    }

    if let Some(log) = &report.log_path {
        ui.add_space(10.0);
        ui.label(
            RichText::new(format!("明细日志：{}", log.display()))
                .size(10.5)
                .color(p.text_faint),
        );
    }
}

fn report_footer(
    ui: &mut egui::Ui,
    p: &Palette,
    report: &crate::model::CleanReport,
    close: &mut bool,
    open_log: &mut Option<std::path::PathBuf>,
) {
    ui.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if widgets::primary_button(ui, p, "完成", true).clicked() {
                *close = true;
            }
            ui.add_space(6.0);
            if report.log_path.is_some() && widgets::ghost_button(ui, p, "打开日志").clicked() {
                *open_log = report.log_path.clone();
                *close = true;
            }
        });
    });
}
