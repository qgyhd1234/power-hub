//! 轮询引擎：独占串口的单一后台线程。
//!
//! 所有串口操作（连接、轮询、电源控制）都串行化在本线程内，
//! 其余模块通过命令通道请求、oneshot 回传结果。

use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use log::{debug, info, warn};
use tauri::Emitter;
use tokio::sync::oneshot;

use crate::protocol::{Command, Response, ChannelTelemetry, CHANNELS};
use crate::serial::{SerialSession, TxError};
use crate::state::{self, SharedState, HISTORY_LEN};
use power_hub_protocol::mock::{DemoDevice, MockHub, DEMO_FW};

/// 连续轮询失败达到该次数即判定断连（串口拔出等）。
const MAX_CONSECUTIVE_FAILURES: u32 = 3;
/// 设备树枚举节流周期。
const USB_REFRESH: Duration = Duration::from_secs(2);

/// 与 hub 的会话：真实串口或演示模式（ADR-0004）。
enum Session {
    Serial(SerialSession),
    Mock(MockHub),
}

impl Session {
    fn transact(&mut self, command: &Command) -> Result<Response, TxError> {
        match self {
            Session::Serial(s) => s.transact(command),
            // 虚拟 hub 对一切合法命令都有应答；None/不可解码不会出现
            Session::Mock(m) => {
                let frame = m.handle(&command.encode()).ok_or(TxError::Timeout)?;
                crate::protocol::decode_response(&frame).ok_or(TxError::Timeout)
            }
        }
    }
}

pub enum ControlCommand {
    Connect { path: String, resp: oneshot::Sender<Result<Option<String>, String>> },
    StartDemo { resp: oneshot::Sender<Result<(), String>> },
    Disconnect,
    SetChannel { ch: u8, on: bool, resp: oneshot::Sender<Result<(), String>> },
}

pub fn spawn(handle: tauri::AppHandle, shared: SharedState, cmd_rx: Receiver<ControlCommand>) {
    std::thread::Builder::new()
        .name("hub-poller".into())
        .spawn(move || run(handle, shared, cmd_rx))
        .expect("启动轮询线程失败");
}

fn run(handle: tauri::AppHandle, shared: SharedState, cmd_rx: Receiver<ControlCommand>) {
    let mut session: Option<Session> = None;
    let mut failures = 0u32;
    let mut next_poll = Instant::now();
    let mut next_status = Instant::now();
    let mut next_usb = Instant::now();

    loop {
        while let Ok(cmd) = cmd_rx.try_recv() {
            match cmd {
                ControlCommand::Connect { path, resp } => {
                    match SerialSession::open(&path) {
                        Ok(mut s) => match connect_handshake(&mut s) {
                            Ok((fw, switches)) => {
                                info!("已连接 {path}（fw=0x{fw:02X}，通道掩码 0x{switches:02X}）");
                                {
                                    let mut hub = shared.hub.write().expect("HubState 锁中毒");
                                    hub.connected = true;
                                    hub.demo = false;
                                    hub.port = Some(path.clone());
                                    hub.fw_version = Some(format!("0x{fw:02X}"));
                                    // 握手应答带回通道状态：连接瞬间同步 UI（规范 §4.1）
                                    apply_switches(&mut hub.telemetry, switches);
                                }
                                failures = 0;
                                next_poll = Instant::now();
                                next_status = Instant::now() + status_poll_interval(&shared);
                                state::notify(&shared);
                                emit(&handle, &shared);
                                let _ = resp.send(Ok(Some(format!("0x{fw:02X}"))));
                                session = Some(Session::Serial(s));
                            }
                            Err(reason) => {
                                warn!("{path}: {reason}");
                                let _ = resp.send(Err(format!("{path}: {reason}")));
                            }
                        },
                        Err(e) => {
                            warn!("打开串口失败：{e}");
                            let _ = resp.send(Err(e));
                        }
                    }
                }
                ControlCommand::StartDemo { resp } => {
                    session = Some(Session::Mock(MockHub::new()));
                    info!("进入演示模式（数据为模拟）");
                    {
                        let mut hub = shared.hub.write().expect("HubState 锁中毒");
                        hub.connected = true;
                        hub.demo = true;
                        hub.port = Some("演示模式".into());
                        hub.fw_version = Some(format!("0x{DEMO_FW:02X}"));
                    }
                    failures = 0;
                    next_poll = Instant::now();
                    next_status = Instant::now() + status_poll_interval(&shared);
                    state::notify(&shared);
                    emit(&handle, &shared);
                    let _ = resp.send(Ok(()));
                }
                ControlCommand::Disconnect => {
                    session = None;
                    mark_disconnected(&shared);
                    emit(&handle, &shared);
                }
                ControlCommand::SetChannel { ch, on, resp } => match session.as_mut() {
                    None => {
                        let _ = resp.send(Err("未连接 hub".into()));
                    }
                    Some(s) => {
                        let result = match Command::set_channel(ch, on) {
                            None => {
                                warn!("非法通道号 {ch}（应为 1~7）");
                                Err("通道号范围 1~7".into())
                            }
                            Some(command) => match s.transact(&command) {
                                // 应答掩码即动作执行后的全通道状态（规范 §4.2）
                                Ok(Response::Switches(mask)) => {
                                    info!(
                                        "通道 {ch} → {}（状态掩码 0x{mask:02X}）",
                                        if on { "开" } else { "关" }
                                    );
                                    commit_switches(&handle, &shared, mask);
                                    next_poll = Instant::now();
                                    Ok(())
                                }
                                other => {
                                    let reason = tx_fail_reason(&other);
                                    warn!("通道 {ch} 设置失败：{reason}");
                                    Err(reason)
                                }
                            },
                        };
                        let _ = resp.send(result);
                    }
                },
            }
        }

        let now = Instant::now();
        let connected = session.is_some();
        if connected && now >= next_status {
            let status_ms = shared.hub.read().expect("HubState 锁中毒").status_poll_interval_ms;
            if status_ms > 0 {
                match session.as_mut().expect("connected").transact(&Command::Handshake) {
                    Ok(resp @ Response::Handshake { switches, .. }) if resp.is_valid_handshake() => {
                        failures = 0;
                        commit_switches(&handle, &shared, switches);
                    }
                    _ => {
                        warn!("状态轮询失败，计 1 次失败");
                        failures += 1;
                    }
                }
            }
            // 禁用（0）时只按 1 秒节拍重查配置，不发送任何 0x00
            next_status = now + status_poll_interval(&shared);
        }
        if connected && now >= next_poll {
            let interval = Duration::from_millis(shared.hub.read().expect("HubState 锁中毒").poll_interval_ms);
            poll_once(&handle, &shared, session.as_mut().expect("connected"), &mut failures);
            next_poll = Instant::now() + interval;
        }
        if connected && failures >= MAX_CONSECUTIVE_FAILURES {
            warn!("连续 {MAX_CONSECUTIVE_FAILURES} 次轮询失败，判定断连");
            session = None;
            mark_disconnected(&shared);
            emit(&handle, &shared);
        }
        if now >= next_usb {
            // 演示模式的设备表随每轮轮询注入（见 poll_once），不走系统枚举
            if matches!(session, Some(Session::Serial(_))) {
                #[cfg(windows)]
                match crate::usb::enumerate_channels() {
                    Ok(map) => {
                        let mut hub = shared.hub.write().expect("HubState 锁中毒");
                        if hub.devices != map {
                            hub.devices = map;
                            drop(hub);
                            state::notify(&shared);
                        }
                    }
                    // 2 秒一响，放 Debug 防刷屏
                    Err(e) => debug!("USB 设备树枚举失败：{e}"),
                }
            }
            next_usb = now + USB_REFRESH;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// 连接握手（规范 §4.1）：发 0x00 并按判据识别；通过则返回 (fw, 状态掩码)。
fn connect_handshake(s: &mut SerialSession) -> Result<(u8, u8), String> {
    match s.transact(&Command::Handshake) {
        Ok(resp @ Response::Handshake { fw, switches, .. }) if resp.is_valid_handshake() => {
            Ok((fw, switches))
        }
        Ok(_) => Err("不是 power-hub（握手判据不符）".into()),
        Err(e) => Err(format!("握手失败：{e}")),
    }
}

fn apply_switches(telemetry: &mut [ChannelTelemetry; CHANNELS], mask: u8) {
    for (i, t) in telemetry.iter_mut().enumerate() {
        t.sw = mask & (1 << i) != 0;
    }
}

/// 把开关掩码写入共享状态并推送 UI（通道设置应答与 3 秒状态轮询共用）。
fn commit_switches(handle: &tauri::AppHandle, shared: &SharedState, switches: u8) {
    {
        let mut hub = shared.hub.write().expect("HubState 锁中毒");
        apply_switches(&mut hub.telemetry, switches);
    }
    state::notify(shared);
    emit(handle, shared);
}

/// 把一次事务结果归一成给人看的失败原因（非法应答 / 超时 / IO）。
fn tx_fail_reason(result: &Result<Response, TxError>) -> String {
    match result {
        Ok(_) => "应答指令异常".into(),
        Err(e) => e.to_string(),
    }
}

/// 状态轮询间隔：读界面配置；禁用（0）时退化为 1 秒的重查节拍，以响应重新启用。
fn status_poll_interval(shared: &SharedState) -> Duration {
    let ms = shared.hub.read().expect("HubState 锁中毒").status_poll_interval_ms;
    if ms == 0 {
        Duration::from_secs(1)
    } else {
        Duration::from_millis(ms)
    }
}

fn poll_once(
    handle: &tauri::AppHandle,
    shared: &SharedState,
    session: &mut Session,
    failures: &mut u32,
) {
    // 一轮 = 依次读 7 个通道电流（规范 §7）；全部成功才提交，
    // 任一事务失败该轮计 1 次失败；通道开关态由设置应答与状态轮询（间隔可配置）同步。
    let mut currents = [0.0f32; CHANNELS];
    for channel in 1..=CHANNELS as u8 {
        match session.transact(&Command::ReadCurrent { channel }) {
            Ok(Response::Current { channel: ch, ma }) if ch == channel => {
                currents[(ch - 1) as usize] = ma;
            }
            other => {
                warn!("读通道 {channel} 电流失败：{}，本轮中止", tx_fail_reason(&other));
                *failures += 1;
                return;
            }
        }
    }
    *failures = 0;
    debug!("一轮轮询完成，电流（mA）：{currents:?}");
    {
        let mut hub = shared.hub.write().expect("HubState 锁中毒");
        hub.connected = true;
        for (i, ma) in currents.into_iter().enumerate() {
            hub.telemetry[i].ma = ma;
            let hist = &mut hub.histories[i];
            hist.push_back(ma as f64 / 1000.0);
            while hist.len() > HISTORY_LEN {
                hist.pop_front();
            }
        }
        if let Session::Mock(m) = session {
            hub.devices = m.device_infos().map(|slot| {
                slot.map(|d: DemoDevice| state::DeviceInfo {
                    name: d.name.into(),
                    vid: d.vid,
                    pid: d.pid,
                })
            });
        }
    }
    state::notify(shared);
    emit(handle, shared);
}

fn mark_disconnected(shared: &SharedState) {
    let mut hub = shared.hub.write().expect("HubState 锁中毒");
    hub.connected = false;
    hub.demo = false;
    hub.port = None;
    hub.fw_version = None;
    drop(hub);
    state::notify(shared);
}

fn emit(handle: &tauri::AppHandle, shared: &SharedState) {
    let _ = handle.emit("telemetry", state::snapshot(shared));
}
