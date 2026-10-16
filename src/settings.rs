//! 设置持久化。
//!
//! 存放位置由 [`crate::sysutil::settings_path`] 决定：
//! Windows 是 `%LOCALAPPDATA%\CleanC\settings.json`，
//! macOS 是 `~/Library/Application Support/CleanC/settings.json`，
//! Linux 是 `~/.local/share/cleanc/settings.json`（遵循 `XDG_DATA_HOME`）。

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::cleaner::CleanMode;
use crate::sysutil;

/// 主题偏好。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThemePref {
    System,
    Light,
    Dark,
}

impl ThemePref {
    pub const ALL: [ThemePref; 3] = [ThemePref::System, ThemePref::Light, ThemePref::Dark];

    pub fn label(self) -> &'static str {
        match self {
            ThemePref::System => "跟随系统",
            ThemePref::Light => "浅色",
            ThemePref::Dark => "深色",
        }
    }
}

fn default_min_age() -> u32 {
    7
}

fn default_big_file_mb() -> u64 {
    500
}

fn default_true() -> bool {
    true
}

/// 用户设置。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme: ThemePref,
    /// 临时文件最小年龄（天），0 表示不过滤
    #[serde(default = "default_min_age")]
    pub min_age_days: u32,
    pub clean_mode: CleanMode,
    /// 演练模式：只统计不删除
    pub dry_run: bool,
    /// 用户勾选的清理项 id。
    ///
    /// `None` 表示首次运行，使用各清理项的默认勾选状态；
    /// `Some(vec![])` 表示用户主动取消了全部勾选（合法状态）。
    pub selected_targets: Option<Vec<String>>,
    /// 大文件扫描的体积下限（MB）
    #[serde(default = "default_big_file_mb")]
    pub big_file_min_mb: u64,
    /// 大文件扫描的自定义根目录（为空时使用默认值）
    pub big_file_roots: Vec<String>,
    /// 大文件页是否统计文件
    #[serde(default = "default_true")]
    pub big_want_files: bool,
    /// 大文件页是否统计文件夹
    #[serde(default = "default_true")]
    pub big_want_folders: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: ThemePref::Light,
            min_age_days: default_min_age(),
            clean_mode: CleanMode::RecycleBin,
            dry_run: false,
            selected_targets: None,
            big_file_min_mb: default_big_file_mb(),
            big_file_roots: Vec::new(),
            big_want_files: true,
            big_want_folders: true,
        }
    }
}

impl Settings {
    pub fn load() -> Self {
        let path = sysutil::settings_path();
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Self::default();
        };
        serde_json::from_str(&text).unwrap_or_default()
    }

    pub fn save(&self) -> std::io::Result<()> {
        let dir = sysutil::app_data_dir();
        sysutil::ensure_dir(&dir);
        let text = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        std::fs::write(sysutil::settings_path(), text)
    }

    /// 大文件扫描的默认根目录。
    ///
    /// 只给**一个**目录：当前用户的主目录（Windows 是 `%USERPROFILE%`，
    /// macOS / Linux 是 `$HOME`）。理由是"占空间的大头基本都在自己的家目录里"，
    /// 而多扫一个 `C:\ProgramData` / `/var` 既慢又需要管理员权限，收益很低。
    /// 需要扩大范围时，用户可以在「大文件」页里自己加。
    pub fn default_big_file_roots() -> Vec<PathBuf> {
        sysutil::home_dir().into_iter().collect()
    }

    pub fn big_file_roots(&self) -> Vec<PathBuf> {
        if self.big_file_roots.is_empty() {
            Self::default_big_file_roots()
        } else {
            self.big_file_roots.iter().map(PathBuf::from).collect()
        }
    }
}
