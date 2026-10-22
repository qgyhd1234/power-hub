//! 应用配置：轮询间隔持久化（应用目录下 `config.json`）。
//!
//! 目录解析与日志同策略（exe 旁优先，不可写回退系统数据目录，见 lib.rs
//! `resolve_app_dir`）；文件缺失/损坏一律按默认值，写入用临时文件+改名保证原子性。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// 电流轮询间隔合法范围（毫秒），与 `set_poll_interval` 命令的钳制一致。
pub const POLL_INTERVAL_RANGE: (u64, u64) = (200, 5_000);
/// 状态轮询间隔合法范围（毫秒）；0 表示禁用状态轮询（通道 1 硬件问题的逃生门）。
pub const STATUS_INTERVAL_RANGE: (u64, u64) = (500, 60_000);
/// 电流轮询间隔默认值（毫秒）。
pub const DEFAULT_POLL_INTERVAL_MS: u64 = 500;
/// 状态轮询间隔默认值（毫秒）。
pub const DEFAULT_STATUS_INTERVAL_MS: u64 = 3_000;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AppConfig {
    pub poll_interval_ms: u64,
    pub status_poll_interval_ms: u64,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            poll_interval_ms: DEFAULT_POLL_INTERVAL_MS,
            status_poll_interval_ms: DEFAULT_STATUS_INTERVAL_MS,
        }
    }
}

impl AppConfig {
    /// 配置文件视为不可信输入：越界一律钳回合法范围（状态 0 = 禁用，保留）。
    fn sanitized(mut self) -> Self {
        self.poll_interval_ms = clamp_poll_interval(self.poll_interval_ms);
        self.status_poll_interval_ms = clamp_status_interval(self.status_poll_interval_ms);
        self
    }
}

/// 电流轮询间隔的钳制。
pub fn clamp_poll_interval(ms: u64) -> u64 {
    ms.clamp(POLL_INTERVAL_RANGE.0, POLL_INTERVAL_RANGE.1)
}

/// 状态轮询间隔的钳制：0 = 禁用，其余收进合法范围。
pub fn clamp_status_interval(ms: u64) -> u64 {
    if ms == 0 {
        0
    } else {
        ms.clamp(STATUS_INTERVAL_RANGE.0, STATUS_INTERVAL_RANGE.1)
    }
}

pub fn config_path(app_dir: &Path) -> PathBuf {
    app_dir.join("config.json")
}

/// 加载配置；文件缺失按默认值，损坏则告警并按默认值。返回 (配置, 是否真的读到了文件)。
pub fn load(app_dir: &Path) -> (AppConfig, bool) {
    match std::fs::read(config_path(app_dir)) {
        Ok(bytes) => match serde_json::from_slice::<AppConfig>(&bytes) {
            Ok(cfg) => (cfg.sanitized(), true),
            Err(e) => {
                log::warn!("config.json 解析失败，按默认值处理: {e}");
                (AppConfig::default(), false)
            }
        },
        Err(_) => (AppConfig::default(), false),
    }
}

/// 原子保存：先写临时文件再改名，避免写一半断电留下半个配置。
pub fn save(app_dir: &Path, config: &AppConfig) -> Result<(), String> {
    let path = config_path(app_dir);
    let tmp = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(config).map_err(|e| e.to_string())?;
    std::fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("power-hub-cfgtest-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn save_load_roundtrip() {
        let dir = temp_dir("roundtrip");
        let cfg = AppConfig { poll_interval_ms: 1_000, status_poll_interval_ms: 10_000 };
        save(&dir, &cfg).unwrap();
        let (loaded, from_file) = load(&dir);
        assert!(from_file);
        assert_eq!(loaded, cfg);
    }

    #[test]
    fn missing_file_yields_defaults() {
        let (cfg, from_file) = load(&temp_dir("missing"));
        assert!(!from_file);
        assert_eq!(cfg, AppConfig::default());
    }

    #[test]
    fn malformed_file_yields_defaults() {
        let dir = temp_dir("malformed");
        std::fs::write(config_path(&dir), "{ not json").unwrap();
        let (cfg, from_file) = load(&dir);
        assert!(!from_file);
        assert_eq!(cfg, AppConfig::default());
    }

    #[test]
    fn out_of_range_values_are_clamped() {
        let dir = temp_dir("clamp");
        let raw = AppConfig { poll_interval_ms: 1, status_poll_interval_ms: 999_999 };
        save(&dir, &raw).unwrap();
        let (cfg, _) = load(&dir);
        assert_eq!(cfg.poll_interval_ms, POLL_INTERVAL_RANGE.0);
        assert_eq!(cfg.status_poll_interval_ms, STATUS_INTERVAL_RANGE.1);
    }

    #[test]
    fn status_zero_means_disabled_and_survives() {
        let dir = temp_dir("disabled");
        let raw = AppConfig { poll_interval_ms: 500, status_poll_interval_ms: 0 };
        save(&dir, &raw).unwrap();
        let (cfg, _) = load(&dir);
        assert_eq!(cfg.status_poll_interval_ms, 0, "禁用语义不被钳制吞掉");
    }
}
