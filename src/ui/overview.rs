//! 概览页：一句话给出结论，一个大按钮动手，一排入口分头去处理。
//!
//! 版式参考「结论先行」的清理工具：先把「一共能腾出多少空间」说清楚，
//! 再给一个最显眼的动作，最后才是细节（磁盘容量、上次战果、安全承诺）。

use eframe::egui::{self, Align, Color32, Layout, Rect, RichText, Sense, vec2};

use crate::app::{App, Page};
use crate::cleaner::CleanMode;
use crate::model::{human_bytes, thousands};
use crate::sysutil;
use crate::theme::{Palette, PillKind};
use crate::ui::icons;
use crate::ui::widgets::{self, bullet, card, section_title, usage_bar};

/// Hero 区的磁盘插画（`assets/disk1.png`），`include_bytes!` 编进可执行文件。
const DISK_ICON_PNG: &[u8] = include_bytes!("../../assets/disk1.png");

/// 插画边长 —— 它是整个页面最重的一块图形。
///
/// 参考图里插画几乎撑满右半区，是 Hero 的视觉重心，所以这里也取得比较大。
/// 再大就会明显放大原图（`assets/disk1.png` 是 256×256），开始发虚了。
const HERO_ART_SIZE: f32 = 220.0;

/// 取 Hero 插画的贴图：首次调用时解码上传，之后复用 `App` 上的缓存。
///
/// 和顶栏品牌图标一样把缓存挂在 `App` 上而不是全局变量 —— wgpu 后端初始化
/// 失败时会换 Glow 再开一次，全局缓存那样会留下失效的句柄。
pub(crate) fn disk_texture(app: &mut App, ctx: &egui::Context) -> Option<egui::TextureHandle> {
    if app.disk_tex.is_none() {
        match crate::ui::chrome::decode_icon_png(DISK_ICON_PNG) {
            Ok(image) => {
                app.disk_tex =
                    Some(ctx.load_texture("cleanc_disk_icon", image, egui::TextureOptions::LINEAR));
            }
            Err(err) => {
                eprintln!("assets/disk1.png 解码失败（{err}），Hero 区改用内置绘制的图形");
            }
        }
    }
    app.disk_tex.clone()
}

/// `assets/disk1.png` 解码失败时的兜底图形（正常走不到这里）。
fn paint_disk_fallback(ui: &mut egui::Ui, p: &Palette, rect: Rect) {
    let body = Rect::from_center_size(rect.center(), rect.size() * 0.72);
    let painter = ui.painter();
    painter.rect_filled(body, egui::CornerRadius::same(16), p.inset);
    painter.rect_filled(
        Rect::from_min_size(body.min, vec2(body.width(), body.height() * 0.34)),
        egui::CornerRadius::same(16),
        p.accent,
    );
}

/// 按占用率给一句人话建议。
fn disk_usage_hint(ratio: f32) -> &'static str {
    if ratio >= 0.9 {
        "空间非常紧张，建议尽快清理"
    } else if ratio >= 0.75 {
        "空间偏紧，清理一次会有明显改善"
    } else {
        "空间充足，可定期清理保持整洁"
    }
}

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let p = Palette::of(ui);

    hero(app, ui, &p);
    ui.add_space(20.0);

    quick_entries(app, ui, &p);
    ui.add_space(20.0);

    disk_card(app, ui, &p);
    ui.add_space(12.0);

    // -----------------------------------------------------------------
    // 最近一次清理
    // -----------------------------------------------------------------
    if let Some(report) = &app.last_report {
        card(ui, &p, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                section_title(ui, &p, "最近一次清理");
                ui.add_space(6.0);
                widgets::pill(
                    ui,
                    if report.dry_run {
                        "演练（未删除）"
                    } else {
                        "已执行"
                    },
                    if report.dry_run {
                        PillKind::Neutral
                    } else {
                        PillKind::Safe
                    },
                    &p,
                );
            });
            ui.add_space(8.0);
            ui.columns(4, |cols| {
                mini_stat(
                    &mut cols[0],
                    &p,
                    "释放空间",
                    &human_bytes(report.freed_bytes),
                );
                mini_stat(
                    &mut cols[1],
                    &p,
                    "处理文件",
                    &thousands(report.deleted_files),
                );
                mini_stat(&mut cols[2], &p, "跳过", &thousands(report.skipped));
                mini_stat(&mut cols[3], &p, "安全拦截", &thousands(report.blocked));
            });
            if let Some(log) = &report.log_path {
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("日志：{}", log.display()))
                            .size(11.0)
                            .color(p.text_faint),
                    );
                    if widgets::ghost_button(ui, &p, "打开").clicked() {
                        crate::sysutil::reveal_in_file_manager(log);
                    }
                });
            }
        });
        ui.add_space(12.0);
    }

    // -----------------------------------------------------------------
    // 安全承诺
    // -----------------------------------------------------------------
    card(ui, &p, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            section_title(ui, &p, "这个工具不会碰什么");
            ui.add_space(6.0);
            widgets::pill(ui, "白名单机制", PillKind::Accent, &p);
        });
        ui.add_space(8.0);
        bullet(
            ui,
            &p,
            p.safe,
            "文档、桌面、下载、图片、视频、音乐目录一律不扫描、不清理。",
        );
        bullet(
            ui,
            &p,
            p.safe,
            &format!(
                "系统关键目录（{} 等）永远在保护名单内。",
                crate::safety::protected_examples()
            ),
        );
        bullet(
            ui,
            &p,
            p.safe,
            "符号链接与目录联接不会被跟随，避免误删链接指向的真实数据。",
        );
        bullet(
            ui,
            &p,
            p.safe,
            "临时文件默认只清理 7 天前的，正在被程序占用的文件会自动跳过。",
        );
        bullet(
            ui,
            &p,
            p.caution,
            &format!(
                "勾选「{}」或把删除方式切成「永久删除」会不可恢复，请留意确认弹窗。",
                crate::recycle::trash_name()
            ),
        );
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if widgets::ghost_button(ui, &p, "查看完整安全说明").clicked() {
                app.page = Page::Safety;
            }
        });
    });
}

// ---------------------------------------------------------------------------
// Hero：结论 + 主按钮 + 插画
// ---------------------------------------------------------------------------

fn hero(app: &mut App, ui: &mut egui::Ui, p: &Palette) {
    let art = disk_texture(app, ui.ctx());
    let summary = app.summary().cloned();
    let (chosen, _, chosen_bytes) = app.pending_selection();
    let scanned = summary.is_some();
    let total = summary.as_ref().map(|s| s.total_bytes()).unwrap_or(0);
    let total_files = summary.as_ref().map(|s| s.total_files()).unwrap_or(0);

    ui.horizontal(|ui| {
        // ---------------- 左：结论 + 主按钮 ----------------
        ui.vertical(|ui| {
            ui.add_space(6.0);

            // 大标题：只有那个数字是绿的，一眼就看得到
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                if scanned {
                    ui.label(RichText::new("共发现").size(30.0).strong().color(p.text));
                    ui.label(
                        RichText::new(human_bytes(total))
                            .size(35.0)
                            .strong()
                            .color(p.accent_ink),
                    );
                    ui.label(
                        RichText::new("可清理文件")
                            .size(30.0)
                            .strong()
                            .color(p.text),
                    );
                } else {
                    ui.label(
                        RichText::new("先扫描一次，看看能腾出多少空间")
                            .size(30.0)
                            .strong()
                            .color(p.text),
                    );
                }
            });

            ui.add_space(8.0);

            // 副行：待清理体积 + 查看 + 当前勾选
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                if scanned {
                    if chosen_bytes > 0 {
                        ui.label(
                            RichText::new(human_bytes(chosen_bytes))
                                .size(14.0)
                                .color(p.caution),
                        );
                        ui.label(RichText::new("可一键清理").size(13.0).color(p.text_weak));
                    } else {
                        ui.label(
                            RichText::new("还没有勾选可清理项")
                                .size(13.0)
                                .color(p.text_weak),
                        );
                    }
                    ui.label(RichText::new("｜").size(13.0).color(p.text_faint));
                    if ui.link(RichText::new("查看").size(13.0)).clicked() {
                        app.page = Page::Clean;
                    }
                    ui.add_space(2.0);
                    ui.label(
                        RichText::new(format!(
                            "共 {} 项 / {} 个文件",
                            thousands(chosen.len() as u64),
                            thousands(total_files)
                        ))
                        .size(11.5)
                        .color(p.text_faint),
                    );
                } else {
                    ui.label(
                        RichText::new("扫描只读文件信息，不会删除任何东西")
                            .size(13.0)
                            .color(p.text_weak),
                    );
                }
            });

            ui.add_space(22.0);

            // 主按钮：没扫过就先扫，扫过了就直接清。
            // 带一个垃圾桶图标，和参考图那颗「一键清理」一致。
            let (label, icon, enabled) = if !scanned {
                ("立即扫描", icons::Icon::Scan, !app.busy())
            } else if chosen_bytes > 0 {
                ("一键清理", icons::Icon::Clean, !app.busy())
            } else {
                ("重新扫描", icons::Icon::Scan, !app.busy())
            };

            if widgets::hero_button(ui, &p, Some(icon), label, enabled).clicked() {
                if scanned && chosen_bytes > 0 {
                    app.request_clean();
                } else {
                    if app.page != Page::Clean {
                        app.page = Page::Clean;
                    }
                    app.start_scan();
                }
            }

            ui.add_space(10.0);

            // 删除去向：这是安全性上最要紧的一条信息，放在按钮下面顺手能看见
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.label(
                    RichText::new(format!("删除去向：{}", app.settings.clean_mode.label()))
                        .size(11.5)
                        .color(p.text_faint),
                );
                if app.settings.clean_mode == CleanMode::RecycleBin
                    && !crate::recycle::is_available()
                {
                    widgets::pill(ui, "回收站不可用", PillKind::Caution, &p);
                }
            });
        });

        // ---------------- 右：磁盘插画 ----------------
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let (rect, _) =
                ui.allocate_exact_size(vec2(HERO_ART_SIZE, HERO_ART_SIZE), Sense::hover());
            match &art {
                Some(tex) => {
                    // 裁剪后的图标接近正方形，等比缩放后居中放进插画位
                    let size = tex.size_vec2();
                    let scale = (rect.width() / size.x).min(rect.height() / size.y);
                    egui::Image::new(tex)
                        .paint_at(ui, Rect::from_center_size(rect.center(), size * scale));
                }
                None => paint_disk_fallback(ui, &p, rect),
            }
        });
    });
}

// ---------------------------------------------------------------------------
// 快捷入口
// ---------------------------------------------------------------------------

/// 入口图的色调。纯装饰，两套主题共用同几个中间调，所以不放进 `Palette`。
const TINT_BLUE: Color32 = Color32::from_rgb(0x3B, 0x9B, 0xF6);
const TINT_AMBER: Color32 = Color32::from_rgb(0xF5, 0x9E, 0x0B);
const TINT_VIOLET: Color32 = Color32::from_rgb(0x8B, 0x6C, 0xF6);
const TINT_SLATE: Color32 = Color32::from_rgb(0x64, 0x78, 0x8C);
const TINT_CYAN: Color32 = Color32::from_rgb(0x14, 0xB8, 0xD4);

fn quick_entries(app: &mut App, ui: &mut egui::Ui, p: &Palette) {
    let summary = app.summary().cloned();
    let scanned = summary.is_some();
    let total = summary.as_ref().map(|s| s.total_bytes()).unwrap_or(0);
    let clean_sub = if scanned {
        format!("{} 可清理", human_bytes(total))
    } else {
        "缓存与临时文件".to_string()
    };

    // 图标 / 色调 / 标题 / 副文案 / 目标页
    let entries: [(icons::Icon, Color32, &str, String, Page); 6] = [
        (
            icons::Icon::Clean,
            p.accent,
            "智能清理",
            clean_sub,
            Page::Clean,
        ),
        (
            icons::Icon::BigFiles,
            TINT_BLUE,
            "大文件",
            "找出最占地方的文件".into(),
            Page::BigFiles,
        ),
        (
            icons::Icon::Apps,
            TINT_AMBER,
            "软件卸载",
            "统计占用并逐个卸载".into(),
            Page::Apps,
        ),
        (
            icons::Icon::Overview,
            TINT_VIOLET,
            "全部结果",
            "大文件与大文件夹清单".into(),
            Page::BigDetail,
        ),
        (
            icons::Icon::Settings,
            TINT_SLATE,
            "设置",
            "清理策略与外观".into(),
            Page::Settings,
        ),
        (
            icons::Icon::Safety,
            TINT_CYAN,
            "安全说明",
            "保护名单与承诺".into(),
            Page::Safety,
        ),
    ];

    let mut jump: Option<Page> = None;
    ui.columns(4, |cols| {
        for (index, (icon, tint, title, sub, target)) in entries.iter().enumerate() {
            // 4 列排布：第 0..4 落在第一行，4、5 落在第二行
            let col = &mut cols[index % 4];
            if index >= 4 {
                col.add_space(4.0);
            }
            if widgets::quick_entry(col, &p, *icon, *tint, title, sub).clicked() {
                jump = Some(*target);
            }
        }
    });

    if let Some(page) = jump {
        app.page = page;
    }
}

// ---------------------------------------------------------------------------
// 磁盘占用
// ---------------------------------------------------------------------------

fn disk_card(app: &mut App, ui: &mut egui::Ui, p: &Palette) {
    let disk = app.disk.clone();

    card(ui, &p, |ui| {
        ui.set_width(ui.available_width());

        let Some(disk) = &disk else {
            ui.label(
                RichText::new(format!(
                    "{} 容量读取失败",
                    sysutil::mount_label(&sysutil::default_mount())
                ))
                .size(14.0)
                .color(p.danger),
            );
            ui.add_space(4.0);
            ui.label(
                RichText::new("稍后重试，或以更高权限重新打开本程序。")
                    .size(12.0)
                    .color(p.text_weak),
            );
            return;
        };

        let ratio = disk.used_ratio();

        // 第一行：磁盘名 + 已用百分比
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(sysutil::disk_title(&disk.mount))
                    .size(15.0)
                    .color(p.text),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(
                    RichText::new(format!("已用 {:.0}%", ratio * 100.0))
                        .size(14.0)
                        .color(crate::theme::usage_color(&p, ratio)),
                );
            });
        });

        // 第二行：整行铺满的占用条
        ui.add_space(10.0);
        usage_bar(ui, &p, ratio, 14.0);

        // 第三行：容量明细 + 一句建议
        ui.add_space(9.0);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!(
                    "{} 可用，共 {}",
                    human_bytes(disk.available),
                    human_bytes(disk.total)
                ))
                .size(12.5)
                .color(p.text_weak),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(
                    RichText::new(disk_usage_hint(ratio))
                        .size(11.5)
                        .color(p.text_faint),
                );
            });
        });
    });
}

fn mini_stat(ui: &mut egui::Ui, p: &Palette, label: &str, value: &str) {
    ui.vertical(|ui| {
        ui.label(RichText::new(label).size(11.5).color(p.text_weak));
        ui.add_space(2.0);
        ui.label(RichText::new(value).size(16.0).color(p.text));
    });
}
