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
use std::path::PathBuf;

use log::{error, info};
use poller::ControlCommand;
use state::SharedState;
use tauri::{Emitter, Manager};
use tauri_plugin_log::{RotationStrategy, Target, TargetKind};
use tokio::sync::watch;

/// Debug 级开关环境变量（`POWER_HUB_LOG=debug`，spec `.scratch/power-hub-logs/`）。
const LOG_LEVEL_ENV: &str = "POWER_HUB_LOG";
/// 单个日志文件上限（KB）；10240 KB = 10 MiB，KeepOne 滚动后共两份、20 MiB 封顶。
const LOG_MAX_FILE_KB: u128 = 10_240;
/// 日志回退目录子路径，须与 tauri.conf.json 的 identifier 保持一致。
const APP_IDENTIFIER: &str = "dev.powerhub.app";

/// verbose 判定：`--verbose` 启动参数或 `POWER_HUB_LOG=debug`（大小写不敏感）任一命中。
fn verbose_enabled(flag_arg: bool, env_value: Option<&str>) -> bool {
    flag_arg || env_value.is_some_and(|v| v.eq_ignore_ascii_case("debug"))
}

/// 候选目录可作日志目录：能创建且探针可写（目录存在不代表有写权限）。
fn usable_log_dir(candidate: PathBuf) -> Option<PathBuf> {
    std::fs::create_dir_all(&candidate).ok()?;
    let probe = candidate.join(".probe");
    std::fs::write(&probe, b"ok").ok()?;
    std::fs::remove_file(&probe).ok()?;
    Some(candidate)
}

/// 日志目录解析：exe 旁 `logs/` → 系统本地数据目录 → 临时目录。
/// exe 被放进 Program Files 等不可写位置时自动降级，保证总能拿到目录。
fn resolve_log_dir(
    exe_dir: Option<PathBuf>,
    local_data_root: Option<PathBuf>,
    temp_root: PathBuf,
) -> PathBuf {
    let candidates = [
        exe_dir.map(|d| d.join("logs")),
        local_data_root.map(|d| d.join(APP_IDENTIFIER).join("logs")),
        Some(temp_root.join("logs")),
    ];
    candidates
        .into_iter()
        .flatten()
        .find_map(usable_log_dir)
        .unwrap_or_else(std::env::temp_dir)
}

#[cfg(windows)]
fn local_data_root() -> Option<PathBuf> {
    std::env::var("LOCALAPPDATA").ok().map(PathBuf::from)
}

#[cfg(not(windows))]
fn local_data_root() -> Option<PathBuf> {
    None
}

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
async fn start_demo(rt: tauri::State<'_, Runtime>) -> Result<(), String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    rt.cmd_tx
        .send(ControlCommand::StartDemo { resp: tx })
        .map_err(|_| "hub 轮询服务不可用".to_string())?;
    match rx.await {
        Ok(result) => result,
        Err(_) => Err("hub 轮询线程无应答".into()),
    }
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
    let verbose = verbose_enabled(
        std::env::args().any(|a| a == "--verbose"),
        std::env::var(LOG_LEVEL_ENV).ok().as_deref(),
    );
    let level = if verbose { log::LevelFilter::Debug } else { log::LevelFilter::Info };
    let exe_dir =
        std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf()));
    let log_dir = resolve_log_dir(exe_dir, local_data_root(), std::env::temp_dir());

    tauri::Builder::default()
        .plugin(
            tauri_plugin_log::Builder::new()
                .targets([
                    Target::new(TargetKind::Folder {
                        path: log_dir.clone(),
                        file_name: Some("power-hub".into()),
                    }),
                    Target::new(TargetKind::Stdout),
                ])
                .level(level)
                .max_file_size(LOG_MAX_FILE_KB)
                .rotation_strategy(RotationStrategy::KeepOne)
                .build(),
        )
        .setup(move |app| {
            // 钩子必须晚于日志插件初始化，panic 栈才能落进日志文件
            std::panic::set_hook(Box::new(|info| {
                error!("panic: {info}\n{}", std::backtrace::Backtrace::force_capture());
            }));
            info!(
                "power-hub v{} 启动 | 日志目录: {} | verbose: {verbose}",
                env!("CARGO_PKG_VERSION"),
                log_dir.display()
            );

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
                        error!("本地 API 服务异常: {e}");
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
            start_demo,
            set_channel,
            get_status,
            set_poll_interval,
            refresh_devices
        ])
        .run(tauri::generate_context!())
        .expect("power-hub 启动失败");
}

#[cfg(test)]
mod log_support_tests {
    use super::*;

    fn temp_root(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("power-hub-logtest-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn prefers_writable_logs_next_to_exe() {
        let root = temp_root("exe-ok");
        let resolved = resolve_log_dir(Some(root.clone()), Some(root.join("data")), root.join("tmp"));
        assert_eq!(resolved, root.join("logs"));
        assert!(resolved.is_dir());
    }

    #[test]
    fn falls_back_when_exe_side_unwritable() {
        // exe 侧指向一个普通文件：create_dir_all 必然失败，应降级到本地数据目录
        let root = temp_root("exe-bad");
        std::fs::create_dir_all(&root).unwrap();
        let blocker = root.join("not-a-dir");
        std::fs::write(&blocker, b"x").unwrap();
        let resolved = resolve_log_dir(Some(blocker), Some(root.clone()), root.join("tmp"));
        assert_eq!(resolved, root.join("data").join("dev.powerhub.app").join("logs"));
        assert!(resolved.is_dir());
    }

    #[test]
    fn falls_back_to_temp_when_nothing_given() {
        let resolved = resolve_log_dir(None, None, temp_root("no-exe"));
        assert!(resolved.ends_with("logs"));
        assert!(resolved.is_dir());
    }

    #[test]
    fn verbose_flags() {
        assert!(!verbose_enabled(false, None));
        assert!(!verbose_enabled(false, Some("info")));
        assert!(verbose_enabled(true, None), "--verbose 参数命中");
        assert!(verbose_enabled(false, Some("debug")));
        assert!(verbose_enabled(false, Some("DEBUG")), "大小写不敏感");
    }
}
