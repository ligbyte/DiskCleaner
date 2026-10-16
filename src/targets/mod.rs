//! 清理目标白名单。
//!
//! **这是整个工具唯一允许删除文件的地方。** 所有路径都是硬编码的已知安全位置，
//! 不做任何用户输入的路径拼接，也不扫描用户文档目录。
//! 每个目标的 `risk` / `default_on` 共同决定了它在界面上的默认勾选状态。
//!
//! 白名单按平台分别维护：
//!
//! | 平台 | 模块 | 主要清理位置 |
//! |------|------|--------------|
//! | Windows | [`windows`] | `%TEMP%`、`C:\Windows\Temp`、Windows 更新缓存、各浏览器缓存… |
//! | macOS | [`macos`] | `~/Library/Caches`、`~/Library/Logs`、`/Library/Caches`… |
//! | Linux | [`linux`] | `~/.cache`、`/var/cache/*`、`/var/log`、`/tmp`… |
//!
//! 三个平台共享同一套 [`CleanTarget`] 结构与安全层，因此"换平台"只需要换这份名单。

use crate::model::CleanTarget;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

/// 构建当前机器上可用的清理目标列表。
pub fn build() -> Vec<CleanTarget> {
    #[cfg(windows)]
    {
        windows::build()
    }
    #[cfg(target_os = "macos")]
    {
        macos::build()
    }
    #[cfg(target_os = "linux")]
    {
        linux::build()
    }
    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
    {
        Vec::new()
    }
}

/// 当前平台的白名单条目数量（用于文档与自检输出）。
#[allow(dead_code)]
pub fn platform_name() -> &'static str {
    crate::sysutil::PLATFORM.label()
}
