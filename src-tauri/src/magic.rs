//! ⚡ 魔术字 —— 拿到硬件后更新此处（见 docs/protocol.md §8）
//!
//! 全仓库唯一需要随实物硬件修订的文件。

/// hub 主控芯片 USB VID/PID；`0x0000` 表示未配置，
/// 此时设备树定位退化用"下游设备数 = 7"特征匹配。
///
/// 实测（2026-10-19）：TI TUSB8041 两片级联组成 1 转 7（4+4 口，级联占 1 口）。
/// 每片物理 hub 在系统里枚举为两个并联 hub 节点：高速半 HS=`0451:8142`、
/// SuperSpeed 半 SS=`0451:8140`（VID 相同、PID 不同），USB3 设备走 SS 半边——
/// 设备树根扫描只按 VID 匹配（见 usb.rs find_hub_roots），本 PID 留作档案。
#[allow(dead_code)]
pub const HUB_VID: u16 = 0x0451;
#[allow(dead_code)]
pub const HUB_PID: u16 = 0x8142;

/// 控制串口芯片 CH340 的 USB VID/PID（系列默认描述符，出厂固定）。
/// 串口列表按此过滤优先展示 hub 控制口；无命中时回退展示全部串口。
pub const CONTROL_VID: u16 = 0x1A86;
pub const CONTROL_PID: u16 = 0x7523;

/// 特征匹配预期的下游端口数。
pub const EXPECTED_PORTS: usize = 7;

/// 本地 API（ADR-0003）。
pub const API_ADDR: &str = "127.0.0.1:8756";
pub const API_HEADER_KEY: &str = "x-power-hub";
pub const API_HEADER_VALUE: &str = "v1";
