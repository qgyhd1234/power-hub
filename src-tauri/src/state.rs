//! 上位机共享状态：UI 与本地 API 同源（ADR-0003）。

use std::collections::VecDeque;
use std::sync::{Arc, RwLock};

use serde::Serialize;
use tokio::sync::watch;

use crate::protocol::{ChannelTelemetry, CHANNELS};

/// 电流历史窗口：500ms × 120 = 60s
pub const HISTORY_LEN: usize = 120;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DeviceInfo {
    pub name: String,
    pub vid: u16,
    pub pid: u16,
}

pub struct HubState {
    pub connected: bool,
    /// 演示模式（ADR-0004）：数据来自内置模拟器而非真实硬件
    pub demo: bool,
    pub port: Option<String>,
    pub fw_version: Option<String>,
    pub poll_interval_ms: u64,
    pub telemetry: [ChannelTelemetry; CHANNELS],
    /// 每通道电流（单位 A）历史，最旧在前
    pub histories: [VecDeque<f64>; CHANNELS],
    pub devices: [Option<DeviceInfo>; CHANNELS],
}

impl HubState {
    pub fn new() -> Self {
        Self {
            connected: false,
            demo: false,
            port: None,
            fw_version: None,
            poll_interval_ms: 500,
            telemetry: [ChannelTelemetry::default(); CHANNELS],
            histories: std::array::from_fn(|_| VecDeque::with_capacity(HISTORY_LEN)),
            devices: std::array::from_fn(|_| None),
        }
    }
}

pub struct SharedInner {
    pub hub: RwLock<HubState>,
    /// 状态更新通知（WS 推送用）
    pub updated: watch::Sender<()>,
}

pub type SharedState = Arc<SharedInner>;

pub fn notify(shared: &SharedState) {
    let _ = shared.updated.send(());
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelStatus {
    pub index: u8,
    pub on: bool,
    /// V
    pub voltage: f64,
    /// A
    pub current: f64,
    /// W
    pub power: f64,
    /// 电流历史（A），最旧在前
    pub history: Vec<f64>,
    pub device: Option<DeviceInfo>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HubStatus {
    pub connected: bool,
    pub demo: bool,
    pub port: Option<String>,
    pub fw_version: Option<String>,
    pub poll_interval_ms: u64,
    pub channels: Vec<ChannelStatus>,
    pub total_power: f64,
    pub total_current: f64,
    pub device_count: usize,
}

pub fn snapshot(shared: &SharedState) -> HubStatus {
    let hub = shared.hub.read().expect("HubState 锁中毒");
    let channels: Vec<ChannelStatus> = hub
        .telemetry
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let voltage = t.mv as f64 / 1000.0;
            let current = t.c01ma as f64 / 10_000.0;
            ChannelStatus {
                index: (i + 1) as u8,
                on: t.sw,
                voltage,
                current,
                power: voltage * current,
                history: hub.histories[i].iter().copied().collect(),
                device: hub.devices[i].clone(),
            }
        })
        .collect();
    let total_power: f64 = channels.iter().map(|c| c.power).sum();
    let total_current: f64 = channels.iter().map(|c| c.current).sum();
    let device_count = channels.iter().filter(|c| c.device.is_some()).count();
    HubStatus {
        connected: hub.connected,
        demo: hub.demo,
        port: hub.port.clone(),
        fw_version: hub.fw_version.clone(),
        poll_interval_ms: hub.poll_interval_ms,
        channels,
        total_power,
        total_current,
        device_count,
    }
}
