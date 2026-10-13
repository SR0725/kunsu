//! 整合測試共用建構器。時間錨點與 kunsu-core 測試一致：台北週五 14:00
//! （quiet hours 之外，guards 全綠時不誤擋）。vault 與 DB 都在 tempdir。
#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use kunsu_core::policy::Policy;
use kunsu_core::time::Ts;
use kunsu_core::types::*;
use kunsu_server::*;
use tempfile::TempDir;
use tokio::sync::broadcast;

/// 2026-07-31T06:00:00Z = 台北 14:00（週五白天）。
pub const NOW_STR: &str = "2026-07-31T06:00:00Z";

pub fn ts(s: &str) -> Ts {
    DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
}

pub fn now() -> Ts {
    ts(NOW_STR)
}

pub struct TestCtx {
    pub handle: ServerHandle,
    pub discord: Arc<RecordingDiscord>,
    pub guards: Arc<StubGuardProbe>,
    pub db_path: PathBuf,
    pub vault: TempDir,
    _db_dir: TempDir,
}

impl TestCtx {
    pub async fn new() -> TestCtx {
        Self::with_policy(Policy::default()).await
    }

    pub async fn with_policy(policy: Policy) -> TestCtx {
        let vault = TempDir::new().unwrap();
        let db_dir = TempDir::new().unwrap();
        let db_path = db_dir.path().join("kunsu.db");
        let discord = Arc::new(RecordingDiscord::new());
        let guards = Arc::new(StubGuardProbe::new());
        let handle = ServerHandle::new(KunsuConfig {
            db_path: db_path.clone(),
            vault_root: vault.path().to_path_buf(),
            policy,
            discord: discord.clone(),
            guards: guards.clone(),
        })
        .await
        .unwrap();
        TestCtx { handle, discord, guards, db_path, vault, _db_dir: db_dir }
    }

    /// 模擬 app 重啟：同 DB、同 vault 開新 handle（timer 由 DB 重建）。
    pub async fn reopen(&self) -> ServerHandle {
        ServerHandle::new(KunsuConfig {
            db_path: self.db_path.clone(),
            vault_root: self.vault.path().to_path_buf(),
            policy: Policy::default(),
            discord: Arc::new(RecordingDiscord::new()),
            guards: Arc::new(StubGuardProbe::new()),
        })
        .await
        .unwrap()
    }

    /// 同 vault、全新空 DB 的 handle（rebuild_db 測試用）。
    pub async fn fresh_db_same_vault(&self) -> (ServerHandle, TempDir) {
        let db_dir = TempDir::new().unwrap();
        let handle = ServerHandle::new(KunsuConfig {
            db_path: db_dir.path().join("kunsu.db"),
            vault_root: self.vault.path().to_path_buf(),
            policy: Policy::default(),
            discord: Arc::new(RecordingDiscord::new()),
            guards: Arc::new(StubGuardProbe::new()),
        })
        .await
        .unwrap();
        (handle, db_dir)
    }
}

/// 合法 T1 choice 題（a=推薦執行、b=安全預設）。
pub fn base_req(id: &str) -> CreateRequest {
    CreateRequest {
        id: id.into(),
        title: "測試題".into(),
        question_line: "要不要加碼？".into(),
        body_md: "內文".into(),
        why_md: None,
        visual_symbol: None,
        kind: Kind::Choice,
        tier: Tier::T1,
        product: Some("nova".into()),
        gate: Some(Gate::Yellow),
        evidence: vec![EvidenceLine { text: "ROAS 3.2 連三日".into(), source: "meta-api".into() }],
        params_line: None,
        options: vec![
            Opt { id: "a".into(), label: "加碼 20%".into(), detail: None, recommended: Some(true) },
            Opt { id: "b".into(), label: "維持".into(), detail: None, recommended: None },
        ],
        allow_free_text: true,
        money: None,
        decision_ref: None,
        prediction_ref: None,
        kongko_ref: None,
        deep_link: None,
        irreversible: false,
        standing_policy_ref: None,
        internal: false,
        default_action: DefaultAction { on_expire: OnExpire::NoopLog, option_id: None },
        deadline_at: None,
        wake_on_answer: Wake::default(),
        source: Source {
            agent: AgentKind::Claude,
            session_id: None,
            cwd: "/Users/you/notes".into(),
            routine_id: Some("test-routine".into()),
            ..Default::default()
        },
        first_proposed: None,
        items: vec![],
    }
}

/// 測試錢題共用 entity（純數字，過 prepare cmd 白名單）。
pub const TEST_ENTITY: &str = "120000000000005";

/// 白名單合法的 prepare 命令（entity/金額/product 與 money_t0_req 對齊）。
pub fn whitelisted_prepare_cmd(entity: &str, budget: i64, product: &str) -> String {
    format!(
        "python3 projects/meta-ads/scripts/executor.py prepare-budget --entity {entity} --new-budget {budget} --product {product}"
    )
}

/// 通過 T0 准入判準的合法錢類 T0（P1 定案 2 六條件全齊）。
pub fn money_t0_req(id: &str) -> CreateRequest {
    let mut req = base_req(id);
    req.tier = Tier::T0;
    req.money = Some(Money {
        is_money_op: true,
        amount_twd: Some(600),
        entity_id: Some(TEST_ENTITY.into()),
        executor_prepare_cmd: Some(whitelisted_prepare_cmd(TEST_ENTITY, 600, "nova")),
    });
    req.prediction_ref = Some("projects/meta-ads/predictions/2026-07-31-test.md".into());
    req.params_line = Some(format!("adset {TEST_ENTITY} 日預算 500 → 600 (+20%)"));
    req.deadline_at = Some(now() + Duration::hours(6));
    req
}

pub fn ui_answer(choice: &str) -> AnswerInput {
    AnswerInput {
        choice_id: Some(choice.into()),
        free_text: None,
        edited_params: None,
        channel: Channel::Ui,
        signature: None,
        revision: None,
        content_hash: None,
        items: None,
    }
}

pub fn drain_events(rx: &mut broadcast::Receiver<ServerEvent>) -> Vec<ServerEvent> {
    let mut out = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        out.push(ev);
    }
    out
}

/// 讀當日（依 ts 台北日期）ledger jsonl 的所有行。
pub fn read_ledger_rows(ctx: &TestCtx, at: Ts) -> Vec<LedgerRow> {
    let path = ctx.handle.ledger().day_file(at);
    if !path.exists() {
        return Vec::new();
    }
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

pub fn read_open_pending(ctx: &TestCtx) -> Vec<Question> {
    ctx.handle.ledger().read_open().unwrap().map(|s| s.pending).unwrap_or_default()
}

// ---------------------------------------------------------------------------
// HTTP/協定層測試共用（本波新增）
// ---------------------------------------------------------------------------

/// 測試 Bearer token（32 hex，格式與 prod 生成一致）。
pub const TEST_TOKEN: &str = "cafebabecafebabecafebabecafebabe";
/// 測試 admin token（/cli/answer 與 /admin/* 專用；agent token 打不進）。
pub const TEST_ADMIN_TOKEN: &str = "adadadadadadadadadadadadadadadad";

/// 測試注入 HMAC key（32 bytes；prod 走 Keychain）。
pub fn test_key() -> Vec<u8> {
    b"kunsu-test-hmac-key-32-bytes!!!!".to_vec()
}

/// ephemeral port 起 HTTP server（auth middleware + 全路由）。回 base URL。
pub async fn spawn_http(ctx: &TestCtx) -> String {
    let state = kunsu_server::AppState {
        handle: ctx.handle.clone(),
        token: TEST_TOKEN.to_string(),
        admin_token: TEST_ADMIN_TOKEN.to_string(),
        key: Arc::new(kunsu_server::StaticKeyProvider(test_key())),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, kunsu_server::router(state)).await.unwrap();
    });
    format!("http://{addr}")
}

/// 手工簽出一份 approval record（測試 verify 端正反例用）。
pub fn signed_record(
    question_id: &str,
    executor_token: &str,
    param_hash: &str,
    answered_at_epoch: i64,
    nonce: &str,
) -> serde_json::Value {
    let mut rec = serde_json::Map::new();
    rec.insert("question_id".into(), serde_json::json!(question_id));
    rec.insert("executor_token".into(), serde_json::json!(executor_token));
    rec.insert("param_hash".into(), serde_json::json!(param_hash));
    rec.insert("answered_at".into(), serde_json::json!(answered_at_epoch));
    rec.insert("nonce".into(), serde_json::json!(nonce));
    kunsu_server::sign_record(&mut rec, &test_key()).unwrap();
    serde_json::Value::Object(rec)
}

/// 寫一個可執行的假 script（回傳路徑）。
pub fn write_script(dir: &std::path::Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, body).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    path
}

/// 測試共用 executor token（假 executor 印出）。
pub const FAKE_EXECUTOR_TOKEN: &str = "cafe0123deadbeef";

/// 在 vault 固定路徑放一個假 executor.py（run_approval_chain 只 spawn
/// `python3 <vault>/projects/meta-ads/scripts/executor.py …`）。
/// spec **由收到的 argv 誠實回填**（entity/new_budget/product 同 prepare 引數），
/// 讓所見即所簽比對在正常路徑通過；要測不一致就用 `write_lying_executor`。
pub fn write_fake_executor(ctx: &TestCtx) {
    let body = format!(
        r#"#!/usr/bin/env python3
import json, os, sys

def flag(name, default=None):
    args = sys.argv[1:]
    return args[args.index(name) + 1] if name in args else default

token = "{FAKE_EXECUTOR_TOKEN}"
os.makedirs("projects/meta-ads/data/executor_pending", exist_ok=True)
spec = {{
    "action": "update_budget",
    "entity": flag("--entity"),
    "product": flag("--product"),
    "new_budget": int(flag("--new-budget", "0")),
    "current_budget": "500",
    "tier": "yellow",
    "guards": [],
    "_token": token,
    "prepared_at": 1753900000.0,
}}
with open(f"projects/meta-ads/data/executor_pending/{{token}}.json", "w") as f:
    json.dump(spec, f, ensure_ascii=False)
print("=" * 60)
print("  ✅ 要核准請使用者在自己的 Terminal 貼上：")
print(f"     python3 projects/meta-ads/scripts/executor.py commit --token {{token}}")
print("=" * 60)
"#
    );
    let dir = ctx.vault.path().join("projects/meta-ads/scripts");
    std::fs::create_dir_all(&dir).unwrap();
    write_script(&dir, "executor.py", &body);
}

/// 說謊的假 executor：spec 寫死成指定 JSON（與 argv 無關），
/// 用來測所見即所簽比對必須擋下。
pub fn write_lying_executor(ctx: &TestCtx, spec_json: &str) {
    let body = format!(
        r#"#!/usr/bin/env python3
import os
token = "{FAKE_EXECUTOR_TOKEN}"
os.makedirs("projects/meta-ads/data/executor_pending", exist_ok=True)
with open(f"projects/meta-ads/data/executor_pending/{{token}}.json", "w") as f:
    f.write('''{spec_json}''')
print(f"     python3 projects/meta-ads/scripts/executor.py commit --token {{token}}")
"#
    );
    let dir = ctx.vault.path().join("projects/meta-ads/scripts");
    std::fs::create_dir_all(&dir).unwrap();
    write_script(&dir, "executor.py", &body);
}

/// verify 測試用：把手工簽的 record 登記進 approvals_issued（模擬簽章鏈簽發）。
pub async fn issue_record(ctx: &TestCtx, question_id: &str, record: &serde_json::Value) {
    let answer_id = ctx
        .handle
        .get_question(question_id)
        .await
        .unwrap()
        .and_then(|q| q.answer.map(|a| a.answer_id))
        .unwrap_or_else(|| "no-answer".to_string());
    ctx.handle
        .record_approval_issued(
            question_id,
            &answer_id,
            record["nonce"].as_str().unwrap(),
            record["executor_token"].as_str().unwrap(),
            record["param_hash"].as_str().unwrap(),
            record["answered_at"].as_i64().unwrap(),
            chrono::Utc::now(),
        )
        .await
        .unwrap();
}

/// 測試用：模擬「權限恢復、卡住的帳本寫入完成」。
/// 打開 FIFO 讀端（非阻塞），把所有卡在 open 的寫入者放行，收集它們寫出的行。
pub fn drain_fifo(path: &std::path::Path) -> Vec<String> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK).open(path).unwrap();
    let mut buf = Vec::new();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    let mut chunk = [0u8; 65536];
    while std::time::Instant::now() < deadline {
        match std::io::Read::read(&mut f, &mut chunk) {
            Ok(0) => std::thread::sleep(std::time::Duration::from_millis(20)),
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(std::time::Duration::from_millis(20))
            }
            Err(e) => panic!("讀 FIFO 失敗：{e}"),
        }
    }
    String::from_utf8_lossy(&buf).lines().filter(|l| !l.trim().is_empty()).map(str::to_string).collect()
}
