//! 轮询引擎：独占串口的单一后台线程。
//!
//! 所有串口操作（连接、轮询、电源控制）都串行化在本线程内，
//! 其余模块通过命令通道请求、oneshot 回传结果。

use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use tauri::Emitter;
use tokio::sync::oneshot;

use crate::protocol::{Response, CHANNELS};
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
    fn transact(&mut self, payload: &str) -> Result<Response, TxError> {
        match self {
            Session::Serial(s) => s.transact(payload),
            // 虚拟 hub 对一切合法命令都有应答；None 不会出现
            Session::Mock(m) => m.transact(payload).ok_or(TxError::BadLine),
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
    let mut next_usb = Instant::now();

    loop {
        while let Ok(cmd) = cmd_rx.try_recv() {
            match cmd {
                ControlCommand::Connect { path, resp } => {
                    match SerialSession::open(&path) {
                        Ok(mut s) => {
                            let fw = s.transact("VER?").ok().and_then(|r| match r {
                                Response::Ver { fw, .. } => Some(fw),
                                _ => None,
                            });
                            {
                                let mut hub = shared.hub.write().expect("HubState 锁中毒");
                                hub.connected = true;
                                hub.demo = false;
                                hub.port = Some(path);
                                hub.fw_version = fw.clone();
                            }
                            failures = 0;
                            next_poll = Instant::now();
                            state::notify(&shared);
                            emit(&handle, &shared);
                            let _ = resp.send(Ok(fw));
                            session = Some(Session::Serial(s));
                        }
                        Err(e) => {
                            let _ = resp.send(Err(e));
                        }
                    }
                }
                ControlCommand::StartDemo { resp } => {
                    session = Some(Session::Mock(MockHub::new()));
                    {
                        let mut hub = shared.hub.write().expect("HubState 锁中毒");
                        hub.connected = true;
                        hub.demo = true;
                        hub.port = Some("演示模式".into());
                        hub.fw_version = Some(DEMO_FW.into());
                    }
                    failures = 0;
                    next_poll = Instant::now();
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
                        let arg = if on { "ON" } else { "OFF" };
                        let result = match s.transact(&format!("SW,{ch},{arg}")) {
                            Ok(Response::Ok) => Ok(()),
                            Ok(Response::Err { code, msg }) => Err(format!("hub 报错 {code}: {msg}")),
                            Ok(_) => Err("hub 应答异常".into()),
                            Err(e) => Err(e.to_string()),
                        };
                        if result.is_ok() {
                            next_poll = Instant::now();
                        }
                        let _ = resp.send(result);
                    }
                },
            }
        }

        let now = Instant::now();
        let connected = session.is_some();
        if connected && now >= next_poll {
            let interval = Duration::from_millis(shared.hub.read().expect("HubState 锁中毒").poll_interval_ms);
            poll_once(&handle, &shared, session.as_mut().expect("connected"), &mut failures);
            if failures >= MAX_CONSECUTIVE_FAILURES {
                session = None;
                mark_disconnected(&shared);
            }
            next_poll = Instant::now() + interval;
        }
        if now >= next_usb {
            // 演示模式的设备表随每轮 PWR? 注入（见 poll_once），不走系统枚举
            if matches!(session, Some(Session::Serial(_))) {
                #[cfg(windows)]
                if let Ok(map) = crate::usb::enumerate_channels() {
                    let mut hub = shared.hub.write().expect("HubState 锁中毒");
                    if hub.devices != map {
                        hub.devices = map;
                        drop(hub);
                        state::notify(&shared);
                    }
                }
            }
            next_usb = now + USB_REFRESH;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn poll_once(
    handle: &tauri::AppHandle,
    shared: &SharedState,
    session: &mut Session,
    failures: &mut u32,
) {
    match session.transact("PWR?") {
        Ok(Response::Pwr(telemetry)) => {
            *failures = 0;
            {
                let mut hub = shared.hub.write().expect("HubState 锁中毒");
                hub.connected = true;
                for (i, t) in telemetry.iter().enumerate().take(CHANNELS) {
                    hub.telemetry[i] = *t;
                    let hist = &mut hub.histories[i];
                    hist.push_back(t.c01ma as f64 / 10_000.0);
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
        Ok(_) | Err(_) => {
            *failures += 1;
        }
    }
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
