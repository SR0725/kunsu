//! axum HTTP 層（CONTRACT §5；bind `127.0.0.1:4777`）。
//!
//! - Bearer auth middleware：`~/.kunsu/token`；`/health` 免 auth（loopback bind
//!   本身就是外網不可達的邊界）。
//! - `GET /health`／`GET /describe`／`POST /events/kongko`／`GET /events`（WS）／
//!   `POST /approvals/verify`／`POST /mcp`。
//! - 全部 handler 委派服務層／approval／mcp 模組；本層只做協定映射。

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket};
use axum::extract::{Query, Request, State, WebSocketUpgrade};
use axum::http::{header, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::Utc;
use serde_json::{json, Value};

use crate::approval::{self, KeyProvider};
use crate::error::ServerError;
use crate::mcp::{self, McpReply};
use crate::service::{KongkoPublication, ServerHandle};

/// HTTP 層狀態（axum State）。token 由 `auth::ensure_token` 載入。
/// **兩把 token 邊界（Codex 覆檢 P1）**：
/// - `token`（agent token，`~/.kunsu/token`）：MCP 建題/讀取、/events/kongko、
///   /ingest/discord-answer、/describe、/events、/approvals/verify——全部非錢/非管理面。
/// - `admin_token`（`~/.kunsu/admin-token`，agent skill 永不引用）：
///   /cli/answer（使用者本人終端作答）與 /admin/rebuild-db（整庫重建）。
#[derive(Clone)]
pub struct AppState {
    pub handle: ServerHandle,
    pub token: String,
    pub admin_token: String,
    pub key: Arc<dyn KeyProvider>,
}

/// 組完整 Router：`/health` 免 auth；/cli/answer 與 /admin/* 掛 admin token；
/// 其餘掛 agent token。
pub fn router(state: AppState) -> Router {
    let agent = Router::new()
        .route("/describe", get(describe))
        .route("/events/kongko", post(kongko))
        .route("/events", get(events_ws))
        .route("/approvals/verify", post(approvals_verify))
        .route("/mcp", post(mcp_post))
        .route("/ingest/discord-answer", post(ingest_discord_answer))
        .layer(middleware::from_fn_with_state(state.clone(), auth_mw));
    let admin = Router::new()
        .route("/cli/answer", post(cli_answer))
        .route("/admin/rebuild-db", post(admin_rebuild_db))
        .layer(middleware::from_fn_with_state(state.clone(), admin_auth_mw));
    Router::new()
        .merge(agent)
        .merge(admin)
        .route("/health", get(health))
        .with_state(state)
}

/// 綁定 loopback port 並常駐服務（serverd 用）。
pub async fn serve(state: AppState, addr: &str) -> Result<(), std::io::Error> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("Kunsu HTTP 聽 {addr}");
    axum::serve(listener, router(state)).await
}

fn bearer_of(req: &Request) -> Option<String> {
    req.headers()
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .map(|t| t.trim().to_string())
}

async fn auth_mw(State(st): State<AppState>, req: Request, next: Next) -> Response {
    let ok = bearer_of(&req).is_some_and(|t| t == st.token);
    if ok {
        next.run(req).await
    } else {
        (
            StatusCode::UNAUTHORIZED,
            Json(json!({
                "error": "unauthorized：需要 Authorization: Bearer <token>（token 在 ~/.kunsu/token）"
            })),
        )
            .into_response()
    }
}

/// admin 面（/cli/answer、/admin/*）：只認 `~/.kunsu/admin-token`。
/// agent token 打進來一律 401——agent 永遠沒有作答面與整庫重建面。
async fn admin_auth_mw(State(st): State<AppState>, req: Request, next: Next) -> Response {
    let ok = bearer_of(&req).is_some_and(|t| t == st.admin_token);
    if ok {
        next.run(req).await
    } else {
        (
            StatusCode::UNAUTHORIZED,
            Json(json!({
                "error": "unauthorized：此路由需要 admin token（~/.kunsu/admin-token，限使用者本人 CLI）；agent token 無效"
            })),
        )
            .into_response()
    }
}

fn error_response(e: ServerError) -> Response {
    let (status, code) = match &e {
        ServerError::Reject(r) => {
            let body = json!({
                "error": e.to_string(),
                "code": r.code(),
                "malformed": r.is_malformed(),
            });
            return (StatusCode::BAD_REQUEST, Json(body)).into_response();
        }
        ServerError::QuotaExceeded { .. } => (StatusCode::TOO_MANY_REQUESTS, "quota_exceeded"),
        ServerError::NotFound(_) => (StatusCode::NOT_FOUND, "not_found"),
        ServerError::AlreadyAnswered { .. } => (StatusCode::CONFLICT, "already_answered"),
        // INV-15 inv_money_chain_ui_only：非 ui 通道作答錢題 → 400 可讀拒絕。
        ServerError::MoneyAnswerRequiresUi { .. } => {
            (StatusCode::BAD_REQUEST, "money_answer_ui_only")
        }
        ServerError::Terminal { .. } => (StatusCode::CONFLICT, "terminal"),
        ServerError::BadKongkoEvent(_) => (StatusCode::BAD_REQUEST, "bad_kongko_event"),
        ServerError::BadAnswer(_) => (StatusCode::BAD_REQUEST, "bad_answer"),
        ServerError::BadReauthorize(_) => (StatusCode::BAD_REQUEST, "bad_reauthorize"),
        ServerError::LedgerBacklog(_) => (StatusCode::CONFLICT, "ledger_backlog"),
        ServerError::RebuildRefused(_) => (StatusCode::CONFLICT, "rebuild_refused"),
        _ => (StatusCode::INTERNAL_SERVER_ERROR, "internal"),
    };
    (status, Json(json!({"error": e.to_string(), "code": code}))).into_response()
}

// ---------------------------------------------------------------------------
// handlers
// ---------------------------------------------------------------------------

/// `GET /health`（無 auth；CONTRACT §5 + P1 定案 4 oldest_pending_days）。
async fn health(State(st): State<AppState>) -> Response {
    match st.handle.health(Utc::now()).await {
        Ok(h) => Json(h).into_response(),
        Err(e) => error_response(e),
    }
}

/// `GET /describe`：tool schemas + 狀態機轉移表 + 當前 policy（machine-readable）。
async fn describe(State(st): State<AppState>) -> Response {
    Json(json!({
        "name": "kunsu",
        "version": env!("CARGO_PKG_VERSION"),
        "tools": mcp::tool_specs(),
        "state_machine": transition_table(),
        "policy": st.handle.policy(),
    }))
    .into_response()
}

/// 狀態機轉移表（CONTRACT §3 的 machine-readable 快照；行為正本在 kunsu-core）。
fn transition_table() -> Value {
    json!({
        "states": ["created", "pending", "warned", "forced", "snoozed",
                   "answered", "expired", "cancelled", "superseded", "routed"],
        "terminal": ["answered", "expired", "cancelled", "superseded", "routed"],
        "transitions": [
            {"from": "created", "event": "tick", "to": "pending", "note": "建題即推進；deadline 已過則直接 expired"},
            {"from": "pending", "event": "tick", "to": "warned", "guard": "non-internal ∧ warn_at 到", "note": "剩三分鐘警告；不受 guards 阻擋"},
            {"from": "warned", "event": "tick", "to": "forced", "guard": "force_at 到 ∧ guards 全綠", "note": "強制詢問蓋板"},
            {"from": "warned", "event": "tick", "to": "warned", "guard": "force_at 到 ∧ guard 命中", "note": "保留原週期，每秒重試"},
            {"from": "forced", "event": "tick", "to": "warned", "guard": "guard 命中", "note": "撤下蓋板但不重設週期"},
            {"from": "forced", "event": "dismiss", "to": "pending", "note": "cycle_number++，重啟完整 15 分鐘；無上限"},
            {"from": "任一非終態", "event": "answered", "to": "answered", "note": "任一通道作答"},
            {"from": "任一非終態", "event": "tick", "to": "expired", "guard": "deadline_at 到", "note": "執行 default_action；錢類永不 pick_option"},
            {"from": "任一非終態", "event": "cancel/supersede", "to": "cancelled/superseded", "note": "不刪、留軌跡"},
            {"from": "answered", "event": "delivery_complete", "to": "routed", "note": "只有已回答的題被收割後變 routed；過期、取消、被取代不變"}
        ]
    })
}

/// `POST /events/kongko`（Bearer）：發佈確認 → 結案素材題 + 自動建啟用題。
/// 同 publicationId 冪等（P1 定案 5）。
async fn kongko(State(st): State<AppState>, body: Json<Value>) -> Response {
    let ev: KongkoPublication = match serde_json::from_value(body.0) {
        Ok(ev) => ev,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": format!("kongko 事件 payload 非法：{e}"), "code": "bad_kongko_event"})),
            )
                .into_response();
        }
    };
    match st.handle.kongko_event(ev, Utc::now()).await {
        Ok(outcome) => Json(outcome).into_response(),
        Err(e) => error_response(e),
    }
}

/// `POST /approvals/verify`（Bearer）：executor.py 驗證通道。
/// 驗證失敗一律 200 + `{valid:false, reason}`（executor fail-closed 判 valid 欄位）。
async fn approvals_verify(State(st): State<AppState>, body: Json<Value>) -> Response {
    let Some(record) = body.0.get("approval_record") else {
        return Json(json!({
            "valid": false, "consumed_now": false,
            "reason": "請求體必須是 {\"approval_record\": {...}}",
        }))
        .into_response();
    };
    let outcome =
        approval::verify_and_consume(&st.handle, st.key.as_ref(), record, Utc::now()).await;
    Json(outcome).into_response()
}

/// `GET /events`（WS，Bearer）：每個 ServerEvent 一個 JSON frame；
/// `?question=<id>` 只收該題（無 question_id 的全域事件在過濾模式下略過）。
async fn events_ws(
    ws: WebSocketUpgrade,
    Query(params): Query<HashMap<String, String>>,
    State(st): State<AppState>,
) -> Response {
    let filter = params.get("question").cloned();
    ws.on_upgrade(move |socket| stream_events(socket, st, filter))
}

async fn stream_events(mut socket: WebSocket, st: AppState, filter: Option<String>) {
    let mut rx = st.handle.subscribe();
    loop {
        tokio::select! {
            ev = rx.recv() => {
                match ev {
                    Ok(ev) => {
                        let Ok(v) = serde_json::to_value(&ev) else { continue };
                        if let Some(f) = &filter {
                            let qid = v.get("question_id").and_then(Value::as_str);
                            if qid != Some(f.as_str()) {
                                continue;
                            }
                        }
                        if socket.send(Message::Text(v.to_string().into())).await.is_err() {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!("WS /events 落後 {n} 筆（訂閱端太慢），略過續傳");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => continue,
                    Some(Err(_)) => break,
                }
            }
        }
    }
}

/// `POST /cli/answer`（Bearer）：CLI 作答通道的 server 面（CONTRACT §4——
/// answer 通道之一：CLI `kunsu answer`，限使用者本人終端）。isatty 人類閘在
/// CLI 端；本路由只信 loopback + Bearer（token 檔 0600）。channel 固定 cli。
/// 注意：這不是 MCP 面——INV-7（MCP 物理上無作答方法）不受影響。
async fn cli_answer(State(st): State<AppState>, body: Json<Value>) -> Response {
    #[derive(serde::Deserialize)]
    struct CliAnswer {
        question_id: String,
        #[serde(default)]
        choice_id: Option<String>,
        #[serde(default)]
        free_text: Option<String>,
        #[serde(default)]
        edited_params: Option<String>,
    }
    let req: CliAnswer = match serde_json::from_value(body.0) {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": format!("cli answer payload 非法：{e}"), "code": "bad_payload"})),
            )
                .into_response();
        }
    };
    if req.choice_id.is_none() && req.free_text.is_none() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "choice_id 與 free_text 至少要有一個", "code": "bad_payload"})),
        )
            .into_response();
    }
    let input = crate::service::AnswerInput {
        choice_id: req.choice_id,
        free_text: req.free_text,
        edited_params: req.edited_params,
        channel: kunsu_core::types::Channel::Cli,
        signature: None,
        revision: None,
        content_hash: None,
        items: None,
    };
    match st.handle.answer_question(&req.question_id, input, Utc::now()).await {
        Ok(q) => Json(json!({"question": q})).into_response(),
        Err(e) => error_response(e),
    }
}

/// `POST /ingest/discord-answer`（Bearer；CONTRACT §6 規則 12）：Discord 讀回
/// 的落地面（agent 側 SOP 讀到使用者的 Discord 回覆後打進來）。channel 固定
/// discord；錢題被服務層通道白名單拒絕（規則 6 / INV-15，400）；紅燈題允許
/// ——只落 ledger 不執行（執行鎖 ≠ 作答鎖，reactor 一鍵判準本就擋 red 與非 ui）。
async fn ingest_discord_answer(State(st): State<AppState>, body: Json<Value>) -> Response {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct DiscordAnswer {
        question_id: String,
        #[serde(default)]
        choice_id: Option<String>,
        #[serde(default)]
        free_text: Option<String>,
        discord_message_id: String,
    }
    let req: DiscordAnswer = match serde_json::from_value(body.0) {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "error": format!("discord-answer payload 非法：{e}"),
                    "code": "bad_payload",
                })),
            )
                .into_response();
        }
    };
    if req.choice_id.is_none() && req.free_text.is_none() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "choice_id 與 free_text 至少要有一個", "code": "bad_payload"})),
        )
            .into_response();
    }
    if req.discord_message_id.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "discord_message_id 不可為空（回覆溯源鍵）", "code": "bad_payload"})),
        )
            .into_response();
    }
    let now = Utc::now();
    let input = crate::service::AnswerInput {
        choice_id: req.choice_id,
        free_text: req.free_text,
        edited_params: None,
        channel: kunsu_core::types::Channel::Discord,
        signature: None,
        revision: None,
        content_hash: None,
        items: None,
    };
    match st.handle.answer_question(&req.question_id, input, now).await {
        Ok(q) => {
            // Discord 訊息溯源留痕（事件行；ledger 的 answered 行已由服務層落）。
            if let Err(e) = crate::store::insert_event(
                st.handle.pool(),
                Some(&q.id),
                "discord_answer",
                &json!({
                    "question_id": q.id,
                    "discord_message_id": req.discord_message_id,
                }),
                now,
            )
            .await
            {
                tracing::warn!("discord_answer 事件留痕失敗（作答已成立）：{e}");
            }
            Json(json!({"question": q})).into_response()
        }
        Err(e) => error_response(e),
    }
}

/// `POST /admin/rebuild-db`（Bearer）：`kunsu rebuild-db` 在 server 活著時的
/// 通道（避免 CLI 與活體 server 搶同一顆 SQLite）。server 死時 CLI 自己本地重建。
async fn admin_rebuild_db(State(st): State<AppState>) -> Response {
    match st.handle.rebuild_db().await {
        Ok(report) => Json(json!({"report": report})).into_response(),
        Err(e) => error_response(e),
    }
}

/// `POST /mcp`：streamable HTTP MCP（純 JSON body；不開 SSE 也能回應）。
/// notification（無 id）回 202；initialize 回應帶 `Mcp-Session-Id` header。
async fn mcp_post(State(st): State<AppState>, body: Json<Value>) -> Response {
    let is_initialize =
        body.0.get("method").and_then(Value::as_str) == Some("initialize");
    match mcp::handle_rpc(&st.handle, body.0).await {
        McpReply::Accepted => StatusCode::ACCEPTED.into_response(),
        McpReply::Json(v) => {
            let mut resp = Json(v).into_response();
            if is_initialize {
                let sid = uuid::Uuid::new_v4().to_string();
                if let Ok(hv) = header::HeaderValue::from_str(&sid) {
                    resp.headers_mut().insert("Mcp-Session-Id", hv);
                }
            }
            resp
        }
    }
}
