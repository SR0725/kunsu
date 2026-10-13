//! `kunsu` — CLI（CONTRACT §1 crates/kunsu-cli）。
//!
//! HTTP client 打 `127.0.0.1:4777`（Bearer 讀 `~/.kunsu/token`）：
//! - `kunsu ask <草稿資料夾> [--dry-run]`：註冊草稿題（ask_register，PRD §24）；issues 逐行印到 stderr。
//! - `kunsu ask --json <file|->`：建題（自動補原對話身分，PRD §20）；server 不通 → 落
//!   `~/.kunsu/inbox/<timestamp>-<id>.json` 並明說降級（survival 場景 10）。
//! - `kunsu wait <id> --timeout N`：迴圈 long-poll（每輪 ≤540s，P1 定案 9）。
//! - `kunsu drain --consumer <name>`：游標收割、JSONL 到 stdout（at-least-once：
//!   印出成功後才用 next_cursor ack）。
//! - `kunsu list [--json]` / `kunsu answer <id> ...`：answer 有 isatty 人類閘——
//!   非互動環境一律拒絕（CONTRACT §4 CLI 作答通道限使用者本人終端）。
//! - `kunsu rebuild-db`：server 活著走 `/admin/rebuild-db`；死了本地以 lib 重建。
//! - `kunsu doctor`：分層冒煙（survival 場景 6/7；isatty 層限真人終端）。
//!
//! 環境變數覆寫（與 kunsu-serverd 同款）：KUNSU_BIND / KUNSU_HOME_DIR /
//! KUNSU_VAULT_ROOT / KUNSU_DB_PATH。

use std::io::{IsTerminal, Read};
use std::path::PathBuf;
use std::time::Duration;

use serde_json::{json, Value};

const DEFAULT_BIND: &str = "127.0.0.1:4777";
const LAUNCHD_PLIST: &str = "Library/LaunchAgents/me.ray.kunsu.plist";
const APP_PATH: &str = "/Applications/Kunsu.app";
/// ask_wait 單輪上限（server 端 P1 定案 9 同值）。
const WAIT_CHUNK_S: u64 = 540;
/// doctor：安裝版 `/describe` 應回的 MCP tool 數（與 kunsu-server tool_specs 同步）。
const EXPECTED_MCP_TOOLS: usize = 14;

fn env_or(key: &str, default: String) -> String {
    std::env::var(key).ok().filter(|v| !v.trim().is_empty()).unwrap_or(default)
}

fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string()))
}

fn kunsu_dir() -> PathBuf {
    PathBuf::from(env_or(
        "KUNSU_HOME_DIR",
        home().join(".kunsu").to_string_lossy().to_string(),
    ))
}

fn vault_root() -> PathBuf {
    kunsu_server::ledger::vault_root_for_this_mac()
}

fn base_url() -> String {
    format!("http://{}", env_or("KUNSU_BIND", DEFAULT_BIND.to_string()))
}

// ---------------------------------------------------------------------------
// HTTP / MCP client
// ---------------------------------------------------------------------------

struct Client {
    http: reqwest::Client,
    base: String,
    token: Option<String>,
    /// admin token（~/.kunsu/admin-token）：/cli/answer 與 /admin/* 專用。
    /// agent token 對這兩個路由無效（server 端 401）。
    admin_token: Option<String>,
}

/// 錯誤分兩類：Unreachable（連不上 → ask 走 inbox 降級）與其餘（人話訊息）。
enum CliError {
    Unreachable(String),
    Other(String),
}

impl CliError {
    fn msg(&self) -> &str {
        match self {
            CliError::Unreachable(m) | CliError::Other(m) => m,
        }
    }
}

impl Client {
    fn new() -> Client {
        let read_token = |name: &str| {
            std::fs::read_to_string(kunsu_dir().join(name))
                .ok()
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
        };
        Client {
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(WAIT_CHUNK_S + 30))
                .build()
                .expect("reqwest client 恆可建立"),
            base: base_url(),
            token: read_token("token"),
            admin_token: read_token("admin-token"),
        }
    }

    fn bearer(&self) -> Result<&str, CliError> {
        self.token.as_deref().ok_or_else(|| {
            CliError::Other(format!(
                "讀不到 Bearer token（{}/token）——Kunsu app 首次啟動會自動生成",
                kunsu_dir().display()
            ))
        })
    }

    fn admin_bearer(&self) -> Result<&str, CliError> {
        self.admin_token.as_deref().ok_or_else(|| {
            CliError::Other(format!(
                "讀不到 admin token（{}/admin-token）——作答與 rebuild-db 走 admin 面，Kunsu app 首次啟動會自動生成；agent token 對此路由無效",
                kunsu_dir().display()
            ))
        })
    }

    async fn get_json(&self, path: &str, auth: bool) -> Result<Value, CliError> {
        let mut req = self.http.get(format!("{}{}", self.base, path));
        if auth {
            req = req.bearer_auth(self.bearer()?);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| CliError::Unreachable(format!("GET {path} 連線失敗：{e}")))?;
        let status = resp.status();
        let body: Value = resp
            .json()
            .await
            .map_err(|e| CliError::Other(format!("GET {path} 回應非 JSON：{e}")))?;
        if status.is_success() {
            Ok(body)
        } else {
            Err(CliError::Other(format!("GET {path} → {status}：{body}")))
        }
    }

    async fn post_json(&self, path: &str, body: &Value) -> Result<Value, CliError> {
        // admin 面（/cli/answer、/admin/*）用 admin token；其餘 agent token。
        let bearer = if path.starts_with("/cli/") || path.starts_with("/admin/") {
            self.admin_bearer()?
        } else {
            self.bearer()?
        };
        let resp = self
            .http
            .post(format!("{}{}", self.base, path))
            .bearer_auth(bearer)
            .json(body)
            .send()
            .await
            .map_err(|e| CliError::Unreachable(format!("POST {path} 連線失敗：{e}")))?;
        let status = resp.status();
        let body: Value = resp
            .json()
            .await
            .map_err(|e| CliError::Other(format!("POST {path} 回應非 JSON：{e}")))?;
        if status.is_success() {
            Ok(body)
        } else {
            Err(CliError::Other(format!("POST {path} → {status}：{body}")))
        }
    }

    /// MCP tools/call：回 structuredContent（server 一定帶）。
    async fn mcp_call(&self, tool: &str, args: Value) -> Result<Value, CliError> {
        let rpc = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": tool, "arguments": args },
        });
        let body = self.post_json("/mcp", &rpc).await?;
        if let Some(err) = body.get("error") {
            let msg = err.get("message").and_then(Value::as_str).unwrap_or("未知錯誤");
            return Err(CliError::Other(format!("{tool} 失敗：{msg}")));
        }
        body.get("result")
            .and_then(|r| r.get("structuredContent"))
            .cloned()
            .ok_or_else(|| CliError::Other(format!("{tool} 回應缺 structuredContent")))
    }

    async fn mcp_initialize(&self) -> Result<Value, CliError> {
        let rpc = json!({
            "jsonrpc": "2.0", "id": 0, "method": "initialize",
            "params": { "protocolVersion": "2025-03-26", "capabilities": {},
                        "clientInfo": {"name": "kunsu-cli", "version": env!("CARGO_PKG_VERSION")} },
        });
        let body = self.post_json("/mcp", &rpc).await?;
        body.get("result")
            .cloned()
            .ok_or_else(|| CliError::Other(format!("initialize 無 result：{body}")))
    }
}

// ---------------------------------------------------------------------------
// main / dispatch
// ---------------------------------------------------------------------------

fn usage() -> ! {
    eprintln!(
        "kunsu — Kunsu（軍師）CLI\n\n\
         用法：\n\
         \x20 kunsu ask <草稿資料夾> [--dry-run]  註冊草稿題（~/.kunsu/drafts/<id>/；有問題逐行列出，exit 3）\n\
         \x20 kunsu ask --json <file|->          建題（自動帶上原對話身分；server 死 → 降級落 ~/.kunsu/inbox）\n\
         \x20     [--routine <id> --routine-name <名稱> --every <小時> --routine-kind <種類>]  排程 check-in\n\
         \x20 kunsu wait <id> [--timeout N]      long-poll 等單題終態（預設 600s；0 = 一直等到答案；答案已在別處處理 → exit 5；使用者追問 → exit 6）\n\
         \x20 kunsu followup-reply <id> <追問 id> <回覆>  回答使用者在手機上的追問（不是答案）\n\
         \x20 kunsu drain --consumer <name>      游標收割終態，JSONL 到 stdout\n\
         \x20 kunsu list [--json]                列出待決題\n\
         \x20 kunsu answer <id> (--choice <cid> | --text <t>) [--params <line>]\n\
         \x20                                    作答（限互動終端；isatty 人類閘）\n\
         \x20 kunsu rebuild-db                   從 vault ledger 重建 SQLite\n\
         \x20 kunsu doctor                       分層冒煙（真人終端才跑作答回路）"
    );
    std::process::exit(64);
}

fn flag_value(args: &[String], flag: &str) -> Option<String> {
    args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned()
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = args.first() else { usage() };
    let rest = &args[1..];
    let code = match cmd.as_str() {
        "ask" => cmd_ask(rest).await,
        "wait" => cmd_wait(rest).await,
        "followup-reply" => cmd_followup_reply(rest).await,
        "drain" => cmd_drain(rest).await,
        "list" => cmd_list(rest).await,
        "answer" => cmd_answer(rest).await,
        "rebuild-db" => cmd_rebuild_db().await,
        "doctor" => cmd_doctor(rest).await,
        _ => usage(),
    };
    std::process::exit(code);
}

// ---------------------------------------------------------------------------
// ask
// ---------------------------------------------------------------------------

async fn cmd_ask(args: &[String]) -> i32 {
    if let Some(dir) = ask_target(args) {
        return cmd_ask_draft(&dir, args).await;
    }
    let Some(src) = flag_value(args, "--json") else {
        eprintln!("kunsu ask 需要 <草稿資料夾> 或 --json <file|->");
        return 64;
    };
    let raw = if src == "-" {
        let mut buf = String::new();
        if let Err(e) = std::io::stdin().read_to_string(&mut buf) {
            eprintln!("讀 stdin 失敗：{e}");
            return 1;
        }
        buf
    } else {
        match std::fs::read_to_string(&src) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("讀 {src} 失敗：{e}");
                return 1;
            }
        }
    };
    let mut payload: Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("--json 內容非法 JSON：{e}");
            return 1;
        }
    };
    if let Err(e) = ask_json_flags_ok(args) {
        eprintln!("{e}");
        return 64;
    }
    let routine = match routine_flags(args) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{e}");
            return 64;
        }
    };
    // 回答延續原對話：自動帶上發問的這段對話（只補沒填的欄位，不碰任何 token）。
    let cwd = std::env::current_dir().ok().map(|p| p.to_string_lossy().to_string());
    augment_source(&mut payload, &|k| std::env::var(k).ok(), cwd, &routine);
    let qid = payload.get("id").and_then(Value::as_str).unwrap_or("").to_string();
    if qid.is_empty() {
        eprintln!("建題 JSON 缺 id（穩定 dedup id，如 pa-*）");
        return 1;
    }

    let client = Client::new();
    match client.mcp_call("ask_create", payload.clone()).await {
        Ok(outcome) => {
            println!("{}", serde_json::to_string_pretty(&outcome).unwrap_or_default());
            0
        }
        Err(CliError::Unreachable(reason)) => degrade_to_inbox(&payload, &qid, &reason),
        Err(e) => {
            eprintln!("{}", e.msg());
            1
        }
    }
}

/// server 不通 → 檔案 inbox 降級（P1 定案 6：與 ask_create arguments 完全同
/// 格式、一檔一題、檔名 `<timestamp>-<id>.json`）。降級必須明說，絕不假裝成功。
fn degrade_to_inbox(payload: &Value, qid: &str, reason: &str) -> i32 {
    let inbox = kunsu_dir().join("inbox");
    if let Err(e) = std::fs::create_dir_all(&inbox) {
        eprintln!("Kunsu server 不通（{reason}），且 inbox 目錄建立失敗：{e}");
        return 1;
    }
    let ts = chrono::Utc::now().timestamp();
    let path = inbox.join(format!("{ts}-{qid}.json"));
    match std::fs::write(&path, serde_json::to_string_pretty(payload).unwrap_or_default()) {
        Ok(()) => {
            eprintln!(
                "⚠️ 降級模式：Kunsu server 不通（{reason}）。\n\
                 題目已落檔案 inbox：{}\n\
                 app 復活後會自動 ingest（60s 掃描）；在那之前不會有蓋板/通知。",
                path.display()
            );
            println!("{}", json!({"degraded": true, "inbox_file": path.to_string_lossy()}));
            0
        }
        Err(e) => {
            eprintln!("Kunsu server 不通（{reason}），inbox 落檔也失敗：{e}");
            1
        }
    }
}

/// `kunsu ask <草稿資料夾>`：第一個非旗標參數是資料夾 → 草稿註冊模式（PRD §24）。
fn ask_target(args: &[String]) -> Option<std::path::PathBuf> {
    let first = args.first().filter(|a| !a.starts_with("--"))?;
    let path = std::path::PathBuf::from(first);
    path.is_dir().then_some(path)
}

/// ask_register 參數：資料夾絕對路徑、dry_run、偵測到的身分（不在 agent 環境就不帶，沿用 frontmatter）。
fn register_payload(
    dir: &std::path::Path,
    args: &[String],
    env: &dyn Fn(&str) -> Option<String>,
    cwd: Option<String>,
) -> Value {
    let abs = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    let mut payload = json!({
        "draft_dir": abs.to_string_lossy(),
        "dry_run": args.iter().any(|a| a == "--dry-run"),
    });
    let mut probe = json!({"source": {}});
    augment_source(&mut probe, env, cwd, &routine_flags(args).unwrap_or_default());
    if let Some(src) = probe.get("source").filter(|s| s.as_object().is_some_and(|m| !m.is_empty())) {
        payload["source"] = src.clone();
    }
    payload
}

/// 草稿註冊的旗標檢查：拼錯（例如 --dryrun）不能默默變成正式註冊。
fn draft_flags_ok(args: &[String]) -> Result<(), String> {
    const WITH_VALUE: [&str; 4] = ["--routine", "--routine-name", "--every", "--routine-kind"];
    let mut i = 1; // args[0] 是草稿資料夾
    while i < args.len() {
        let a = args[i].as_str();
        if a == "--dry-run" {
            i += 1;
        } else if WITH_VALUE.contains(&a) {
            i += 2;
        } else {
            return Err(format!("不認得的參數 {a:?}：草稿註冊只接受 --dry-run 與 --routine／--routine-name／--every／--routine-kind"));
        }
    }
    routine_flags(args).map(|_| ())
}

/// `kunsu ask --json` 只接受 --json 與排程旗標；`--every=24`、拼錯的旗標一律報錯，不靜默建成互動題。
fn ask_json_flags_ok(args: &[String]) -> Result<(), String> {
    const WITH_VALUE: [&str; 5] = ["--json", "--routine", "--routine-name", "--every", "--routine-kind"];
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if WITH_VALUE.contains(&a) {
            i += 2;
        } else {
            return Err(format!(
                "不認得的參數 {a:?}：kunsu ask --json 只接受 --json <file|-> 與 --routine／--routine-name／--every／--routine-kind（值用空白隔開）"
            ));
        }
    }
    routine_flags(args).map(|_| ())
}

async fn cmd_ask_draft(dir: &std::path::Path, args: &[String]) -> i32 {
    if let Err(e) = draft_flags_ok(args) {
        eprintln!("{e}");
        return 64;
    }
    let cwd = std::env::current_dir().ok().map(|p| p.to_string_lossy().to_string());
    let payload = register_payload(dir, args, &|k| std::env::var(k).ok(), cwd);
    match Client::new().mcp_call("ask_register", payload).await {
        Ok(outcome) => {
            println!("{}", serde_json::to_string_pretty(&outcome).unwrap_or_default());
            let ok = outcome.get("ok").and_then(Value::as_bool).unwrap_or(false);
            if !ok {
                // 人看的摘要走 stderr；agent 讀 stdout 的 JSON。
                for issue in outcome.get("issues").and_then(Value::as_array).into_iter().flatten() {
                    let at = match (issue.get("line").and_then(Value::as_u64), issue.get("column").and_then(Value::as_u64)) {
                        (Some(l), Some(c)) => format!(":{l}:{c}"),
                        (Some(l), None) => format!(":{l}"),
                        _ => String::new(),
                    };
                    eprintln!(
                        "{}{at} [{}] {}\n  → {}",
                        issue.get("file").and_then(Value::as_str).unwrap_or(""),
                        issue.get("code").and_then(Value::as_str).unwrap_or(""),
                        issue.get("message").and_then(Value::as_str).unwrap_or(""),
                        issue.get("hint").and_then(Value::as_str).unwrap_or(""),
                    );
                }
            }
            if ok { 0 } else { 3 }
        }
        Err(e) => {
            // 草稿要靠 app 的畫布檢查器，app 沒開就無法降級成 inbox。
            eprintln!("{}", e.msg());
            1
        }
    }
}

// ---------------------------------------------------------------------------
// wait
// ---------------------------------------------------------------------------

async fn cmd_wait(args: &[String]) -> i32 {
    let Some(qid) = args.first().filter(|a| !a.starts_with("--")).cloned() else {
        eprintln!("kunsu wait 需要 <id>");
        return 64;
    };
    // --timeout 0 = 無總時限：原對話在背景掛著等，答案一到就喚醒（PRD §20）。
    let total = wait_total(args);
    let client = Client::new();
    let deadline = total.map(|s| std::time::Instant::now() + Duration::from_secs(s));
    let mut failures = 0u32;
    // 發問的那段對話消失 → 孤兒等待者自行退出，斷線即釋放租約（AC-CC-18）。Claude 背景 Bash 中間夾一層 zsh，
    // 直接父行程不會變成 1，所以看 $CLAUDE_PID（發問對話的行程）是否還活著。
    #[cfg(unix)]
    tokio::spawn(async {
        let conversation_pid = std::env::var("CLAUDE_PID").ok().and_then(|p| p.trim().parse::<u32>().ok());
        loop {
            tokio::time::sleep(Duration::from_secs(2)).await;
            if orphaned(std::os::unix::process::parent_id(), conversation_pid.map(pid_alive)) {
                eprintln!("kunsu wait：發問的對話已結束，停止等待（答案會改由 Kunsu 送回原對話）");
                std::process::exit(4);
            }
        }
    });
    loop {
        let chunk = match deadline {
            Some(d) => {
                let remaining = d.saturating_duration_since(std::time::Instant::now());
                if remaining.is_zero() {
                    println!("{}", json!({"status": "pending_timeout"}));
                    return 2;
                }
                remaining.as_secs().clamp(1, WAIT_CHUNK_S)
            }
            None => WAIT_CHUNK_S,
        };
        match client
            .mcp_call("ask_wait", json!({"question_id": qid, "timeout_seconds": chunk}))
            .await
        {
            Ok(res) => {
                let status = res.get("status").and_then(Value::as_str).unwrap_or("");
                if status != "pending_timeout" {
                    println!("{}", serde_json::to_string_pretty(&res).unwrap_or_default());
                    let (code, message) = wait_exit_for(&qid, &res);
                    if let Some(m) = message {
                        eprintln!("{m}");
                    }
                    return code;
                }
                // 續等下一輪。
                failures = 0;
            }
            Err(e) => match wait_retry_delay(&e, failures) {
                // app 重啟／更新中：背景等待不能因此斷線，退避後續等。
                Some(delay) => {
                    eprintln!("{}（{} 秒後重試）", e.msg(), delay.as_secs());
                    failures += 1;
                    tokio::time::sleep(delay).await;
                }
                None => {
                    eprintln!("{}", e.msg());
                    return 1;
                }
            },
        }
    }
}

/// 使用者在手機上問本人 → 結束碼 6，告訴原 AI 怎麼回、回完再等（AC-RA-05）；其他照 `wait_exit`。
fn wait_exit_for(qid: &str, res: &Value) -> (i32, Option<String>) {
    if res.get("status").and_then(Value::as_str) == Some("followup") {
        let fid = res.pointer("/followup/id").and_then(Value::as_str).unwrap_or("");
        let text = res.pointer("/followup/text").and_then(Value::as_str).unwrap_or("");
        return (
            6,
            Some(format!(
                "使用者在手機上追問你（不是答案，題目還在等）：{text}\n\
                 回答：~/.kunsu/bin/kunsu followup-reply {qid} {fid} \"<回覆>\"\n\
                 回完再背景執行：~/.kunsu/bin/kunsu wait {qid} --timeout 0"
            )),
        );
    }
    wait_exit(res)
}

/// `kunsu followup-reply <題目 id> <追問 id> <回覆…>` 的參數。
fn followup_reply_payload(args: &[String]) -> Result<Value, String> {
    match args {
        [qid, fid, text @ ..] if !text.is_empty() => {
            Ok(json!({"question_id": qid, "followup_id": fid, "text": text.join(" ")}))
        }
        _ => Err("用法：kunsu followup-reply <題目 id> <追問 id> \"<回覆>\"".into()),
    }
}

async fn cmd_followup_reply(args: &[String]) -> i32 {
    let payload = match followup_reply_payload(args) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            return 64;
        }
    };
    match Client::new().mcp_call("ask_followup_reply", payload).await {
        Ok(res) => {
            println!("{}", serde_json::to_string_pretty(&res).unwrap_or_default());
            0
        }
        Err(e) => {
            eprintln!("{}", e.msg());
            1
        }
    }
}

/// 答案已交到別處（追加、開新對話、原對話自行回報…）→ 結束碼 5，提醒不要重複處理（AC-CC-18）。
fn wait_exit(res: &Value) -> (i32, Option<String>) {
    let answered = res.get("status").and_then(Value::as_str) == Some("answered");
    match res.pointer("/delivery/state").and_then(Value::as_str) {
        Some(state @ ("resuming" | "resumed" | "fresh" | "picked_up")) if answered => (
            5,
            Some(format!("答案已在別處處理（交付狀態 {state}），不要重複處理；看 answer.report 或 Kunsu 回執即可")),
        ),
        _ => (0, None),
    }
}

/// 發問的對話已經不在：對話行程（已知時）不再存活，或直接父行程變成 launchd（pid 1）。
fn orphaned(parent_pid: u32, conversation_alive: Option<bool>) -> bool {
    parent_pid == 1 || conversation_alive == Some(false)
}

/// 行程是否還在（signal 0：存在但沒權限也算活著）。
fn pid_alive(pid: u32) -> bool {
    let Ok(pid) = libc::pid_t::try_from(pid) else { return false };
    if pid <= 0 {
        return false;
    }
    // SAFETY：signal 0 不送任何訊號，只檢查行程是否存在。
    let rc = unsafe { libc::kill(pid, 0) };
    rc == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// 連不上 app → 1, 2, 4… 秒退避（上限 30 秒）；其他錯誤不重試。
fn wait_retry_delay(err: &CliError, failures: u32) -> Option<Duration> {
    match err {
        CliError::Unreachable(_) => Some(Duration::from_secs(2u64.saturating_pow(failures.min(5)).min(30))),
        CliError::Other(_) => None,
    }
}

// ---------------------------------------------------------------------------
// drain
// ---------------------------------------------------------------------------

async fn cmd_drain(args: &[String]) -> i32 {
    let Some(consumer) = flag_value(args, "--consumer") else {
        eprintln!("kunsu drain 需要 --consumer <name>");
        return 64;
    };
    let client = Client::new();
    // at-least-once：先收（不帶 cursor＝沿用存檔游標），印出成功後才帶
    // next_cursor 續呼叫（ack 前一批）。中途失敗 → 下次 drain 重收同批。
    let mut cursor: Option<i64> = None;
    let mut total = 0u32;
    loop {
        let mut req = json!({"consumer": consumer});
        if let Some(c) = cursor {
            req["cursor"] = json!(c);
        }
        let batch = match client.mcp_call("ask_answers_since", req).await {
            Ok(b) => b,
            Err(e) => {
                eprintln!("{}", e.msg());
                return 1;
            }
        };
        let events = batch.get("events").and_then(Value::as_array).cloned().unwrap_or_default();
        if events.is_empty() {
            eprintln!("drain：共 {total} 筆終態事件（consumer={consumer}）");
            return 0;
        }
        for ev in &events {
            // JSONL：一行一事件（question 全量快照）。
            println!("{ev}");
        }
        total += events.len() as u32;
        cursor = batch.get("next_cursor").and_then(Value::as_i64);
        if cursor.is_none() {
            eprintln!("drain：server 未回 next_cursor，停止（已收 {total} 筆）");
            return 1;
        }
    }
}

// ---------------------------------------------------------------------------
// list
// ---------------------------------------------------------------------------

async fn cmd_list(args: &[String]) -> i32 {
    let as_json = args.iter().any(|a| a == "--json");
    let client = Client::new();
    let res = match client.mcp_call("ask_list", json!({})).await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{}", e.msg());
            return 1;
        }
    };
    let questions = res.get("questions").and_then(Value::as_array).cloned().unwrap_or_default();
    if as_json {
        println!("{}", serde_json::to_string_pretty(&questions).unwrap_or_default());
        return 0;
    }
    if questions.is_empty() {
        println!("沒有待決題。");
        return 0;
    }
    let today = chrono::Utc::now().date_naive();
    println!("{:<28} {:<4} {:<9} {:<6} {:<10} {:>4}  題", "id", "tier", "state", "gate", "product", "積壓");
    for q in &questions {
        let s = |k: &str| q.get(k).and_then(Value::as_str).unwrap_or("-").to_string();
        let days = q
            .get("first_proposed")
            .and_then(Value::as_str)
            .and_then(|d| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
            .map(|d| (today - d).num_days())
            .unwrap_or(0);
        println!(
            "{:<28} {:<4} {:<9} {:<6} {:<10} {:>4}  {}",
            s("id"),
            s("tier"),
            s("state"),
            s("gate"),
            s("product"),
            days,
            s("question_line"),
        );
    }
    0
}

// ---------------------------------------------------------------------------
// answer（isatty 人類閘）
// ---------------------------------------------------------------------------

fn is_interactive() -> bool {
    std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
}

async fn cmd_answer(args: &[String]) -> i32 {
    if !is_interactive() {
        eprintln!(
            "⛔ kunsu answer 只能在互動終端使用（isatty 檢查失敗）。\n\
             這是 CLI 作答通道的人類閘：agent / 腳本 / pipe 一律拒絕——\n\
             作答只能來自使用者本人（CONTRACT §4）。"
        );
        return 3;
    }
    let Some(qid) = args.first().filter(|a| !a.starts_with("--")).cloned() else {
        eprintln!("kunsu answer 需要 <id>");
        return 64;
    };
    let choice = flag_value(args, "--choice");
    let text = flag_value(args, "--text");
    let params = flag_value(args, "--params");
    if choice.is_none() && text.is_none() {
        eprintln!("kunsu answer 需要 --choice <option_id> 或 --text <自由文字>");
        return 64;
    }
    let client = Client::new();
    let body = json!({
        "question_id": qid,
        "choice_id": choice,
        "free_text": text,
        "edited_params": params,
    });
    match client.post_json("/cli/answer", &body).await {
        Ok(res) => {
            let state = res
                .get("question")
                .and_then(|q| q.get("state"))
                .and_then(Value::as_str)
                .unwrap_or("?");
            println!("✅ 已作答（channel=cli）：{qid} → {state}");
            0
        }
        Err(e) => {
            eprintln!("{}", e.msg());
            1
        }
    }
}

// ---------------------------------------------------------------------------
// rebuild-db
// ---------------------------------------------------------------------------

async fn cmd_rebuild_db() -> i32 {
    let client = Client::new();
    match client.post_json("/admin/rebuild-db", &json!({})).await {
        Ok(res) => {
            println!("rebuild-db（server 通道）完成：{res}");
            return 0;
        }
        Err(CliError::Unreachable(_)) => {
            eprintln!("server 不通——改用本地重建（直接讀 vault ledger 寫 SQLite）。");
        }
        Err(e) => {
            eprintln!("{}", e.msg());
            return 1;
        }
    }
    match local_rebuild().await {
        Ok(report) => {
            println!(
                "rebuild-db（本地）完成：{}",
                serde_json::to_string(&report).unwrap_or_default()
            );
            0
        }
        Err(e) => {
            eprintln!("本地 rebuild 失敗：{e}");
            1
        }
    }
}

/// app 的 SQLite 路徑（與 app 相同的 `KUNSU_DB_PATH` 覆寫）。
fn db_path() -> PathBuf {
    PathBuf::from(env_or(
        "KUNSU_DB_PATH",
        home().join("Library/Application Support/me.ray.kunsu/kunsu.db").to_string_lossy().to_string(),
    ))
}

async fn local_rebuild() -> Result<kunsu_server::RebuildReport, String> {
    let db_path = db_path();
    if let Some(dir) = db_path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("DB 目錄建立失敗：{e}"))?;
    }
    let handle = kunsu_server::ServerHandle::new(kunsu_server::KunsuConfig {
        db_path,
        vault_root: vault_root(),
        policy: kunsu_core::policy::Policy::default(),
        discord: std::sync::Arc::new(kunsu_server::LoggingDiscord),
        guards: std::sync::Arc::new(kunsu_server::StubGuardProbe::new()),
    })
    .await
    .map_err(|e| format!("開 DB 失敗：{e}"))?;
    handle.rebuild_db().await.map_err(|e| format!("重建失敗：{e}"))
}

// ---------------------------------------------------------------------------
// doctor
// ---------------------------------------------------------------------------

struct Check {
    name: &'static str,
    ok: bool,
}

fn check(list: &mut Vec<Check>, name: &'static str, res: Result<String, String>) {
    match res {
        Ok(detail) => {
            println!("  ✅ {name} — {detail}");
            list.push(Check { name, ok: true });
        }
        Err(detail) => {
            println!("  ❌ {name} — {detail}");
            list.push(Check { name, ok: false });
        }
    }
}

async fn cmd_doctor(args: &[String]) -> i32 {
    // --install-gate：build-install.sh 的回滾判準模式。MCP 註冊是一次性人工
    // 步驟（doctor 只印指令不自己改），不該讓「還沒註冊」觸發 .app 回滾；
    // 此模式下註冊缺失照印指令但記 WARN 不記 FAIL。手動跑 doctor 仍是硬檢查。
    let install_gate = args.iter().any(|a| a == "--install-gate");
    let client = Client::new();
    let mut checks: Vec<Check> = Vec::new();
    println!("kunsu doctor — 無條件層");

    // (a1) /health 可達（無 auth）。
    let health = client.get_json("/health", false).await;
    check(
        &mut checks,
        "health",
        match &health {
            Ok(h) => Ok(format!(
                "pending={} overdue={} oldest_pending_days={}",
                h.get("pending_count").and_then(Value::as_u64).unwrap_or(0),
                h.get("overdue_count").and_then(Value::as_u64).unwrap_or(0),
                h.get("oldest_pending_days").and_then(Value::as_i64).unwrap_or(0),
            )),
            Err(e) => Err(format!("GET /health 失敗：{}", e.msg())),
        },
    );

    // (a2) /describe tool 數與本版 server 一致。
    check(
        &mut checks,
        "describe",
        match client.get_json("/describe", true).await {
            Ok(d) => {
                let n = d.get("tools").and_then(Value::as_array).map(|t| t.len()).unwrap_or(0);
                if n == EXPECTED_MCP_TOOLS {
                    Ok(format!("{n} tools"))
                } else {
                    Err(format!("tools 數量 {n} ≠ {EXPECTED_MCP_TOOLS}"))
                }
            }
            Err(e) => Err(e.msg().to_string()),
        },
    );

    // (a3) MCP initialize 握手。
    check(
        &mut checks,
        "mcp_initialize",
        match client.mcp_initialize().await {
            Ok(r) => {
                let name = r
                    .get("serverInfo")
                    .and_then(|s| s.get("name"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if name == "kunsu" {
                    Ok(format!(
                        "serverInfo.name=kunsu protocolVersion={}",
                        r.get("protocolVersion").and_then(Value::as_str).unwrap_or("?")
                    ))
                } else {
                    Err(format!("serverInfo.name={name:?} ≠ kunsu"))
                }
            }
            Err(e) => Err(e.msg().to_string()),
        },
    );

    // (a3.5) admin token 檔（兩 token 邊界：/cli/answer 與 /admin/* 專用）。
    check(&mut checks, "admin_token_file", {
        let path = kunsu_dir().join("admin-token");
        match std::fs::read_to_string(&path) {
            Ok(t) if !t.trim().is_empty() => {
                Ok(format!("{} 存在（CLI 作答/rebuild-db 專用，agent 永不引用）", path.display()))
            }
            _ => Err(format!(
                "{} 讀不到——Kunsu app / kunsu-serverd 首次啟動會生成；沒有它 kunsu answer 與 rebuild-db 打不進 admin 面",
                path.display()
            )),
        }
    });

    // (a4) ledger 路徑可寫（探針檔寫後即刪；絕不碰 jsonl 本體）。
    check(&mut checks, "ledger_writable", {
        let asks = vault_root().join("logs").join("asks");
        let probe = asks.join(".doctor-write-probe");
        (|| -> Result<String, String> {
            std::fs::create_dir_all(&asks).map_err(|e| format!("建目錄失敗：{e}"))?;
            std::fs::write(&probe, b"probe").map_err(|e| format!("寫入失敗：{e}"))?;
            std::fs::remove_file(&probe).map_err(|e| format!("清理探針失敗：{e}"))?;
            Ok(format!("{} 可寫", asks.display()))
        })()
    });

    // (a4.5) app 行程自己的帳本狀態（/health）：ledger_writable 是從終端機的權限探測，app 寫不進去時照樣 ✅，
    // 所以另外看 app 回報的待補寫行數。只警告、不計入失敗（不觸發安裝回滾）；刪資料庫前一定要是 0。
    match &health {
        Ok(h) if h.get("ledger_backlog").is_none() => {
            println!("  ⚠️ ledger_backlog（WARN）— app 版本比 CLI 舊，/health 沒有帳本積欠欄位，無法確認；刪資料庫前先更新 Kunsu 再跑一次");
        }
        Ok(h) => {
        let backlog = h.get("ledger_backlog").and_then(Value::as_u64).unwrap_or(0);
        let dirty = h.get("open_json_dirty").and_then(Value::as_bool).unwrap_or(false);
        let stuck = h.get("ledger_stuck_s").and_then(Value::as_i64).unwrap_or(0);
        if backlog == 0 && !dirty && stuck == 0 {
            println!("  ✅ ledger_backlog — app 帳本沒有積欠");
        } else {
            println!(
                "  ⚠️ ledger_backlog（WARN）— app 帳本有 {backlog} 行沒補寫{}{}：先給 Kunsu.app 完全磁碟權限讓它補寫，補完前不要刪資料庫或 rebuild-db",
                if dirty { "、OPEN.json 待重寫" } else { "" },
                if stuck > 0 { format!("、寫入已卡住 {stuck} 秒（超過 2 分鐘請重啟 Kunsu）") } else { String::new() },
            );
        }
        }
        // app 沒開：唯讀打開資料庫自己數（只讀，不建檔、不寫入）。
        Err(_) => match kunsu_server::db::offline_ledger_backlog(&db_path()).await {
            Ok(0) => println!("  ✅ ledger_backlog — app 沒開；資料庫裡沒有待補寫的帳本行"),
            Ok(n) => println!(
                "  ⚠️ ledger_backlog（WARN）— app 沒開；資料庫裡有 {n} 行帳本還沒補寫：先打開 Kunsu、給它完全磁碟權限讓它補寫，補完前不要刪資料庫"
            ),
            Err(e) => println!("  ⚠️ ledger_backlog（WARN）— app 沒開，也讀不到資料庫（{e}），無法確認帳本積欠；刪資料庫前先打開 Kunsu 再跑一次"),
        },
    }

    // (a4.6) 設定頁的個人覆寫（CONTRACT §15）：只說明，不算失敗、不觸發安裝回滾。
    {
        let paths = kunsu_server::settings::SettingsPaths::in_dir(&kunsu_dir());
        let base = std::fs::read_to_string(&paths.base).ok();
        let local = std::fs::read_to_string(&paths.local).ok();
        println!(
            "  ℹ️ settings — {}",
            kunsu_server::settings::doctor_line(base.as_deref(), local.as_deref(), &paths.local)
        );
    }

    // (a5) launchd plist 存在且指向 /Applications/Kunsu.app（survival 場景 8）。
    check(&mut checks, "launchd_plist", {
        let plist = home().join(LAUNCHD_PLIST);
        match std::fs::read_to_string(&plist) {
            Ok(content) if content.contains(APP_PATH) => {
                Ok(format!("{} → {APP_PATH}", plist.display()))
            }
            Ok(_) => Err(format!(
                "{} 存在但未指向 {APP_PATH}（絕不指 repo build 路徑——重跑 scripts/build-install.sh）",
                plist.display()
            )),
            Err(e) => Err(format!("{} 讀不到：{e}（跑 scripts/build-install.sh 安裝）", plist.display())),
        }
    });

    // (a6) ~/.claude.json 有 kunsu MCP 註冊（沒有就印指令，不自己改）。
    let reg_check = |list: &mut Vec<Check>, name: &'static str, res: Result<String, String>| {
        match (install_gate, res) {
            (_, Ok(d)) => {
                println!("  ✅ {name} — {d}");
                list.push(Check { name, ok: true });
            }
            (true, Err(d)) => {
                // install-gate：印指令、記 WARN、不觸發回滾。
                println!("  ⚠️ {name}（WARN，不擋安裝）— {d}");
                list.push(Check { name, ok: true });
            }
            (false, Err(d)) => {
                println!("  ❌ {name} — {d}");
                list.push(Check { name, ok: false });
            }
        }
    };
    reg_check(&mut checks, "claude_mcp_registered", {
        let path = home().join(".claude.json");
        let registered = std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
            .and_then(|v| v.get("mcpServers").and_then(|m| m.get("kunsu")).cloned())
            .is_some();
        if registered {
            Ok("~/.claude.json mcpServers.kunsu 已註冊".to_string())
        } else {
            Err(format!(
                "~/.claude.json 沒有 kunsu MCP 註冊。請使用者自己跑：\n     \
                 claude mcp add --scope user --transport http kunsu http://127.0.0.1:4777/mcp \
                 --header \"Authorization: Bearer $(cat {}/token)\"",
                kunsu_dir().display()
            ))
        }
    });

    // (a7) ~/.codex/config.toml 有 kunsu（同上，只印不改）。
    reg_check(&mut checks, "codex_mcp_registered", {
        let path = home().join(".codex").join("config.toml");
        let registered = std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| raw.parse::<toml::Table>().ok())
            .is_some_and(|t| codex_kunsu_registered(&t));
        if registered {
            Ok("Codex Kunsu 外掛或直接 MCP 已啟用".to_string())
        } else {
            // 與主視窗「Agent 連線」的修復指令同一行（kunsu_server::settings::CODEX_FIX；CONTRACT §15、§16）。
            Err("~/.codex/config.toml 沒有啟用的 kunsu 外掛或 [mcp_servers.kunsu]。請使用者自己跑（安裝本機外掛）：\n     \
                 python3 ~/.kunsu/plugins/kunsu/scripts/manage.py install"
                .to_string())
        }
    });

    // (b) isatty 層：真人終端才跑「建題→CLI 作答→驗 ledger + reactor dry-run→清除」。
    if is_interactive() {
        if health.is_ok() {
            println!("kunsu doctor — isatty 層（真人終端作答回路）");
            doctor_roundtrip(&client, &mut checks).await;
        } else {
            println!("kunsu doctor — isatty 層跳過（/health 不通，作答回路無從驗起）");
        }
    } else {
        println!(
            "kunsu doctor — isatty 層跳過（非互動環境：CLI 作答通道是人類閘，\
             agent/腳本不跑建題→作答回路）"
        );
    }

    // 總結（survival AC-SV-06：一行 PASS/FAIL 清單、失敗人話報段）。
    let failed: Vec<&str> = checks.iter().filter(|c| !c.ok).map(|c| c.name).collect();
    let summary: Vec<String> = checks
        .iter()
        .map(|c| format!("{}={}", c.name, if c.ok { "PASS" } else { "FAIL" }))
        .collect();
    println!("doctor 總結：{}", summary.join("｜"));
    if failed.is_empty() {
        println!("doctor：全綠 ✅");
        0
    } else {
        println!("doctor：{} 項失敗 ❌ — {}", failed.len(), failed.join(", "));
        1
    }
}

/// doctor isatty 層（survival 場景 6/7）：internal 測試題不佔 T0 預算、
/// 不推 Discord、reactor 只 dry-run、ledger 行帶 internal 供統計排除。
async fn doctor_roundtrip(client: &Client, checks: &mut Vec<Check>) {
    let qid = format!("pa-doctor-{}", chrono::Utc::now().timestamp());
    let create = json!({
        "id": qid,
        "title": "doctor 冒煙測試題（internal，統計排除）",
        "question_line": "doctor 冒煙可以答嗎？",
        "body_md": "kunsu doctor 自動建立；作答走 CLI 通道，reactor 只 dry-run。",
        "kind": "choice",
        "tier": "t1",
        "internal": true,
        "options": [
            {"id": "ok", "label": "可以（冒煙通過）", "recommended": true},
            {"id": "skip", "label": "略過"}
        ],
        "default_action": {"on_expire": "noop_log"},
        "wake_on_answer": {"mode": "fresh"},
        "source": {"agent": "other", "cwd": vault_root().to_string_lossy(), "routine_id": "kunsu-doctor"},
    });

    // 1) 建題。
    let created = client.mcp_call("ask_create", create).await;
    check(
        checks,
        "smoke_create",
        match &created {
            Ok(o) => Ok(format!(
                "{qid} state={}",
                o.get("state").and_then(Value::as_str).unwrap_or("?")
            )),
            Err(e) => Err(e.msg().to_string()),
        },
    );
    if created.is_err() {
        return;
    }

    // 2) CLI answer 通道作答。
    let answered = client
        .post_json("/cli/answer", &json!({"question_id": qid, "choice_id": "ok"}))
        .await;
    check(
        checks,
        "smoke_cli_answer",
        match &answered {
            Ok(_) => Ok("channel=cli 作答成功".to_string()),
            Err(e) => Err(e.msg().to_string()),
        },
    );
    if answered.is_err() {
        let _ = client.mcp_call("ask_cancel", json!({"question_id": qid})).await;
        return;
    }

    // 3) 驗 ledger 行出現（今天的 jsonl 內含本題 id 且 internal=true）。
    let ledger_file = vault_root()
        .join("logs")
        .join("asks")
        .join(format!("{}.jsonl", chrono::Utc::now().with_timezone(&chrono_tz_taipei()).date_naive()));
    check(checks, "smoke_ledger_row", {
        match std::fs::read_to_string(&ledger_file) {
            Ok(content) => {
                let hit = content.lines().rev().find(|l| l.contains(&qid));
                match hit {
                    Some(line) if line.contains("\"internal\":true") => {
                        Ok(format!("{} 有本題終態行（internal 標記）", ledger_file.display()))
                    }
                    Some(_) => Err("ledger 行存在但缺 internal:true 標記".to_string()),
                    None => Err(format!("{} 找不到 {qid} 的行", ledger_file.display())),
                }
            }
            Err(e) => Err(format!("{} 讀不到：{e}", ledger_file.display())),
        }
    });

    // 4) 驗 reactor dry-run 消化（execution_result 回寫、internal 全乾跑）。
    let mut exec_result: Option<String> = None;
    for _ in 0..20 {
        tokio::time::sleep(Duration::from_millis(500)).await;
        if let Ok(res) = client.mcp_call("ask_get", json!({"question_id": qid})).await
            && let Some(r) = res
                .get("question")
                .and_then(|q| q.get("answer"))
                .and_then(|a| a.get("execution_result"))
                .and_then(Value::as_str)
        {
            exec_result = Some(r.to_string());
            break;
        }
    }
    check(
        checks,
        "smoke_reactor_dry_run",
        match &exec_result {
            Some(r) if r.contains("DRY_RUN") => Ok(format!("execution_result={r}")),
            Some(r) => Err(format!("execution_result 不含 DRY_RUN 標記：{r}")),
            None => Err("10s 內 reactor 未回寫 execution_result".to_string()),
        },
    );

    // 5) 清除：題已進終態（answered→routed），cancel 預期被拒＝已不在 pending 池。
    let cleanup = client.mcp_call("ask_cancel", json!({"question_id": qid})).await;
    check(checks, "smoke_cleanup", {
        match cleanup {
            Ok(_) => Ok("測試題已 cancel".to_string()),
            Err(e) if e.msg().contains("終態") || e.msg().contains("terminal") => {
                Ok("測試題已在終態（不佔 pending 池），無需 cancel".to_string())
            }
            Err(e) => Err(e.msg().to_string()),
        }
    });
}

/// 台北時區（ledger 檔名用 taipei_date；避免引 chrono-tz，固定 +8 無 DST）。
fn chrono_tz_taipei() -> chrono::FixedOffset {
    chrono::FixedOffset::east_opt(8 * 3600).expect("UTC+8 合法")
}

// ---------------------------------------------------------------------------
// 原對話身分（回答延續原對話，PRD §20；BDD conversation-continuity 場景 1–2）
// ---------------------------------------------------------------------------

/// 排程身分旗標（routine-check-in 場景 1）。
#[derive(Debug, Default, Clone, PartialEq)]
struct RoutineFlags {
    id: Option<String>,
    name: Option<String>,
    kind: Option<String>,
    every_hours: Option<f64>,
}

/// `--every` 必須是有限正數（小時）；解析不了、旗標沒給值或沒搭配 `--routine` 都直接報錯，不靜默忽略（AC-RC-01b）。
fn routine_flags(args: &[String]) -> Result<RoutineFlags, String> {
    for flag in ["--routine", "--routine-name", "--every", "--routine-kind"] {
        if let Some(i) = args.iter().position(|a| a == flag)
            && args.get(i + 1).is_none_or(|v| v.starts_with("--") || v.trim().is_empty())
        {
            return Err(format!("{flag} 後面要接值"));
        }
    }
    if let Some(id) = flag_value(args, "--routine")
        && id.starts_with("kunsu-")
    {
        return Err(format!("--routine {id:?}：kunsu- 開頭保留給 Kunsu 自己的題，換一個排程 id"));
    }
    if flag_value(args, "--routine").is_none()
        && let Some(flag) = ["--routine-name", "--every", "--routine-kind"].into_iter().find(|f| args.iter().any(|a| a == f))
    {
        return Err(format!("{flag} 要搭配 --routine <排程 id>"));
    }
    let every_hours = match flag_value(args, "--every") {
        None => None,
        Some(v) => match v.trim().parse::<f64>() {
            Ok(h) if h.is_finite() && h > 0.0 => Some(h),
            _ => return Err(format!("--every {v:?} 不是正數小時：例如 --every 24（每天）、--every 168（每週）")),
        },
    };
    Ok(RoutineFlags {
        id: flag_value(args, "--routine"),
        name: flag_value(args, "--routine-name"),
        kind: flag_value(args, "--routine-kind"),
        every_hours,
    })
}

/// Doctor accepts the installed local plugin or an enabled legacy direct connection.
fn codex_kunsu_registered(config: &toml::Table) -> bool {
    let direct = config.get("mcp_servers").and_then(|m| m.get("kunsu"))
        .is_some_and(|m| m.get("enabled").and_then(toml::Value::as_bool) != Some(false));
    let plugin = config.get("plugins").and_then(|m| m.get("kunsu@personal"))
        .is_some_and(|p| p.get("enabled").and_then(toml::Value::as_bool) == Some(true));
    direct || plugin
}

/// 依 agent 的環境補齊 `source` 的原對話身分；agent 自填的值優先，絕不讀取任何 token／socket。
fn augment_source(
    payload: &mut Value,
    env: &dyn Fn(&str) -> Option<String>,
    cwd: Option<String>,
    routine: &RoutineFlags,
) {
    let present = |k: &str| env(k).filter(|v| !v.trim().is_empty());
    let codex_thread = present("CODEX_THREAD_ID");
    let claude_session = present("CLAUDE_CODE_SESSION_ID");
    let host = kunsu_core::continuity::host_from_env(
        present("__CFBundleIdentifier").as_deref(),
        present("CLAUDE_CODE_ENTRYPOINT").as_deref(),
        codex_thread.is_some(),
    );
    let Some(obj) = payload.as_object_mut() else { return };
    let in_agent = host.is_some() || claude_session.is_some();
    if !in_agent && routine.id.is_none() && obj.contains_key("source") {
        return;
    }
    let source = obj.entry("source").or_insert_with(|| json!({}));
    let Some(src) = source.as_object_mut() else { return };
    let mut fill = |key: &str, value: Option<Value>| {
        if let Some(v) = value
            && src.get(key).is_none_or(Value::is_null)
        {
            src.insert(key.to_string(), v);
        }
    };
    let is_codex = codex_thread.is_some();
    fill(
        "agent",
        Some(json!(if is_codex { "codex" } else if claude_session.is_some() { "claude" } else { "other" })),
    );
    fill("cwd", cwd.map(Value::from));
    if let Some(h) = host {
        fill("host", serde_json::to_value(h).ok());
    }
    if is_codex {
        fill("conversation_id", codex_thread.map(Value::from));
    } else {
        fill("conversation_id", claude_session.map(Value::from));
        fill("host_session_id", present("CLAUDE_CODE_HOST_SESSION_ID").map(Value::from));
        fill("cli_path", present("CLAUDE_CODE_EXECPATH").map(Value::from));
    }
    let attended = present("CLAUDE_CODE_SESSION_ATTENDED").map(|v| v == "1" || v.eq_ignore_ascii_case("true"));
    fill("attended", attended.map(Value::from));
    if let Some(id) = &routine.id {
        fill("routine_id", Some(json!(id)));
        fill("trigger", Some(json!("scheduled")));
        fill("routine_name", routine.name.clone().map(Value::from));
        fill("expected_every_hours", routine.every_hours.map(Value::from));
        // 只在環境推得出來時自動帶：Claude 排程任務可以；Codex 的 cron 與 heartbeat 從環境分不出來，不猜（要 --routine-kind）。
        let kind = routine.kind.clone().or_else(|| match host {
            Some(kunsu_core::continuity::HostKind::ClaudeDesktop | kunsu_core::continuity::HostKind::ClaudeCli) => {
                Some("claude_scheduled_task".to_string())
            }
            _ => None,
        });
        fill("routine_kind", kind.map(Value::from));
    } else if attended == Some(true) {
        fill("trigger", Some(json!("interactive")));
    }
}

/// `--timeout 0` = 無總時限（每輪仍 ≤540s）；其他值 = 總秒數。
fn wait_total(args: &[String]) -> Option<u64> {
    match flag_value(args, "--timeout").and_then(|v| v.parse::<u64>().ok()) {
        Some(0) => None,
        Some(n) => Some(n),
        None => Some(600),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// AC-EC-01/03：`kunsu ask <草稿資料夾> [--dry-run]` → ask_register；只帶偵測到的身分，
    /// 不在 agent 環境時不覆寫 frontmatter 的 source。
    #[test]
    fn ask_dir_builds_register_payload() {
        let dir = tempfile::TempDir::new().unwrap();
        let args = vec![dir.path().to_string_lossy().to_string(), "--dry-run".to_string()];
        let target = ask_target(&args).expect("資料夾 → 草稿模式");
        let env: HashMap<&str, &str> = HashMap::from([("CLAUDE_CODE_SESSION_ID", "sess-1")]);
        let payload = register_payload(&target, &args, &|k| env.get(k).map(|v| v.to_string()), Some("/w".into()));
        assert_eq!(payload["draft_dir"], json!(dir.path().canonicalize().unwrap().to_string_lossy()));
        assert_eq!(payload["dry_run"], true);
        assert_eq!(payload["source"]["agent"], "claude");
        assert_eq!(payload["source"]["conversation_id"], "sess-1");
        assert_eq!(payload["source"]["cwd"], "/w");

        let none: HashMap<&str, &str> = HashMap::new();
        let plain = vec![dir.path().to_string_lossy().to_string()];
        let payload = register_payload(&ask_target(&plain).unwrap(), &plain, &|k| none.get(k).map(|v| v.to_string()), Some("/w".into()));
        assert_eq!(payload["dry_run"], false);
        assert!(payload.get("source").is_none(), "不在 agent 環境：沿用 frontmatter：{payload}");

        assert!(ask_target(&["--json".to_string(), "q.json".to_string()]).is_none(), "--json 照舊");
    }

    /// 證據畫布 QA #8：草稿註冊遇到拼錯的旗標（例如 --dryrun）要報錯，不能當成正式註冊。
    #[test]
    fn draft_ask_rejects_unknown_flags() {
        let ok = |v: &[&str]| draft_flags_ok(&v.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert!(ok(&["/d", "--dry-run"]).is_ok());
        assert!(ok(&["/d", "--routine", "daily", "--every", "24"]).is_ok());
        assert!(ok(&["/d", "--dryrun"]).is_err());
        assert!(ok(&["/d", "-n"]).is_err());
    }

    /// QA 第二輪 #8：答案已在別處處理（追加、開新對話、原對話自行回報）→ 不要再做一次。
    #[test]
    fn wait_exit_respects_where_the_answer_went() {
        let answered = |state: Option<&str>| {
            let mut v = json!({"status": "answered", "answer": {"answer_id": "a1"}});
            if let Some(s) = state {
                v["delivery"] = json!({"state": s});
            }
            v
        };
        assert_eq!(wait_exit(&answered(None)), (0, None), "現行題：照答案做");
        assert_eq!(wait_exit(&answered(Some("live_waiter"))), (0, None), "交給你了");
        assert_eq!(wait_exit(&answered(Some("dead"))).0, 0, "沒人處理過（交付失敗）→ 照答案做");
        for s in ["resumed", "fresh", "picked_up", "resuming"] {
            let (code, msg) = wait_exit(&answered(Some(s)));
            assert_eq!(code, 5, "{s}");
            assert!(msg.unwrap().contains("不要重複"), "{s}");
        }
        assert_eq!(wait_exit(&json!({"status": "expired"})), (0, None));
    }

    /// AC-RA-05：使用者在手機上問本人 → 等待以結束碼 6 結束，告訴原 AI 怎麼回、回完再等。
    #[test]
    fn wait_exit_on_followup_tells_the_agent_how_to_reply() {
        let v = json!({"status": "followup", "followup": {"id": "f1", "text": "含稅嗎？"}});
        let (code, msg) = wait_exit_for("q1", &v);
        assert_eq!(code, 6);
        let msg = msg.unwrap();
        assert!(msg.contains("含稅嗎？"), "{msg}");
        assert!(msg.contains("kunsu followup-reply q1 f1"), "{msg}");
        assert!(msg.contains("kunsu wait q1 --timeout 0"), "{msg}");
    }

    #[test]
    fn followup_reply_args_are_question_followup_and_text() {
        let args: Vec<String> = ["q1", "f1", "含稅，", "總共", "5,250"].iter().map(|s| s.to_string()).collect();
        assert_eq!(
            followup_reply_payload(&args).unwrap(),
            json!({"question_id": "q1", "followup_id": "f1", "text": "含稅， 總共 5,250"})
        );
        assert!(followup_reply_payload(&args[..2]).is_err());
    }

    /// QA 第二輪 E／第三輪 #1：發問的對話不在了 → 孤兒等待者自己退出，不佔住答案。Claude 背景 Bash 中間夾一層
    /// zsh，直接父行程不會變成 1，所以優先看發問對話的行程（$CLAUDE_PID）是否還活著。
    #[test]
    fn orphaned_waiter_detection() {
        assert!(orphaned(1, None), "直接被 launchd 收養");
        assert!(!orphaned(4242, None));
        assert!(orphaned(4242, Some(false)), "對話行程已結束（中間夾 zsh 也一樣）");
        assert!(!orphaned(4242, Some(true)));
        assert_eq!(pid_alive(std::process::id()), true);
        assert_eq!(pid_alive(0x7fff_fff0), false);
    }

    /// QA #12：`kunsu wait` 連不上 app（重啟、更新）時退避重試而不是退出；其他錯誤照常結束。
    #[test]
    fn wait_retries_unreachable_with_backoff() {
        let unreachable = CliError::Unreachable("連線失敗".into());
        let other = CliError::Other("題目不存在".into());
        assert_eq!(wait_retry_delay(&unreachable, 0), Some(Duration::from_secs(1)));
        assert_eq!(wait_retry_delay(&unreachable, 3), Some(Duration::from_secs(8)));
        assert_eq!(wait_retry_delay(&unreachable, 20), Some(Duration::from_secs(30)), "上限 30 秒");
        assert_eq!(wait_retry_delay(&other, 0), None);
    }

    fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> =
            pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        move |k: &str| map.get(k).cloned()
    }

    fn base() -> Value {
        json!({"id": "pa-x", "source": {"agent": "claude", "cwd": "/w"}})
    }

    /// AC-CC-01：Claude 桌面（含排程）自動補 host／conversation／host session／attended／cli_path，不碰 token。
    #[test]
    fn claude_desktop_identity_is_added_without_secrets() {
        let env = env_of(&[
            ("__CFBundleIdentifier", "com.anthropic.claudefordesktop"),
            ("CLAUDE_CODE_ENTRYPOINT", "claude-desktop"),
            ("CLAUDE_CODE_SESSION_ID", "093b2b24"),
            ("CLAUDE_CODE_HOST_SESSION_ID", "local_093b2b24"),
            ("CLAUDE_CODE_SESSION_ATTENDED", "0"),
            ("CLAUDE_CODE_EXECPATH", "/Apps/claude"),
            ("CLAUDE_CODE_MESSAGING_TOKEN", "SECRET-TOKEN"),
            ("CLAUDE_CODE_MESSAGING_SOCKET", "/tmp/cc-socks/1.sock"),
        ]);
        let mut p = base();
        augment_source(&mut p, &env, Some("/w".into()), &RoutineFlags::default());
        let s = &p["source"];
        assert_eq!(s["host"], "claude_desktop");
        assert_eq!(s["conversation_id"], "093b2b24");
        assert_eq!(s["host_session_id"], "local_093b2b24");
        assert_eq!(s["attended"], false);
        assert_eq!(s["cli_path"], "/Apps/claude");
        let text = p.to_string();
        assert!(!text.contains("SECRET-TOKEN") && !text.contains("cc-socks"), "不得帶出任何秘密：{text}");
    }

    /// agent 自填的值優先。
    #[test]
    fn agent_supplied_identity_wins() {
        let env = env_of(&[("CLAUDE_CODE_SESSION_ID", "from-env"), ("CLAUDE_CODE_ENTRYPOINT", "cli")]);
        let mut p = json!({"id": "x", "source": {"agent": "claude", "cwd": "/w", "conversation_id": "mine"}});
        augment_source(&mut p, &env, None, &RoutineFlags::default());
        assert_eq!(p["source"]["conversation_id"], "mine");
        assert_eq!(p["source"]["host"], "claude_cli");
    }

    /// AC-CC-02：Codex App 帶 thread id；缺 source 時自建並推得 agent 與 cwd。
    #[test]
    fn codex_thread_identity_and_missing_source() {
        let env = env_of(&[("CODEX_THREAD_ID", "01a1-t"), ("__CFBundleIdentifier", "com.openai.codex")]);
        let mut p = json!({"id": "x"});
        augment_source(&mut p, &env, Some("/repo".into()), &RoutineFlags::default());
        assert_eq!(p["source"]["agent"], "codex");
        assert_eq!(p["source"]["cwd"], "/repo");
        assert_eq!(p["source"]["host"], "codex_app");
        assert_eq!(p["source"]["conversation_id"], "01a1-t");
        assert!(p["source"].get("host_session_id").is_none());
    }

    /// 沒有任何 agent 環境 → 不亂補。
    #[test]
    fn plain_terminal_adds_nothing() {
        let env = env_of(&[]);
        let mut p = base();
        augment_source(&mut p, &env, Some("/w".into()), &RoutineFlags::default());
        assert_eq!(p, base());
    }

    /// routine-check-in 場景 1：--routine 旗標帶入排程身分。
    #[test]
    fn routine_flags_mark_scheduled_check_in() {
        let args: Vec<String> = ["--json", "-", "--routine", "daily-journal", "--routine-name", "每日日記", "--every", "24"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let flags = routine_flags(&args).unwrap();
        assert_eq!(flags.id.as_deref(), Some("daily-journal"));
        let env = env_of(&[("CLAUDE_CODE_SESSION_ID", "s1"), ("CLAUDE_CODE_ENTRYPOINT", "claude-desktop")]);
        let mut p = base();
        augment_source(&mut p, &env, None, &flags);
        let s = &p["source"];
        assert_eq!(s["routine_id"], "daily-journal");
        assert_eq!(s["routine_name"], "每日日記");
        assert_eq!(s["expected_every_hours"], 24.0);
        assert_eq!(s["trigger"], "scheduled");
        assert_eq!(s["routine_kind"], "claude_scheduled_task");
    }

    /// AC-RC-01b：`--every` 解析不成有限正數就報錯，不靜默變成「未宣告頻率」。
    #[test]
    fn routine_every_must_be_a_positive_number() {
        let flags = |every: &str| {
            let args: Vec<String> = ["--json", "-", "--routine", "d", "--every", every].iter().map(|s| s.to_string()).collect();
            routine_flags(&args)
        };
        for bad in ["24h", "-24", "0", "NaN", "inf", ""] {
            assert!(flags(bad).is_err(), "--every {bad:?} 應報錯");
        }
        assert_eq!(flags("1.5").unwrap().every_hours, Some(1.5));
        let none: Vec<String> = ["--json", "-"].iter().map(|s| s.to_string()).collect();
        assert_eq!(routine_flags(&none).unwrap().every_hours, None);
    }

    /// AC-RC-01b：旗標沒給值、或沒搭配 --routine 都報錯，不靜默忽略。
    #[test]
    fn routine_flags_need_values_and_a_routine() {
        let parse = |args: &[&str]| routine_flags(&args.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert!(parse(&["--json", "-", "--routine", "diary", "--every"]).is_err(), "--every 沒給值");
        assert!(parse(&["--json", "-", "--routine", "diary", "--routine-name"]).is_err(), "--routine-name 沒給值");
        assert!(parse(&["--json", "-", "--routine", "--every", "24"]).is_err(), "--routine 的值不能是另一個旗標");
        assert!(parse(&["--json", "-", "--every", "24"]).is_err(), "--every 要搭配 --routine");
        assert!(parse(&["--json", "-", "--routine-name", "日記"]).is_err(), "--routine-name 要搭配 --routine");
        assert!(parse(&["--json", "-", "--routine", "diary", "--routine-name", "日記", "--every", "24"]).is_ok());
    }

    /// AC-RC-01b（QA 督導第 3 輪）：`kunsu ask --json` 拒絕不認得的旗標（含 `--every=24` 與拼錯的旗標）與
    /// 保留前綴 `kunsu-` 的排程 id，不靜默丟掉頻率。
    #[test]
    fn ask_json_rejects_unknown_flags_and_reserved_routines() {
        let check = |args: &[&str]| ask_json_flags_ok(&args.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert!(check(&["--json", "q.json", "--routine", "diary", "--every=24"]).is_err(), "--every=24");
        assert!(check(&["--json", "q.json", "--routine", "diary", "--evry", "24"]).is_err(), "拼錯");
        assert!(check(&["--json", "q.json", "--routine", "kunsu-x"]).is_err(), "保留前綴");
        assert!(check(&["--json", "q.json", "--routine", "diary", "--routine-name", "日記", "--every", "24", "--routine-kind", "other"]).is_ok());
        assert!(check(&["--json", "-"]).is_ok());
    }

    /// AC-LP-05：外掛或直接連線須真的 enabled，停用舊連線不被誤判已接線。
    #[test]
    fn doctor_recognizes_enabled_local_plugin_or_direct_mcp() {
        let check = |raw: &str| codex_kunsu_registered(&raw.parse::<toml::Table>().unwrap());
        assert!(check("[plugins.\"kunsu@personal\"]\nenabled = true"));
        assert!(!check("[plugins.\"kunsu@personal\"]\nenabled = false"));
        assert!(check("[mcp_servers.kunsu]\ncommand = \"python3\""));
        assert!(!check("[mcp_servers.kunsu]\nenabled = false"));
        assert!(!check("[plugins.\"other@personal\"]\nenabled = true"));
    }

    /// doctor 的 tool 數必須跟 server 實際提供的一致，否則每次加 tool 都會誤判回滾。
    #[test]
    fn doctor_expects_the_real_mcp_surface() {
        assert_eq!(EXPECTED_MCP_TOOLS, kunsu_server::tool_specs().len());
    }

    /// AC-CC-04：`kunsu wait --timeout 0` 無總時限。
    #[test]
    fn wait_timeout_zero_means_forever() {
        let args = |t: &str| vec!["pa-x".to_string(), "--timeout".to_string(), t.to_string()];
        assert_eq!(wait_total(&args("0")), None);
        assert_eq!(wait_total(&args("120")), Some(120));
        assert_eq!(wait_total(&["pa-x".to_string()]), Some(600));
    }
}
