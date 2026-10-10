//! ⚡ 魔术字 —— 拿到硬件后更新此处（见 docs/protocol.md §8）
//!
//! 全仓库唯一需要随实物硬件修订的文件。

/// hub 主控芯片 USB VID/PID；`0x0000` 表示未配置，
/// 此时设备树定位退化用"下游设备数 = 7"特征匹配。
pub const HUB_VID: u16 = 0x0000;
pub const HUB_PID: u16 = 0x0000;

/// 特征匹配预期的下游端口数。
pub const EXPECTED_PORTS: usize = 7;

/// 本地 API（ADR-0003）。
pub const API_ADDR: &str = "127.0.0.1:8756";
pub const API_HEADER_KEY: &str = "x-power-hub";
pub const API_HEADER_VALUE: &str = "v1";
