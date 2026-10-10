//! power-hub 上位机：Tauri 2 应用组装与对外命令。

mod api;
mod magic;
mod poller;
mod serial;
mod state;
#[cfg(windows)]
mod usb;

// 协议编解码是独立 crate（可被固件侧/技能侧复用，单测不依赖 tauri），
// 在 crate 根以 `crate::protocol::` 路径 re-export。
use power_hub_protocol as protocol;

use std::sync::mpsc;
use std::sync::{Arc, RwLock};

use poller::ControlCommand;
use state::SharedState;
use tauri::{Emitter, Manager};
use tokio::sync::watch;

pub struct Runtime {
    shared: SharedState,
    cmd_tx: mpsc::Sender<ControlCommand>,
}

#[tauri::command]
fn list_ports() -> Vec<String> {
    serial::list_ports()
}

#[tauri::command]
async fn connect(port: String, rt: tauri::State<'_, Runtime>) -> Result<Option<String>, String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    rt.cmd_tx
        .send(ControlCommand::Connect { path: port, resp: tx })
        .map_err(|_| "hub 轮询服务不可用".to_string())?;
    match rx.await {
        Ok(result) => result,
        Err(_) => Err("hub 轮询线程无应答".into()),
    }
}

#[tauri::command]
fn disconnect(rt: tauri::State<Runtime>) {
    let _ = rt.cmd_tx.send(ControlCommand::Disconnect);
}

#[tauri::command]
async fn set_channel(ch: u8, on: bool, rt: tauri::State<'_, Runtime>) -> Result<(), String> {
    if !(1..=7).contains(&ch) {
        return Err("通道号范围 1~7".into());
    }
    let (tx, rx) = tokio::sync::oneshot::channel();
    rt.cmd_tx
        .send(ControlCommand::SetChannel { ch, on, resp: tx })
        .map_err(|_| "hub 轮询服务不可用".to_string())?;
    match rx.await {
        Ok(result) => result,
        Err(_) => Err("hub 轮询线程无应答".into()),
    }
}

#[tauri::command]
fn get_status(rt: tauri::State<Runtime>) -> state::HubStatus {
    state::snapshot(&rt.shared)
}

#[tauri::command]
fn set_poll_interval(ms: u64, rt: tauri::State<Runtime>) {
    let ms = ms.clamp(200, 5_000);
    if let Ok(mut hub) = rt.shared.hub.write() {
        hub.poll_interval_ms = ms;
    }
}

#[tauri::command]
fn refresh_devices(rt: tauri::State<Runtime>) -> Result<(), String> {
    #[cfg(windows)]
    {
        let map = usb::enumerate_channels()?;
        if let Ok(mut hub) = rt.shared.hub.write() {
            hub.devices = map;
        }
        state::notify(&rt.shared);
    }
    #[cfg(not(windows))]
    let _ = &rt;
    Ok(())
}

pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let handle = app.handle().clone();
            let (cmd_tx, cmd_rx) = mpsc::channel::<ControlCommand>();
            let shared: SharedState = Arc::new(state::SharedInner {
                hub: RwLock::new(state::HubState::new()),
                updated: watch::channel(()).0,
            });

            poller::spawn(handle.clone(), shared.clone(), cmd_rx);

            {
                let shared = shared.clone();
                let api_handle = handle.clone();
                let api_tx = cmd_tx.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = api::serve(shared, api_tx).await {
                        let _ = api_handle.emit("api-error", e);
                    }
                });
            }

            app.manage(Runtime { shared, cmd_tx });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_ports,
            connect,
            disconnect,
            set_channel,
            get_status,
            set_poll_interval,
            refresh_devices
        ])
        .run(tauri::generate_context!())
        .expect("power-hub 启动失败");
}
