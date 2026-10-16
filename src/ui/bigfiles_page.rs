//! 大文件页：找出占空间最多的文件与文件夹，给出处置建议，由用户自行决定。

use eframe::egui::{self, Align, CornerRadius, Layout, RichText};

use crate::advice::{self, Kind};
use crate::app::{App, BigState, Page};
use crate::model::{BigFolder, human_bytes, thousands};
use crate::theme::{Palette, PillKind};
use crate::ui::widgets::{self, card, pill, primary_button};

/// 主页面预览的行数：超过这个数就在卡片右上角给一个「查看更多」。
const PREVIEW_ROWS: usize = 5;

/// 「全部结果」页最多渲染的行数（扫描层本身也限制了条数，这里只做兜底）。
const MAX_ROWS: usize = 500;

/// 阈值滑块的范围（MB）。目录体积可以很大，上限给得宽一些。
const MIN_MB: f64 = 10.0;
const MAX_MB: f64 = 10_240.0;

/// 常用阈值档位。
const PRESETS: &[(u64, &str)] = &[
    (100, "100 MB"),
    (500, "500 MB"),
    (1024, "1 GB"),
    (5120, "5 GB"),
];

/// 列表卡的两种形态。
#[derive(Clone, Copy, PartialEq, Eq)]
enum ListMode {
    /// 主页：只列体积最大的几条，右上角带「查看更多」
    Preview,
    /// 「全部结果」页：列全部，右上角换成「返回」
    Full,
}

/// 本帧界面产生的动作，统一在渲染结束后处理（避免借用冲突）。
#[derive(Default)]
struct Actions {
    recycle_file: Option<std::path::PathBuf>,
    recycle_folder: Option<std::path::PathBuf>,
    toggle_files: Option<bool>,
    toggle_folders: Option<bool>,
    hide_nested: Option<bool>,
    /// 跳到「全部结果」页
    open_detail: bool,
    /// 从「全部结果」页返回
    back: bool,
}

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let p = Palette::of(ui);

    let mut actions = Actions::default();

    config_card(app, ui, &p, &mut actions);
    ui.add_space(10.0);

    if app.big_scanning() {
        progress_card(app, ui, &p);
        ui.add_space(10.0);
    }

    results_card(app, ui, &p, &mut actions);

    handle_actions(app, actions);
}

/// 「全部结果」页：大文件与大文件夹的完整列表。
///
/// 主页每张卡只放 5 条，再多就在这里看全 —— 放在同一个页面里滚动，
/// 比在主页里塞一个内嵌滚动条更好找东西。
pub fn show_all(app: &mut App, ui: &mut egui::Ui) {
    let p = Palette::of(ui);
    let mut actions = Actions::default();

    ui.horizontal(|ui| {
        if widgets::ghost_button(ui, &p, "← 返回大文件").clicked() {
            actions.back = true;
        }
    });
    ui.add_space(6.0);

    // 过滤 + 摘要（要先处理，后面拿只读借用渲染列表）
    detail_toolbar(app, ui, &p, &mut actions);

    match &app.big {
        BigState::Done {
            files,
            folders,
            totals,
            scanned,
            elapsed_ms,
            canceled,
            want_files,
            want_folders,
        } => {
            let totals = *totals;
            let (want_files, want_folders) = (*want_files, *want_folders);
            let canceled = *canceled;
            let summary = DetailSummary {
                scanned: *scanned,
                elapsed_ms: *elapsed_ms,
                file_bytes: files.iter().map(|f| f.size).sum(),
                folder_bytes: folders.iter().map(|f| f.size).sum(),
                file_count: files.len(),
                folder_count: folders.len(),
            };
            let query = app.big_detail_query.trim().to_lowercase();
            let hide_nested = app.big_hide_nested;

            ui.add_space(10.0);
            summary_card(ui, &p, &summary, want_files, want_folders, canceled);
            ui.add_space(10.0);
            advice_summary(ui, &p, totals);

            // 大文件夹
            if want_folders && summary.folder_count > 0 {
                let rows: Vec<&BigFolder> = folders
                    .iter()
                    .filter(|f| matches_query(&f.path, &query))
                    .take(MAX_ROWS)
                    .collect();
                ui.add_space(10.0);
                folders_card(
                    ui,
                    &p,
                    &rows,
                    summary.folder_bytes,
                    hide_nested,
                    ListMode::Full,
                    &mut actions,
                );
            }

            // 大文件
            if want_files && summary.file_count > 0 {
                let rows: Vec<FileRow> = files
                    .iter()
                    .filter(|f| matches_query(&f.path, &query))
                    .take(MAX_ROWS)
                    .map(|f| FileRow {
                        path: &f.path,
                        size: f.size,
                        modified: f.modified,
                        protected: f.protected,
                    })
                    .collect();
                ui.add_space(10.0);
                files_card(
                    ui,
                    &p,
                    &rows,
                    summary.file_bytes,
                    ListMode::Full,
                    &mut actions,
                );
            }
        }
        _ => {
            ui.add_space(10.0);
            card(ui, &p, |ui| {
                ui.set_width(ui.available_width());
                widgets::section_title(ui, &p, "还没有扫描结果");
                ui.add_space(6.0);
                ui.label(
                    RichText::new(
                        "回到「大文件」页执行一次扫描，结果会完整地列在这里。扫描只读取文件信息，不会删除任何东西。",
                    )
                    .size(12.5)
                    .color(p.text_weak),
                );
            });
        }
    }

    handle_actions(app, actions);
}

/// 统一处理两个页面收集到的动作。
fn handle_actions(app: &mut App, actions: Actions) {
    if let Some(want) = actions.toggle_files {
        apply_toggle(app, true, want);
    }
    if let Some(want) = actions.toggle_folders {
        apply_toggle(app, false, want);
    }
    if let Some(hide) = actions.hide_nested {
        app.big_hide_nested = hide;
    }
    if let Some(path) = actions.recycle_file {
        app.recycle_big_file(path);
    }
    if let Some(path) = actions.recycle_folder {
        app.request_recycle_folder(&path);
    }
    if actions.open_detail {
        app.page = Page::BigDetail;
    }
    if actions.back {
        app.page = Page::BigFiles;
    }
}

/// 路径是否命中过滤关键词（大小写不敏感）。
fn matches_query(path: &std::path::Path, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    path.to_string_lossy().to_lowercase().contains(query)
}

/// 「文件」「文件夹」至少留一项 —— 两样都不勾选的话扫描没有意义。
fn apply_toggle(app: &mut App, is_files: bool, want: bool) {
    let (files, folders) = (app.big_want_files, app.big_want_folders);
    if !want {
        // 想关掉的是最后一项就不允许
        let other_on = if is_files { folders } else { files };
        if !other_on {
            app.toast(
                "「文件」和「文件夹」至少要保留一项",
                crate::app::ToastKind::Warn,
            );
            return;
        }
    }
    if is_files {
        app.big_want_files = want;
    } else {
        app.big_want_folders = want;
    }
    app.touch_settings();
}

// ---------------------------------------------------------------------------
// 配置
// ---------------------------------------------------------------------------

fn config_card(app: &mut App, ui: &mut egui::Ui, p: &Palette, actions: &mut Actions) {
    let mut remove_root: Option<usize> = None;
    let mut add_root = false;

    card(ui, p, |ui| {
        ui.set_width(ui.available_width());
        widgets::section_title(ui, p, "扫描范围");
        ui.add_space(4.0);
        ui.label(
            RichText::new("只读取文件信息做统计，不会自动删除任何东西。找到结果后由你逐个决定。")
                .size(12.0)
                .color(p.text_weak),
        );
        ui.add_space(10.0);

        // ---- 统计对象 + 体积阈值 ----
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("统计对象").size(12.5).color(p.text_weak));
            ui.add_space(2.0);
            for (label, is_files, value) in [
                ("文件", true, app.big_want_files),
                ("文件夹", false, app.big_want_folders),
            ] {
                let mut checked = value;
                let resp = ui.checkbox(&mut checked, RichText::new(label).size(12.5));
                if resp.changed() {
                    if is_files {
                        actions.toggle_files = Some(checked);
                    } else {
                        actions.toggle_folders = Some(checked);
                    }
                }
                ui.add_space(2.0);
            }

            ui.add_space(14.0);
            ui.label(RichText::new("体积大于").size(12.5).color(p.text_weak));
            let mut mb = app.big_min_mb as f64;
            let slider = egui::Slider::new(&mut mb, MIN_MB..=MAX_MB)
                .logarithmic(true)
                .clamping(egui::SliderClamping::Always)
                .show_value(false);
            // egui 把轨道画成 widgets.inactive.bg_fill —— 主题里是纯白，在白卡片上等于隐形。
            // 这里在局部把它换成边框灰，并打开 trailing_fill，让已选区间带一条品牌绿。
            // 滑块宽度也由 spacing.slider_width 决定（add_sized 管不住它），一并在这里设。
            let resp = ui
                .scope(|ui| {
                    ui.style_mut().spacing.slider_width = 200.0;
                    ui.style_mut().spacing.slider_rail_height = 5.0;
                    ui.style_mut().visuals.slider_trailing_fill = true;
                    ui.style_mut().visuals.widgets.inactive.bg_fill = p.border;
                    ui.add(slider)
                })
                .inner;
            if resp.changed() {
                app.big_min_mb = mb.round() as u64;
                app.touch_settings();
            }
            // 自带的数值框长得像个按钮（DragValue 本质是 Button），和后面的档位按钮混在一起，
            // 所以关掉 show_value，改用一个纯文本数值。
            ui.label(
                RichText::new(format!("≥ {}", app.big_min_mb))
                    .size(12.5)
                    .strong()
                    .color(p.accent),
            );

            ui.add_space(6.0);
            for &(value, label) in PRESETS {
                let active = app.big_min_mb == value;
                if widgets::ghost_button(ui, p, label).clicked() {
                    app.big_min_mb = value;
                    app.touch_settings();
                }
                if active {
                    ui.label(RichText::new("●").size(7.0).color(p.accent));
                }
            }
        });

        ui.add_space(6.0);
        ui.label(
            RichText::new(
                "「文件」统计单个文件的体积；「文件夹」统计整个目录（含所有子目录）的合计体积。",
            )
            .size(11.0)
            .color(p.text_faint),
        );

        ui.add_space(10.0);
        widgets::divider(ui, p);
        ui.add_space(4.0);

        // ---- 目录列表 ----
        widgets::section_title(ui, p, "扫描目录");
        ui.add_space(6.0);

        if app.big_roots.is_empty() {
            ui.label(
                RichText::new("还没有添加目录")
                    .size(12.0)
                    .color(p.text_faint),
            );
        }

        for (index, root) in app.big_roots.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(root.to_string_lossy().to_string())
                        .size(12.0)
                        .color(p.text),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if widgets::ghost_button(ui, p, "移除").clicked() {
                        remove_root = Some(index);
                    }
                    if widgets::ghost_button(ui, p, "打开").clicked() {
                        crate::sysutil::open_path(root);
                    }
                });
            });
        }

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let resp = ui.add(
                egui::TextEdit::singleline(&mut app.big_roots_input)
                    .hint_text("粘贴一个目录路径，例如 C:\\Users\\me\\Projects")
                    .desired_width(320.0),
            );
            let submitted = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if widgets::ghost_button(ui, p, "添加目录").clicked() || submitted {
                add_root = true;
            }
            if widgets::ghost_button(ui, p, "恢复默认").clicked() {
                app.big_roots = crate::settings::Settings::default_big_file_roots();
                app.touch_settings();
            }
        });

        ui.add_space(12.0);
        ui.horizontal(|ui| {
            let scanning = app.big_scanning();
            let label = if scanning {
                "扫描中…"
            } else {
                "开始扫描"
            };
            if primary_button(ui, p, label, !app.busy()).clicked() {
                app.start_big_scan();
            }
            if scanning && widgets::ghost_button(ui, p, "取消").clicked() {
                app.cancel_big_scan();
            }
            ui.add_space(6.0);
            ui.label(
                RichText::new("提示：扫描整个用户目录可能需要几十秒到几分钟")
                    .size(11.5)
                    .color(p.text_faint),
            );
        });
    });

    if let Some(index) = remove_root
        && index < app.big_roots.len()
    {
        app.big_roots.remove(index);
        app.touch_settings();
    }

    if add_root {
        let raw = app.big_roots_input.trim().trim_matches('"').to_string();
        if raw.is_empty() {
            app.toast("请输入目录路径", crate::app::ToastKind::Warn);
        } else {
            let path = std::path::PathBuf::from(&raw);
            if !path.is_dir() {
                app.toast("该路径不是一个存在的目录", crate::app::ToastKind::Error);
            } else if app.big_roots.iter().any(|r| r == &path) {
                app.toast("该目录已经在列表里了", crate::app::ToastKind::Warn);
            } else {
                app.big_roots.push(path);
                app.big_roots_input.clear();
                app.touch_settings();
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 进度
// ---------------------------------------------------------------------------

fn progress_card(app: &App, ui: &mut egui::Ui, p: &Palette) {
    let BigState::Running {
        started,
        scanned,
        current,
        ..
    } = &app.big
    else {
        return;
    };

    card(ui, p, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(RichText::new("正在扫描").size(13.5).strong().color(p.text));
            ui.add_space(6.0);
            ui.label(
                RichText::new(format!("已检查 {} 个文件", thousands(*scanned)))
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
        ui.add_space(6.0);
        // 未知总量，用一条流动的进度条表达"进行中"
        let phase = (started.elapsed().as_secs_f32() * 0.6).fract();
        widgets::progress_bar(ui, p, 0.15 + phase * 0.85, 8.0, p.accent);
        ui.add_space(6.0);
        ui.label(
            RichText::new(shorten(&current, 96))
                .size(11.0)
                .color(p.text_faint),
        );
    });
}

// ---------------------------------------------------------------------------
// 结果
// ---------------------------------------------------------------------------

fn results_card(app: &mut App, ui: &mut egui::Ui, p: &Palette, actions: &mut Actions) {
    let BigState::Done {
        files,
        folders,
        totals,
        scanned,
        elapsed_ms,
        canceled,
        want_files,
        want_folders,
    } = &app.big
    else {
        return;
    };
    let totals = *totals;
    // 用"上次扫描的选项"渲染结果区，而不是当前勾选状态
    let (want_files, want_folders) = (*want_files, *want_folders);

    let file_total: u64 = files.iter().map(|f| f.size).sum();
    let folder_total: u64 = folders.iter().map(|f| f.size).sum();
    let file_count = files.len();
    let folder_count = folders.len();
    let scanned = *scanned;
    let elapsed_ms = *elapsed_ms;
    let canceled = *canceled;

    // 两个列表都直接借用扫描结果（不再每帧克隆路径），行数由卡片自己按形态截取
    let file_rows: Vec<FileRow<'_>> = files
        .iter()
        .map(|f| FileRow {
            path: &f.path,
            size: f.size,
            modified: f.modified,
            protected: f.protected,
        })
        .collect();

    card(ui, p, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            widgets::section_title(ui, p, "扫描结果");
            ui.add_space(6.0);
            if canceled {
                pill(ui, "已取消", PillKind::Caution, p);
            }
        });
        ui.add_space(6.0);

        ui.columns(4, |cols| {
            stat(
                &mut cols[0],
                p,
                "大文件",
                &format!("{} 个", thousands(file_count as u64)),
                if want_files { "" } else { "未统计" },
            );
            stat(
                &mut cols[1],
                p,
                "大文件夹",
                &format!("{} 个", thousands(folder_count as u64)),
                if want_folders { "" } else { "未统计" },
            );
            stat(
                &mut cols[2],
                p,
                "合计体积",
                &human_bytes(file_total + folder_total),
                "",
            );
            stat(
                &mut cols[3],
                p,
                "扫描耗时",
                &format!("{:.1}s", elapsed_ms as f32 / 1000.0),
                &format!("{} 个文件", thousands(scanned)),
            );
        });

        if file_count == 0 && folder_count == 0 {
            ui.add_space(12.0);
            ui.label(
                RichText::new(format!(
                    "没有找到体积超过 {} 的{}。可以把阈值调小一点，或者换一个扫描目录。",
                    human_bytes(app.big_min_mb.max(1) * 1024 * 1024),
                    match (want_files, want_folders) {
                        (true, true) => "文件或文件夹",
                        (true, false) => "文件",
                        (false, true) => "文件夹",
                        (false, false) => "对象",
                    }
                ))
                .size(12.5)
                .color(p.text_weak),
            );
            return;
        }

        ui.add_space(12.0);
        advice_summary(ui, p, totals);
    });

    // ---- 大文件夹 ----
    if want_folders && folder_count > 0 {
        let folder_rows: Vec<&BigFolder> = folders.iter().collect();
        ui.add_space(10.0);
        folders_card(
            ui,
            p,
            &folder_rows,
            folder_total,
            app.big_hide_nested,
            ListMode::Preview,
            actions,
        );
    }

    // ---- 大文件 ----
    if want_files && file_count > 0 {
        ui.add_space(10.0);
        files_card(ui, p, &file_rows, file_total, ListMode::Preview, actions);
    }
}

/// 「全部结果」页顶部的筛选工具条。
fn detail_toolbar(app: &mut App, ui: &mut egui::Ui, p: &Palette, actions: &mut Actions) {
    ui.horizontal(|ui| {
        ui.label(RichText::new("过滤").size(12.5).color(p.text_weak));
        ui.add(
            egui::TextEdit::singleline(&mut app.big_detail_query)
                .hint_text("按路径关键词过滤，例如 .zip、node_modules")
                .desired_width(300.0),
        );
        if !app.big_detail_query.is_empty() && widgets::ghost_button(ui, p, "清空").clicked() {
            app.big_detail_query.clear();
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if widgets::ghost_button(ui, p, "回到大文件页").clicked() {
                actions.back = true;
            }
        });
    });
}

/// 一行大文件（借用扫描结果里的字段）。
#[derive(Clone, Copy)]
struct FileRow<'a> {
    path: &'a std::path::Path,
    size: u64,
    modified: Option<std::time::SystemTime>,
    protected: bool,
}

/// 「全部结果」页的概况数据。
struct DetailSummary {
    scanned: u64,
    elapsed_ms: u64,
    file_bytes: u64,
    folder_bytes: u64,
    file_count: usize,
    folder_count: usize,
}

fn summary_card(
    ui: &mut egui::Ui,
    p: &Palette,
    s: &DetailSummary,
    want_files: bool,
    want_folders: bool,
    canceled: bool,
) {
    card(ui, p, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            widgets::section_title(ui, p, "扫描概况");
            ui.add_space(6.0);
            if canceled {
                pill(ui, "上次扫描被取消", PillKind::Caution, p);
            }
        });
        ui.add_space(8.0);
        ui.columns(4, |cols| {
            stat(
                &mut cols[0],
                p,
                "大文件夹",
                &format!("{} 个", thousands(s.folder_count as u64)),
                if want_folders { "" } else { "未统计" },
            );
            stat(
                &mut cols[1],
                p,
                "大文件",
                &format!("{} 个", thousands(s.file_count as u64)),
                if want_files { "" } else { "未统计" },
            );
            stat(
                &mut cols[2],
                p,
                "合计体积",
                &human_bytes(s.file_bytes + s.folder_bytes),
                "",
            );
            stat(
                &mut cols[3],
                p,
                "扫描耗时",
                &format!("{:.1}s", s.elapsed_ms as f32 / 1000.0),
                &format!("{} 个文件", thousands(s.scanned)),
            );
        });
    });
}

/// 汇总一份「删除建议」，把扫描层去重后的统计按风险分档。
fn advice_summary(ui: &mut egui::Ui, p: &Palette, totals: crate::bigfiles::AdviceTotals) {
    // 分档：0 可先清理、1 确认后再删、2 建议保留
    let mut bytes = [0u64; 3];
    let mut counts = [0usize; 3];

    for kind in Kind::ALL {
        let (kind_bytes, kind_count) = totals.of(kind);
        let slot = match widgets::advice_pill(kind) {
            PillKind::Safe => 0,
            PillKind::Caution => 1,
            _ => 2,
        };
        bytes[slot] += kind_bytes;
        counts[slot] += kind_count;
    }
    // 受保护条目一律归入「建议保留」
    bytes[2] += totals.protected.0;
    counts[2] += totals.protected.1;

    widgets::section(ui, p, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(RichText::new("删除建议").size(13.0).strong().color(p.text));
            ui.add_space(4.0);
            ui.label(
                RichText::new("按风险分档，仅供参考；最终由你决定")
                    .size(11.0)
                    .color(p.text_faint),
            );
        });
        ui.add_space(8.0);

        advice_line(
            ui,
            p,
            PillKind::Safe,
            "可以先清理",
            "缓存 / 构建产物、安装包这类内容删掉后基本可以重新获得",
            bytes[0],
            counts[0],
        );
        ui.add_space(4.0);
        advice_line(
            ui,
            p,
            PillKind::Caution,
            "确认后再删",
            "大体积媒体、磁盘镜像，以及判断不了用途的普通文件",
            bytes[1],
            counts[1],
        );
        ui.add_space(4.0);
        advice_line(
            ui,
            p,
            PillKind::Danger,
            "建议保留",
            "系统或已安装程序的一部分，以及本工具的保护目录",
            bytes[2],
            counts[2],
        );

        ui.add_space(8.0);
        ui.label(
            RichText::new("统计时已去除目录之间的重复计算，每个字节只归入其中一档。")
                .size(11.0)
                .color(p.text_faint),
        );
        ui.add_space(4.0);
        let tip = if counts[0] == 0 && counts[1] == 0 {
            "本次没有发现可以放心清理的内容，空间占用多半来自系统与已安装程序。"
        } else if counts[0] > 0 {
            "最省事的做法：先把上面第一档里的缓存和安装包处理掉。"
        } else {
            "第一档没有内容，建议逐个打开目录确认后再决定。"
        };
        ui.label(RichText::new(tip).size(11.5).color(p.text_weak));
    });
}

fn advice_line(
    ui: &mut egui::Ui,
    p: &Palette,
    kind: PillKind,
    title: &str,
    desc: &str,
    bytes: u64,
    count: usize,
) {
    ui.horizontal(|ui| {
        pill(ui, title, kind, p);
        ui.add_space(2.0);
        ui.label(RichText::new(desc).size(11.5).color(p.text_weak));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(
                RichText::new(format!("{count} 项 · {}", human_bytes(bytes)))
                    .size(11.5)
                    .color(p.text_faint),
            );
        });
    });
}

// ---- 大文件夹列表 ----

fn folders_card(
    ui: &mut egui::Ui,
    p: &Palette,
    rows: &[&BigFolder],
    total: u64,
    hide_nested: bool,
    mode: ListMode,
    actions: &mut Actions,
) {
    let filtered: Vec<&BigFolder> = rows
        .iter()
        .copied()
        .filter(|f| !(hide_nested && f.nested))
        .collect();
    let hidden = rows.len() - filtered.len();
    let shown = match mode {
        ListMode::Preview => filtered.len().min(PREVIEW_ROWS),
        ListMode::Full => filtered.len(),
    };
    let more = filtered.len() - shown;

    card(ui, p, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            widgets::section_title(ui, p, "大文件夹");
            ui.add_space(6.0);
            ui.label(
                RichText::new(format!(
                    "{} 个 · 合计 {}（含子目录）",
                    thousands(rows.len() as u64),
                    human_bytes(total)
                ))
                .size(11.5)
                .color(p.text_faint),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if mode == ListMode::Preview && more > 0 {
                    let label = format!("查看更多（共 {} 个）", thousands(filtered.len() as u64));
                    if widgets::ghost_button(ui, p, &label).clicked() {
                        actions.open_detail = true;
                    }
                    ui.add_space(4.0);
                }
                let mut hide = hide_nested;
                if ui
                    .checkbox(&mut hide, RichText::new("隐藏被包含的目录").size(11.5))
                    .changed()
                {
                    actions.hide_nested = Some(hide);
                }
            });
        });
        ui.add_space(6.0);

        for folder in filtered.iter().take(shown) {
            folder_row(ui, p, folder, actions);
            ui.add_space(6.0);
        }

        ui.add_space(2.0);
        // 体积是整棵子树的递归合计，所以上层目录会包含下层 —— 这点必须说清楚
        let mut note = String::from("体积是整棵子目录的递归合计，所以上层目录会包含下层目录。");
        if mode == ListMode::Preview && more > 0 {
            note.push_str(&format!(
                "这里只列体积最大的 {shown} 个，其余 {} 个点右上角「查看更多」查看。",
                thousands(more as u64)
            ));
        } else if hidden > 0 {
            note.push_str(&format!("已隐藏 {hidden} 个被包含的条目。"));
        } else {
            note.push_str("被包含的条目会标出「已包含」。");
        }
        ui.label(RichText::new(note).size(11.0).color(p.text_faint));
    });
}

fn folder_row(ui: &mut egui::Ui, p: &Palette, folder: &BigFolder, actions: &mut Actions) {
    let kind = advice::for_dir(&folder.path);
    let name = folder
        .path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| folder.path.to_string_lossy().to_string());

    egui::Frame::new()
        .fill(p.surface_alt)
        .corner_radius(CornerRadius::same(crate::theme::CARD_RADIUS))
        .inner_margin(egui::Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(shorten(&name, 46))
                                .size(13.0)
                                .strong()
                                .color(p.text),
                        );
                        if folder.protected {
                            pill(ui, "受保护", PillKind::Neutral, p);
                        } else {
                            pill(ui, kind.short(), widgets::advice_pill(kind), p);
                        }
                        if folder.nested {
                            pill(ui, "已包含", PillKind::Neutral, p);
                        }
                    });
                    ui.label(
                        RichText::new(shorten(&folder.path.to_string_lossy(), 104))
                            .size(10.5)
                            .color(p.text_faint),
                    );
                    ui.add_space(2.0);
                    ui.label(
                        RichText::new(format!(
                            "{} 个文件 · 最近修改 {}",
                            thousands(folder.files),
                            crate::cleaner::format_time(folder.modified)
                        ))
                        .size(10.5)
                        .color(p.text_faint),
                    );
                    if !folder.protected {
                        ui.label(
                            RichText::new(format!("{} · {}", kind.label(), kind.advice()))
                                .size(10.5)
                                .color(widgets::advice_color(kind, p)),
                        );
                    }
                });

                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if !folder.protected && widgets::ghost_button(ui, p, "移入回收站").clicked()
                    {
                        actions.recycle_folder = Some(folder.path.clone());
                    }
                    if widgets::ghost_button(ui, p, "打开").clicked() {
                        crate::sysutil::open_path(&folder.path);
                    }
                    ui.add_space(6.0);
                    ui.label(widgets::size_text(folder.size, p));
                });
            });
        });
}

// ---- 大文件列表 ----

fn files_card(
    ui: &mut egui::Ui,
    p: &Palette,
    rows: &[FileRow<'_>],
    total: u64,
    mode: ListMode,
    actions: &mut Actions,
) {
    let shown = match mode {
        ListMode::Preview => rows.len().min(PREVIEW_ROWS),
        ListMode::Full => rows.len(),
    };
    let more = rows.len() - shown;

    card(ui, p, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            widgets::section_title(ui, p, "大文件");
            ui.add_space(6.0);
            ui.label(
                RichText::new(format!(
                    "{} 个 · 合计 {}",
                    thousands(rows.len() as u64),
                    human_bytes(total)
                ))
                .size(11.5)
                .color(p.text_faint),
            );
            if mode == ListMode::Preview && more > 0 {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let label = format!("查看更多（共 {} 个）", thousands(rows.len() as u64));
                    if widgets::ghost_button(ui, p, &label).clicked() {
                        actions.open_detail = true;
                    }
                });
            }
        });
        ui.add_space(6.0);

        for row in rows.iter().take(shown) {
            file_row(ui, p, row, actions);
            ui.add_space(6.0);
        }

        ui.add_space(2.0);
        let note = if mode == ListMode::Preview && more > 0 {
            format!(
                "这里只列体积最大的 {shown} 个，其余 {} 个点右上角「查看更多」查看。",
                thousands(more as u64)
            )
        } else {
            "「移入回收站」会走与其他清理相同的安全检查，删错也能找回来。".to_string()
        };
        ui.label(RichText::new(note).size(11.0).color(p.text_faint));
    });
}

fn file_row(ui: &mut egui::Ui, p: &Palette, row: &FileRow<'_>, actions: &mut Actions) {
    let path = row.path;
    let (size, modified, protected) = (row.size, row.modified, row.protected);
    let kind = advice::for_file(path);
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string_lossy().to_string());

    egui::Frame::new()
        .fill(p.surface_alt)
        .corner_radius(CornerRadius::same(crate::theme::CARD_RADIUS))
        .inner_margin(egui::Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(shorten(&name, 52))
                                .size(13.0)
                                .strong()
                                .color(p.text),
                        );
                        if protected {
                            pill(ui, "受保护", PillKind::Neutral, p);
                        } else {
                            pill(ui, kind.short(), widgets::advice_pill(kind), p);
                        }
                    });
                    ui.label(
                        RichText::new(shorten(&path.to_string_lossy(), 110))
                            .size(10.5)
                            .color(p.text_faint),
                    );
                    ui.add_space(2.0);
                    ui.label(
                        RichText::new(if protected {
                            advice::PROTECTED_ADVICE.to_string()
                        } else {
                            format!("{} · {}", kind.label(), kind.advice())
                        })
                        .size(10.5)
                        .color(if protected {
                            p.text_faint
                        } else {
                            widgets::advice_color(kind, p)
                        }),
                    );
                });

                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if !protected && widgets::ghost_button(ui, p, "移入回收站").clicked() {
                        actions.recycle_file = Some(path.to_path_buf());
                    }
                    if widgets::ghost_button(ui, p, "定位").clicked() {
                        crate::sysutil::reveal_in_file_manager(path);
                    }
                    ui.add_space(6.0);
                    ui.vertical(|ui| {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(widgets::size_text(size, p));
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(
                                RichText::new(crate::cleaner::format_time(modified))
                                    .size(10.5)
                                    .color(p.text_faint),
                            );
                        });
                    });
                });
            });
        });
}

fn stat(ui: &mut egui::Ui, p: &Palette, label: &str, value: &str, sub: &str) {
    ui.vertical(|ui| {
        ui.label(RichText::new(label).size(11.5).color(p.text_weak));
        ui.add_space(2.0);
        ui.label(RichText::new(value).size(15.5).strong().color(p.text));
        if !sub.is_empty() {
            ui.label(RichText::new(sub).size(10.5).color(p.text_faint));
        }
    });
}

fn shorten(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let head: String = text.chars().take(limit.saturating_sub(1)).collect();
    format!("{head}…")
}
