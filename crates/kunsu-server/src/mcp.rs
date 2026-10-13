//! `/mcp` streamable HTTP MCP server（CONTRACT §0/§4）。
//!
//! 手寫 JSON-RPC 2.0（依賴少、可控）：`initialize` → `notifications/initialized`
//! （202）→ `tools/list`（14 tools）→ `tools/call`。回應一律純 JSON body
//! （client 可帶 `Accept: text/event-stream`，本 server 不開 SSE 也必須能回應）。
//!
//! **MCP 面物理上沒有作答方法**（INV-7 inv_no_answer_via_mcp）：tools 只有
//! 建/讀/等/收割/補充/撤回/取代——answer 只能來自 Tauri IPC、`/events/kongko`、
//! Discord 讀回、CLI isatty 通道。

use chrono::Utc;
use kunsu_core::types::CreateRequest;
use serde_json::{json, Value};

use crate::error::ServerError;
use crate::service::{ServerHandle, UpdatePatch};

/// `ask_wait` 上限（CONTRACT §4 / P1 定案 9：>540 拒收，避開 harness tool timeout）。
pub const ASK_WAIT_MAX_TIMEOUT_S: u64 = 540;
/// 本 server 支援的 MCP 協定版本（client 帶其他值時回報本值）。
pub const PROTOCOL_VERSION: &str = "2025-03-26";

/// JSON-RPC 處理結果：Json 直接回 200；Accepted 是 notification 的 202 空體。
pub enum McpReply {
    Json(Value),
    Accepted,
}

struct RpcError {
    code: i64,
    message: String,
}

impl RpcError {
    fn invalid_params(msg: impl Into<String>) -> Self {
        RpcError { code: -32602, message: msg.into() }
    }

    fn server(msg: impl Into<String>) -> Self {
        RpcError { code: -32000, message: msg.into() }
    }
}

impl From<ServerError> for RpcError {
    fn from(e: ServerError) -> Self {
        RpcError::server(e.to_string())
    }
}

/// 批次遷移腳本等 client 用 `requested_tier` 稱呼 agent 原始請求值；
/// `CreateRequest` 的收單鍵是 `tier`（server 守門後寫入 requested_tier/tier 兩欄）。
/// 這裡把 `requested_tier` 正規化成 `tier`（兩者都給時以 `tier` 為準）。
/// 檔案 inbox 與 ask_create 完全同格式（P1 定案 6），共用本函數。
pub fn normalize_create_args(mut args: Value) -> Value {
    if let Some(obj) = args.as_object_mut() {
        if obj.contains_key("tier") {
            obj.remove("requested_tier");
        } else if let Some(rt) = obj.remove("requested_tier") {
            obj.insert("tier".to_string(), rt);
        }
    }
    args
}

/// 單一入口：整包 JSON-RPC body → 回應。notification（無 id）一律 202。
pub async fn handle_rpc(handle: &ServerHandle, body: Value) -> McpReply {
    let Some(id) = body.get("id").cloned() else {
        // notifications/initialized 等 notification：202 收下即可。
        return McpReply::Accepted;
    };
    let method = body.get("method").and_then(Value::as_str).unwrap_or_default();
    let params = body.get("params").cloned().unwrap_or_else(|| json!({}));

    let outcome: Result<Value, RpcError> = match method {
        "initialize" => Ok(initialize_result(&params)),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tool_specs() })),
        "tools/call" => tools_call(handle, &params).await,
        other => Err(RpcError {
            code: -32601,
            message: format!("method {other:?} 不存在"),
        }),
    };

    McpReply::Json(match outcome {
        Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
        Err(e) => json!({
            "jsonrpc": "2.0", "id": id,
            "error": {"code": e.code, "message": e.message},
        }),
    })
}

fn initialize_result(params: &Value) -> Value {
    // 協定版本協商：client 要的版本若非本 server 支援值，回報本 server 支援值。
    let requested = params.get("protocolVersion").and_then(Value::as_str);
    let version = match requested {
        Some(v) if v == PROTOCOL_VERSION => v,
        _ => PROTOCOL_VERSION,
    };
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": { "listChanged": false } },
        "serverInfo": {
            "name": "kunsu",
            "version": env!("CARGO_PKG_VERSION"),
        },
    })
}

async fn tools_call(handle: &ServerHandle, params: &Value) -> Result<Value, RpcError> {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| RpcError::invalid_params("tools/call 缺 name"))?;
    let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
    let now = Utc::now();

    let payload: Value = match name {
        "ask_create" => {
            // 錯誤訊息帶欄位路徑（例如 source.expected_every_hours），agent 才知道要改哪裡。
            let req: CreateRequest = serde_path_to_error::deserialize(normalize_create_args(args))
                .map_err(|e| RpcError::invalid_params(format!("ask_create 參數非法：{}：{}", e.path(), e.inner())))?;
            let mut outcome = handle.create_question(req, now).await?;
            // 重送已答題回傳的既有答案也要去敏（不含簽章與錢鏈原始輸出）。
            if outcome.answer.is_some()
                && let Some(q) = handle.get_question(&outcome.question_id).await?
            {
                outcome.answer = agent_view_of(handle, &q).await?.answer;
            }
            serde_json::to_value(outcome).map_err(|e| RpcError::server(e.to_string()))?
        }
        "ask_wait" => ask_wait(handle, &args).await?,
        "ask_followup_reply" => {
            let qid = require_str(&args, "question_id")?;
            let fid = require_str(&args, "followup_id")?;
            let text = require_str(&args, "text")?;
            handle.followup_reply(&qid, &fid, &text).map_err(|e| RpcError::invalid_params(e.to_string()))?;
            json!({"ok": true, "note": "已轉給使用者的手機。題目還在等正式答案，請繼續 ask_wait。"})
        }
        "ask_register" => {
            let dir = require_str(&args, "draft_dir")?;
            let dry_run = match args.get("dry_run") {
                None | Some(Value::Null) => false,
                Some(Value::Bool(b)) => *b,
                Some(other) => {
                    return Err(RpcError::invalid_params(format!("dry_run 必須是 true／false（收到 {other}），沒有註冊")));
                }
            };
            let opts = crate::draft::RegisterOptions {
                dry_run,
                source: args.get("source").filter(|v| v.is_object()).cloned(),
            };
            let mut outcome = handle.register_draft(&crate::draft::expand_home(&dir), opts, now).await?;
            if outcome.answer.is_some()
                && let Some(qid) = outcome.question_id.clone()
                && let Some(q) = handle.get_question(&qid).await?
            {
                outcome.answer = agent_view_of(handle, &q).await?.answer;
            }
            serde_json::to_value(outcome).map_err(|e| RpcError::server(e.to_string()))?
        }
        "ask_report" => {
            let qid = require_str(&args, "question_id")?;
            let answer_id = require_str(&args, "answer_id")?;
            let status: kunsu_core::continuity::ReportStatus = serde_json::from_value(
                args.get("status").cloned().unwrap_or(Value::Null),
            )
            .map_err(|_| RpcError::invalid_params("status 必須是 done｜failed｜blocked"))?;
            let summary = require_str(&args, "summary_md")?;
            let q = handle.report_answer(&qid, &answer_id, status, &summary, now).await?;
            let delivery = q.answer.as_ref().and_then(|a| a.delivery.clone());
            json!({ "question_id": q.id, "recorded": true, "delivery": delivery })
        }
        "ask_answers_since" => {
            let consumer = require_str(&args, "consumer")?;
            let cursor = args.get("cursor").and_then(Value::as_i64);
            let batch = handle.answers_since(&consumer, cursor, now).await?;
            serde_json::to_value(batch).map_err(|e| RpcError::server(e.to_string()))?
        }
        "ask_list" => {
            let include_terminal =
                args.get("include_terminal").and_then(Value::as_bool).unwrap_or(false);
            let mut qs = Vec::new();
            for q in handle.list_questions(include_terminal).await? {
                qs.push(agent_view_of(handle, &q).await?);
            }
            json!({ "questions": qs })
        }
        "ask_get" => {
            let qid = require_str(&args, "question_id")?;
            let q = handle
                .get_question(&qid)
                .await?
                .ok_or_else(|| RpcError::server(format!("題目 {qid} 不存在")))?;
            json!({ "question": agent_view_of(handle, &q).await? })
        }
        "ask_update" => {
            let qid = require_str(&args, "question_id")?;
            // UpdatePatch 開 deny_unknown_fields（CONTRACT §6 規則 11）：
            // question_id 是路由鍵不屬 patch，先剝掉再嚴格解析。
            let mut patch_args = args.clone();
            if let Some(obj) = patch_args.as_object_mut() {
                obj.remove("question_id");
            }
            let patch: UpdatePatch = serde_json::from_value(patch_args).map_err(|e| {
                RpcError::invalid_params(format!(
                    "ask_update 參數非法（未知欄位一律拒收，不靜默忽略；作答請走 UI/授權通道）：{e}"
                ))
            })?;
            let q = handle.update_question_meta(&qid, patch, now).await?;
            json!({ "question_id": q.id, "state": q.state })
        }
        "ask_cancel" => {
            let qid = require_str(&args, "question_id")?;
            let q = handle.cancel_question(&qid, now).await?;
            json!({ "question_id": q.id, "state": q.state })
        }
        "ask_supersede" => {
            let qid = require_str(&args, "question_id")?;
            let q = handle.supersede_question(&qid, now).await?;
            json!({ "question_id": q.id, "state": q.state })
        }
        "dnd_get" => serde_json::to_value(handle.dnd_status(now).await?)
            .map_err(|e| RpcError::server(e.to_string()))?,
        "dnd_set" => {
            let raw = args
                .get("duration_minutes")
                .and_then(Value::as_u64)
                .ok_or_else(|| RpcError::invalid_params("缺必填整數參數 duration_minutes"))?;
            let minutes = u32::try_from(raw)
                .map_err(|_| RpcError::invalid_params("duration_minutes 超出整數範圍"))?;
            let actor = args.get("actor").and_then(Value::as_str).unwrap_or("agent");
            serde_json::to_value(handle.set_dnd(minutes, actor, now).await?)
                .map_err(|e| RpcError::server(e.to_string()))?
        }
        "dnd_clear" => {
            let actor = args.get("actor").and_then(Value::as_str).unwrap_or("agent");
            serde_json::to_value(handle.clear_dnd(actor, now).await?)
                .map_err(|e| RpcError::server(e.to_string()))?
        }
        // INV-7：作答類方法在 MCP 面物理上不存在——一律 unknown tool。
        other => {
            return Err(RpcError::invalid_params(format!(
                "unknown tool {other:?}：MCP 面只有 10 個 ask_* 與 3 個 dnd_* 工具，且不存在任何作答/批准方法"
            )));
        }
    };

    Ok(json!({
        "content": [{ "type": "text", "text": payload.to_string() }],
        "structuredContent": payload,
        "isError": false,
    }))
}

fn require_str(args: &Value, key: &str) -> Result<String, RpcError> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(str::to_string)
        .ok_or_else(|| RpcError::invalid_params(format!("缺必填字串參數 {key}")))
}

/// long-poll：訂閱 broadcast + timeout。無伺服器端 session 狀態，超時回
/// `pending_timeout`，agent 直接再呼叫續等（harness-roundtrip 場景 1–2）。
async fn ask_wait(handle: &ServerHandle, args: &Value) -> Result<Value, RpcError> {
    let qid = require_str(args, "question_id")?;
    let timeout_s = args
        .get("timeout_seconds")
        .and_then(Value::as_u64)
        .ok_or_else(|| RpcError::invalid_params("缺必填整數參數 timeout_seconds"))?;
    if timeout_s == 0 || timeout_s > ASK_WAIT_MAX_TIMEOUT_S {
        return Err(RpcError::invalid_params(format!(
            "timeout_seconds 必須在 1..={ASK_WAIT_MAX_TIMEOUT_S}（收到 {timeout_s}）——超過 harness tool timeout 的等待請迴圈續等"
        )));
    }
    // 等待者登記（回答延續原對話：在線或 10 秒租約內，reactor 不 resume／不開新對話）。
    // guard 在 drop 時離場：連線被切斷、future 被取消也一定釋放，不會永遠算在線。
    let _guard = WaiterGuard::enter(handle, &qid);
    let mut value = ask_wait_inner(handle, &qid, timeout_s).await?;
    if value.get("status").and_then(Value::as_str) != Some("answered") {
        return Ok(value);
    }
    let Some(q) = handle.get_question(&qid).await? else {
        return Ok(value);
    };
    // 交給等待中的原對話：認領交付（同一答案只有一條路徑；已定案則不變）。
    // 沒有原對話身分的題維持現行行為，等待者不影響 reactor 的 wake。
    if let Some(answer) = q.answer.as_ref()
        && kunsu_core::continuity::continuation_applies(&q.source, &q.wake_on_answer, q.internal)
    {
        handle
            .set_delivery(
                &qid,
                &answer.answer_id,
                kunsu_core::continuity::DeliveryState::LiveWaiter,
                None,
                None,
                Utc::now(),
            )
            .await?;
        // 回傳最終交付去向：遲到的等待者才知道答案已在哪裡被處理。
        let delivery = handle.get_question(&qid).await?.and_then(|q| q.answer).and_then(|a| a.delivery);
        if let Some(obj) = value.as_object_mut() {
            obj.insert("delivery".to_string(), json!(delivery));
            if let Some(a) = obj.get_mut("answer").and_then(Value::as_object_mut) {
                a.insert("delivery".to_string(), json!(delivery));
            }
        }
    }
    Ok(value)
}

/// 等待者租約的 RAII：建立時登記、drop 時離場（含取消與斷線）。
struct WaiterGuard<'a> {
    handle: &'a ServerHandle,
    qid: String,
}

impl<'a> WaiterGuard<'a> {
    fn enter(handle: &'a ServerHandle, qid: &str) -> Self {
        handle.waiter_enter(qid, Utc::now());
        Self { handle, qid: qid.to_string() }
    }
}

impl Drop for WaiterGuard<'_> {
    fn drop(&mut self) {
        self.handle.waiter_exit(&self.qid, Utc::now());
    }
}

/// agent 讀取面（ask_get／ask_list／ask_wait）：不回 UI 簽章（只給 Lane 1 drain 回寫 pending-approvals，
/// 走 ask_answers_since）；錢題的錢鏈原始輸出（approval 路徑、token）一律換成人話狀態，不論 agent 是否已回報
/// （INV-25）。agent 自己的回饋在 `answer.report`，照原樣。
/// agent 讀取面的去敏版本，連同「錢的結果不明」（以 nonce 判斷，CONTRACT §7）一起算。
async fn agent_view_of(handle: &ServerHandle, q: &kunsu_core::types::Question) -> Result<kunsu_core::types::Question, RpcError> {
    let (running, unknown) = money_flags(handle, q).await?;
    Ok(agent_view(q, running, unknown))
}

/// 錢鏈還在跑（錢鏈列 queued／executing、這一輪的結果還沒寫入）與結果不明（不在跑、有結果不明的授權）——在跑的空檔不是結果不明；
/// 錢鏈跑完、同一列還在交付時照錢鏈結果說，跟追加訊息一致（CONTRACT §4、§7）。
async fn money_flags(handle: &ServerHandle, q: &kunsu_core::types::Question) -> Result<(bool, bool), RpcError> {
    let running = handle.money_result_pending(q).await?;
    let unknown = !running && handle.money_outcome_unknown(q).await?;
    Ok((running, unknown))
}

fn agent_view(q: &kunsu_core::types::Question, money_running: bool, money_unknown: bool) -> kunsu_core::types::Question {
    let money = q.is_money_op();
    let mut q = q.clone();
    let applies = q.answer.as_ref().is_some_and(|a| crate::reactor::money_chain_applies_to(&q, a));
    if let Some(a) = q.answer.as_mut() {
        a.signature = None;
        if money {
            a.execution_result = match a.execution_result.as_deref() {
                // 錢鏈還在跑（錢鏈列 queued／executing）：一律「錢鏈還在跑」，即使 executor 已驗證、commit 還沒回報（CONTRACT §4）。
                _ if money_running => Some(crate::reactor::MONEY_PENDING.to_string()),
                // 結果不明：executor 已用掉某一次授權卻沒有結果——不論紀錄寫什麼都先說結果不明（CONTRACT §7）。
                _ if money_unknown => Some(crate::reactor::money_status(Some("EXEC_UNKNOWN")).to_string()),
                // 錢鏈不適用（安全選項、紅燈、非 UI 通道）：不論交付是否失敗，錢都沒動——一律「未觸發」（AC-CC-14）。
                _ if !applies => Some(crate::reactor::money_status(None).to_string()),
                // 錢鏈適用但還沒有結果：不能讓 agent 以為錢已經動了或沒動。
                None => Some(crate::reactor::MONEY_PENDING.to_string()),
                raw => Some(crate::reactor::money_status(raw).to_string()),
            };
        }
    }
    q
}

async fn ask_wait_inner(handle: &ServerHandle, qid: &str, timeout_s: u64) -> Result<Value, RpcError> {
    let qid = qid.to_string();
    // 先訂閱再讀狀態：作答發生在兩步之間也不會漏 wake。
    let mut rx = handle.subscribe();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(timeout_s);
    loop {
        let q = handle
            .get_question(&qid)
            .await?
            .ok_or_else(|| RpcError::server(format!("題目 {qid} 不存在")))?;
        // 錢題：錢鏈（reactor 列）跑完才交付，等待者拿到的答案已帶錢鏈的人話狀態（AC-CC-14）。
        let money_running = match q.answer.as_ref() {
            Some(a) if q.is_money_op() => handle.execution_pending(&a.answer_id).await?,
            _ => false,
        };
        if q.state.is_terminal() && !money_running {
            let (running, unknown) = money_flags(handle, &q).await?;
            return Ok(wait_result(&q, running, unknown));
        }
        // 使用者在手機上「問本人」：先把追問交給等待中的原 AI（每則只交一次；不是答案，INV-31）。
        if !q.state.is_terminal()
            && let Some((id, text)) = handle.followup_take(&qid)
        {
            return Ok(json!({
                "status": "followup",
                "followup": {"id": id, "text": text},
                "hint": format!(
                    "使用者在手機上追問你（不是答案，題目還在等）。用 ask_followup_reply {{question_id: \"{qid}\", followup_id: \"{id}\", text}} 回答，回完再呼叫 ask_wait 繼續等。"
                ),
            }));
        }
        tokio::select! {
            _ = tokio::time::sleep_until(deadline) => {
                return Ok(json!({ "status": "pending_timeout" }));
            }
            ev = rx.recv() => {
                match ev {
                    Ok(_) => continue, // 任一事件都重查一次（流量低、查詢便宜）
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        // 廣播關閉（server 收尾）：退化為輪詢直到 deadline。
                        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                        if tokio::time::Instant::now() >= deadline {
                            return Ok(json!({ "status": "pending_timeout" }));
                        }
                        continue;
                    }
                }
            }
        }
    }
}

fn wait_result(q: &kunsu_core::types::Question, money_running: bool, money_unknown: bool) -> Value {
    use kunsu_core::types::State;
    // routed（投遞完成）沿用終態語義：有 answer 視為 answered，其餘視為 expired。
    let status = match (q.state, q.answer.is_some()) {
        (State::Answered, _) | (State::Routed, true) => "answered",
        (State::Cancelled, _) | (State::Superseded, _) => "cancelled",
        _ => "expired",
    };
    let mut out = match &agent_view(q, money_running, money_unknown).answer {
        Some(a) => json!({ "status": status, "answer": a }),
        None => json!({ "status": status }),
    };
    // 使用者按「這題不該問我」退回：等待中的 agent 要知道是使用者推掉與原因（AC-AN-06）。
    if let Some(r) = &q.rejected {
        out["rejected"] = json!(r);
    }
    out
}

// ---------------------------------------------------------------------------
// tool schemas（CONTRACT §4：schema 依 §2；/describe 共用）
// ---------------------------------------------------------------------------

fn evidence_schema() -> Value {
    json!({
        "type": "array", "maxItems": 3,
        "description": "證據 ≤3 條；source 是身世標籤，必填",
        "items": {
            "type": "object",
            "properties": {
                "text": {"type": "string"},
                "source": {"type": "string", "description": "身世標籤（如 meta-api / stripe / kongko-webhook）"}
            },
            "required": ["text", "source"]
        }
    })
}

/// 建題來源（含原對話與排程身分，PRD §20/§21）。拆出來避免 json! 巨集遞迴過深。
fn source_schema() -> Value {
    json!({
                "type": "object",
                "properties": {
                    "agent": {"type": "string", "enum": ["claude", "codex", "other"]},
                    "session_id": {"type": ["string", "null"]},
                    "cwd": {"type": "string"},
                    "routine_id": {"type": ["string", "null"]},
                    "host": {"type": ["string", "null"], "enum": ["claude_desktop", "claude_cli", "codex_app", "codex_cli", "other", null], "description": "發問主機；`kunsu ask` 會依環境自動補"},
                    "conversation_id": {"type": ["string", "null"], "description": "原對話 id：Claude 用 $CLAUDE_CODE_SESSION_ID，Codex 用 $CODEX_THREAD_ID。帶了就會把答案送回這段對話"},
                    "host_session_id": {"type": ["string", "null"], "description": "Claude 桌面 session（$CLAUDE_CODE_HOST_SESSION_ID，local_…）"},
                    "attended": {"type": ["boolean", "null"]},
                    "cli_path": {"type": ["string", "null"], "description": "$CLAUDE_CODE_EXECPATH"},
                    "trigger": {"type": ["string", "null"], "enum": ["scheduled", "interactive", null]},
                    "routine_name": {"type": ["string", "null"]},
                    "routine_kind": {"type": ["string", "null"], "enum": ["claude_scheduled_task", "codex_cron", "codex_heartbeat", "other", null]},
                    "expected_every_hours": {"type": ["number", "null"], "description": "排程宣告的報到頻率（小時）"}
                },
                "required": ["agent", "cwd"]
            })
}

/// 題組項目（CONTRACT §14）。拆出來避免 json! 巨集遞迴過深。
fn items_schema() -> Value {
    json!({
        "type": "array",
        "minItems": 1,
        "maxItems": 20,
        "description": "kind=batch（題組）才帶：1–20 個同類項目，使用者逐項採用／不處理／請 AI 改。不能放動錢的項目；題組不能帶 kongko_ref；題組 id 最多 116 字（留給 -r2 輪次後綴）",
        "items": {
            "type": "object",
            "properties": {
                "id": {"type": "string", "description": "題內唯一；只能用英數、.、_、-"},
                "title": {"type": "string", "maxLength": 80, "description": "一眼認出是哪一項（寄件人＋主旨、留言者）"},
                "subtitle": {"type": ["string", "null"]},
                "context_md": {"type": "string", "description": "原文（來信全文、留言串、申請內容），Markdown"},
                "proposal": {"type": "string", "description": "你的草稿或建議；editable 時 ≤5,000 字，使用者採用的文字要一字不改送出。建議不處理（default: skip）的可編輯項目給空字串或真的回覆，不准寫「（不回）」之類的佔位文字"},
                "editable": {"type": "boolean", "description": "true＝使用者可改草稿；false＝只核可（不能改文字：照建議、不處理或請 AI 改）"},
                "default": {"type": "string", "enum": ["approve", "skip"], "description": "預設 approve；建議不處理的填 skip 並寫 default_reason"},
                "default_reason": {"type": ["string", "null"]}
            },
            "required": ["id", "title", "context_md", "proposal", "editable"],
            "additionalProperties": false
        }
    })
}

fn question_ref_arg() -> Value {
    json!({
        "type": "object",
        "properties": {
            "question_id": {"type": "string", "description": "題目 id（pa-* 穩定 dedup id）"}
        },
        "required": ["question_id"]
    })
}

/// 13 個 tool 的 JSON Schema（CONTRACT §4）。tools/list 與 /describe 共用。
pub fn tool_specs() -> Vec<Value> {
    let tier_enum = json!({"type": "string", "enum": ["t0", "t1", "t2", "t3"]});
    let create_schema = json!({
        "type": "object",
        "properties": {
            "id": {"type": "string", "description": "穩定 dedup id（沿用 pa-*；同 id upsert 只 touch last_seen/證據）"},
            "title": {"type": "string", "maxLength": 80},
            "question_line": {"type": "string", "maxLength": 24, "description": "≤24 字決策句（蓋板大字）"},
            "body_md": {"type": "string", "description": "證據內文（Markdown／safe HTML；表格、圖片）"},
            "why_md": {"type": ["string", "null"], "description": "為什麼問你：一段摘要＋以 `HH:MM` 或 `現在` 開頭的清單（自動變時間軸）"},
            "kind": {"type": "string", "enum": ["choice", "text", "confirm", "kongko_review", "batch"], "description": "batch＝題組（一批同類項目逐項決定，帶 items）"},
            "tier": tier_enum,
            "requested_tier": {"type": "string", "enum": ["t0", "t1", "t2", "t3"], "description": "tier 的別名（migrate/backlog client 用法）；與 tier 擇一，兩者都給以 tier 為準"},
            "product": {"type": ["string", "null"], "description": "使用者自訂的產品代號（例如 course-a、demo-shop）；account＝帳戶層級"},
            "gate": {"type": ["string", "null"], "enum": ["yellow", "red", null]},
            "evidence": evidence_schema(),
            "params_line": {"type": ["string", "null"], "description": "動錢題必填：精確變更一行（所見即所簽）"},
            "options": {
                "type": "array",
                "description": "choice 題 2..=4 個；recommended=true 標執行型選項，至少留一個非執行型安全預設",
                "items": {
                    "type": "object",
                    "properties": {
                        "id": {"type": "string"},
                        "label": {"type": "string"},
                        "detail": {"type": ["string", "null"]},
                        "recommended": {"type": ["boolean", "null"]}
                    },
                    "required": ["id", "label"]
                }
            },
            "allow_free_text": {"type": "boolean", "description": "恆 true"},
            "money": {
                "type": ["object", "null"],
                "properties": {
                    "is_money_op": {"type": "boolean"},
                    "amount_twd": {"type": ["integer", "null"]},
                    "entity_id": {"type": ["string", "null"]},
                    "executor_prepare_cmd": {"type": ["string", "null"], "description": "完整 prepare 命令字串；建題時不執行，使用者點批准當下才 spawn"}
                },
                "required": ["is_money_op"]
            },
            "decision_ref": {"type": ["string", "null"]},
            "prediction_ref": {"type": ["string", "null"], "description": "錢類題必填（inv_money_requires_prediction）"},
            "kongko_ref": {"type": ["string", "null"], "description": "batchId；發佈事件自動結案鍵"},
            "deep_link": {"type": ["string", "null"]},
            "irreversible": {"type": "boolean", "description": "不動錢但不可逆時標 true（T0 准入判準之一）"},
            "standing_policy_ref": {"type": ["string", "null"]},
            "internal": {"type": "boolean", "description": "doctor 測試題標記：不佔 T0 預算、不推 Discord、reactor 只 dry-run"},
            "default_action": {
                "type": "object",
                "properties": {
                    "on_expire": {"type": "string", "enum": ["noop_log", "escalate_discord", "pick_option"], "description": "錢類題永不允許 pick_option"},
                    "option_id": {"type": ["string", "null"]}
                },
                "required": ["on_expire"]
            },
            "deadline_at": {"type": ["string", "null"], "format": "date-time"},
            "wake_on_answer": {
                "type": "object",
                "properties": {
                    "mode": {"type": "string", "enum": ["none", "resume", "fresh", "continue"], "description": "有 source.conversation_id 時 server 一律改為 continue（回答延續原對話）"},
                    "session_id": {"type": ["string", "null"]},
                    "playbook": {"type": ["string", "null"], "description": "vault 相對路徑（如 playbooks/approved-money-op.md）"}
                },
                "required": ["mode"]
            },
            "source": source_schema(),
            "first_proposed": {"type": ["string", "null"], "format": "date", "description": "積壓天數誠實原則：client 可自帶；未提供 server 填今日（台北）"},
            "items": items_schema()
        },
        "required": ["id", "title", "question_line", "kind", "default_action", "source"],
        "additionalProperties": false
    });

    vec![
        json!({
            "name": "ask_create",
            "description": "建題（同 id upsert 去重：只 touch last_seen/證據，不重置倒數不發通知；已終態題回既有答案）。per-source 每日配額預設 12。一批同類工作（回信、留言回覆、逐項核可）用 kind=batch＋items 建一題題組，不要拆成多題。",
            "inputSchema": create_schema
        }),
        json!({
            "name": "ask_register",
            "description": "註冊草稿題（PRD §24，Claude Artifact 模式）：draft_dir 是 ~/.kunsu/drafts/<id>/，內含 question.md（YAML frontmatter＋Markdown 證據）、可選 canvas.tsx|jsx|html 與 assets/。Kunsu 檢查欄位、Markdown、畫布並真的渲染一次：ok=false 時 issues 逐條列出 file／line／column／code／message／hint，修正草稿後再註冊；ok=true 回 question_id、revision 與 preview_png（使用者會看到的畫面截圖）。dry_run=true 只檢查與預覽、不建題。同 id 重新註冊＝升版，不重設倒數。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "draft_dir": {"type": "string", "description": "草稿資料夾（~/.kunsu/drafts/<題目 id>）"},
                    "dry_run": {"type": "boolean"},
                    "source": source_schema()
                },
                "required": ["draft_dir"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "ask_report",
            "description": "原對話處理完答案後回報結果與給使用者的回饋（AI 回饋）。只記錄執行結果，永遠不能作答或改答案；同內容重送冪等。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "question_id": {"type": "string"},
                    "answer_id": {"type": "string", "description": "ask_get／kunsu wait 回傳的 answer.answer_id"},
                    "status": {"type": "string", "enum": ["done", "failed", "blocked"]},
                    "summary_md": {"type": "string", "description": "給使用者看的結果與回饋（Markdown，≤20,000 字）"}
                },
                "required": ["question_id", "answer_id", "status", "summary_md"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "ask_wait",
            "description": "long-poll 等待單題進終態。timeout_seconds ≤ 540；超時回 pending_timeout，無伺服器端狀態可直接迴圈續等。使用者在手機追問時回 status=followup（帶 followup.id／text）：用 ask_followup_reply 回答後再續等。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "question_id": {"type": "string"},
                    "timeout_seconds": {"type": "integer", "minimum": 1, "maximum": 540}
                },
                "required": ["question_id", "timeout_seconds"]
            }
        }),
        json!({
            "name": "ask_followup_reply",
            "description": "回答使用者在手機上對這題的追問（ask_wait 回 status=followup 時用）。只把文字轉給使用者，不是答案、不改題目或倒數；回完繼續 ask_wait。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "question_id": {"type": "string"},
                    "followup_id": {"type": "string", "description": "ask_wait 回傳的 followup.id"},
                    "text": {"type": "string", "description": "給使用者的回覆（≤4,000 字）"}
                },
                "required": ["question_id", "followup_id", "text"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "ask_answers_since",
            "description": "per-consumer 游標收割終態事件（at-least-once：帶 cursor 即 ack 其之前；同 cursor 重呼叫重收同批）。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "consumer": {"type": "string"},
                    "cursor": {"type": ["integer", "null"]}
                },
                "required": ["consumer"]
            }
        }),
        json!({
            "name": "ask_list",
            "description": "列出題目（預設只回非終態；include_terminal=true 含終態）。",
            "inputSchema": {
                "type": "object",
                "properties": { "include_terminal": {"type": "boolean"} }
            }
        }),
        json!({
            "name": "ask_get",
            "description": "讀單題全量快照（含答案與狀態）。",
            "inputSchema": question_ref_arg()
        }),
        json!({
            "name": "ask_update",
            "description": "更新非終態題的補充/文案欄位（evidence / body_md / deep_link / title / question_line / options / wake_on_answer）；不重置倒數、不動狀態與 tier；錢題 options 拒絕（走 edit_option 紅線路徑）、作答仍不在此面。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "question_id": {"type": "string"},
                    "evidence": evidence_schema(),
                    "body_md": {"type": ["string", "null"]},
                    "deep_link": {"type": ["string", "null"]},
                    "title": {"type": ["string", "null"], "description": "白話決策句 ≤80 字"},
                    "question_line": {"type": ["string", "null"], "description": "蓋板大字問句 ≤24 字"},
                    "options": {"type": ["array", "null"], "description": "選項文案重寫（非錢題限定；choice 題 2-4 個）"},
                    "wake_on_answer": {"type": ["object", "null"], "description": "答後喚醒設定 {mode: none|resume|fresh, playbook?}"}
                },
                "required": ["question_id"]
            }
        }),
        json!({
            "name": "ask_cancel",
            "description": "agent 撤回題目（terminal=cancelled；不刪、留軌跡）。",
            "inputSchema": question_ref_arg()
        }),
        json!({
            "name": "ask_supersede",
            "description": "agent 以新題取代舊題（terminal=superseded；不刪、留軌跡）。",
            "inputSchema": question_ref_arg()
        }),
        json!({
            "name": "dnd_get",
            "description": "讀取 Kunsu 勿擾狀態；不改任何題目或授權。",
            "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false}
        }),
        json!({
            "name": "dnd_set",
            "description": "設定最長 24 小時勿擾；期間 notification/capsule/overlay 靜默，題目與倒數保留。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "duration_minutes": {"type": "integer", "minimum": 1, "maximum": 1440},
                    "actor": {"type": "string", "minLength": 1, "maxLength": 80}
                },
                "required": ["duration_minutes"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "dnd_clear",
            "description": "提前解除 Kunsu 勿擾；冪等，不回答或取消任何題目。",
            "inputSchema": {
                "type": "object",
                "properties": {"actor": {"type": "string", "minLength": 1, "maxLength": 80}},
                "additionalProperties": false
            }
        }),
    ]
}
