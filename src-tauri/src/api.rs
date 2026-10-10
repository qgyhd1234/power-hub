//! 本地 API（ADR-0003）：127.0.0.1:8756，与 UI 同源，供 AI 技能等客户端使用。
//!
//! 防恶意网页跨站调用：所有请求必须携带 `X-Power-Hub: v1` 自定义头，
//! 浏览器跨站"简单请求"无法携带自定义头，CORS 预检会将其拦下。

use std::sync::mpsc::Sender;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Request, State};
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use tokio::sync::oneshot;

use crate::magic;
use crate::poller::ControlCommand;
use crate::state::{self, SharedState};

#[derive(Clone)]
struct ApiCtx {
    state: SharedState,
    cmd_tx: Sender<ControlCommand>,
}

pub async fn serve(shared: SharedState, cmd_tx: Sender<ControlCommand>) -> Result<(), String> {
    let ctx = ApiCtx { state: shared, cmd_tx };
    let app = Router::new()
        .route("/api/status", get(status))
        .route("/api/channel/{ch}/{action}", post(set_channel))
        .route("/ws", get(ws))
        .layer(middleware::from_fn(check_header))
        .with_state(ctx);
    let listener = tokio::net::TcpListener::bind(magic::API_ADDR)
        .await
        .map_err(|e| format!("绑定 {} 失败: {e}", magic::API_ADDR))?;
    axum::serve(listener, app)
        .await
        .map_err(|e| format!("本地 API 服务异常: {e}"))
}

async fn check_header(req: Request, next: Next) -> Response {
    let ok = req
        .headers()
        .get(magic::API_HEADER_KEY)
        .and_then(|v| v.to_str().ok())
        == Some(magic::API_HEADER_VALUE);
    if ok {
        next.run(req).await
    } else {
        (StatusCode::FORBIDDEN, "缺少 X-Power-Hub 请求头").into_response()
    }
}

async fn status(State(ctx): State<ApiCtx>) -> Json<state::HubStatus> {
    Json(state::snapshot(&ctx.state))
}

async fn set_channel(State(ctx): State<ApiCtx>, Path((ch, action)): Path<(u8, String)>) -> Response {
    let on = match action.as_str() {
        "on" => true,
        "off" => false,
        _ => return (StatusCode::BAD_REQUEST, "action 必须是 on 或 off").into_response(),
    };
    if !(1..=7).contains(&ch) {
        return (StatusCode::BAD_REQUEST, "通道号范围 1~7").into_response();
    }
    let (tx, rx) = oneshot::channel();
    if ctx.cmd_tx.send(ControlCommand::SetChannel { ch, on, resp: tx }).is_err() {
        return (StatusCode::SERVICE_UNAVAILABLE, "hub 服务不可用").into_response();
    }
    match rx.await {
        Ok(Ok(())) => Json(serde_json::json!({ "ok": true })).into_response(),
        Ok(Err(e)) => (StatusCode::CONFLICT, e).into_response(),
        Err(_) => (StatusCode::SERVICE_UNAVAILABLE, "hub 无应答").into_response(),
    }
}

async fn ws(State(ctx): State<ApiCtx>, ws: WebSocketUpgrade) -> Response {
    ws.on_upgrade(move |socket| ws_loop(ctx, socket))
}

async fn ws_loop(ctx: ApiCtx, mut socket: WebSocket) {
    let mut rx = ctx.state.updated.subscribe();
    if send_snapshot(&ctx, &mut socket).await.is_err() {
        return;
    }
    loop {
        if rx.changed().await.is_err() {
            return;
        }
        if send_snapshot(&ctx, &mut socket).await.is_err() {
            return;
        }
    }
}

async fn send_snapshot(ctx: &ApiCtx, socket: &mut WebSocket) -> Result<(), axum::Error> {
    let snap = serde_json::to_string(&state::snapshot(&ctx.state)).expect("快照序列化失败");
    socket.send(Message::Text(snap.into())).await
}
