//! Mac 端手機橋接（BDD docs/bdd/mobile-pairing.md AC-MP-02、03、05；INVARIANTS 7、15、28、29）。
//! 不連真的中轉站：直接把中轉站的 JSON 訊框餵給橋接、檢查它要送出的訊框。

mod common;

use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;

use common::*;
use kunsu_core::types::{Channel, State};
use kunsu_mobile::{DeviceKeys, Envelope, PairAccepted, parse_qr_payload};
use kunsu_server::mobile::MobileBridge;
use kunsu_server::StaticKeyProvider;
use rand_core::OsRng;
use serde_json::{json, Value};
use tempfile::TempDir;

const RELAY: &str = "wss://relay.test/v1";

struct Phone {
    keys: DeviceKeys,
    mac_kx: [u8; 32],
    mac_sig: [u8; 32],
    seq: u64,
}

impl Phone {
    fn frame(&mut self, body: Value, ts: i64) -> String {
        self.seq += 1;
        let env = self.keys.seal(&self.mac_kx, self.seq, ts, body.to_string().as_bytes(), rand_nonce());
        json!({"from": self.keys.device_id(), "body": env}).to_string()
    }
    /// 橋接送出的訊框 → 解開成 JSON。
    fn read(&self, frame: &str) -> Value {
        let v: Value = serde_json::from_str(frame).unwrap();
        assert_eq!(v["to"], self.keys.device_id());
        let env: Envelope = serde_json::from_value(v["body"].clone()).unwrap();
        let body = self.keys.open_from_mac(&self.mac_kx, &self.mac_sig, &env).unwrap();
        serde_json::from_slice(&body).unwrap()
    }
}

fn rand_nonce() -> [u8; 12] {
    use rand_core::RngCore;
    let mut n = [0u8; 12];
    OsRng.fill_bytes(&mut n);
    n
}

fn unix(t: chrono::DateTime<chrono::Utc>) -> i64 { t.timestamp() }

/// 建好橋接並完成配對（含主視窗按允許）。
async fn paired(ctx: &TestCtx, dir: &TempDir) -> (MobileBridge, Phone) {
    let bridge = MobileBridge::load(ctx.handle.clone(), Arc::new(StaticKeyProvider(test_key())), dir.path(), RELAY).unwrap();
    let qr = bridge.start_pairing(unix(now()));
    assert!(qr.svg.contains("<svg"));
    let p = parse_qr_payload(&qr.payload).unwrap();
    assert_eq!(p.relay, RELAY);
    let keys = DeviceKeys::from_seed([3; 32], [4; 32]);
    let wire = keys.pair_request(&p.kx, &p.mac, &p.code, "Alex 的 iPhone", unix(now()), &mut OsRng);
    let frame = json!({"from": keys.device_id(), "body": wire}).to_string();
    let out = bridge.handle_relay_message(&frame, now()).await;
    assert!(out.is_empty(), "按允許之前什麼都不回");
    let status = bridge.status();
    assert_eq!(status.pending.len(), 1);
    assert!(status.devices.is_empty(), "INV-29：還沒按允許，裝置清單是空的");
    let out = bridge.decide_pairing(&status.pending[0].id, true, now()).await.unwrap();
    assert_eq!(out.len(), 1);
    let v: Value = serde_json::from_str(&out[0]).unwrap();
    let accepted: PairAccepted = serde_json::from_value(v["body"].clone()).unwrap();
    assert!(keys.verify_pair_accepted(&p.sig, &accepted));
    assert_eq!(bridge.status().devices.len(), 1);
    (bridge, Phone { keys, mac_kx: p.kx, mac_sig: p.sig, seq: 0 })
}

#[tokio::test]
async fn keys_and_devices_live_in_files_only_ray_can_read() {
    let ctx = TestCtx::new().await;
    let dir = TempDir::new().unwrap();
    let _ = paired(&ctx, &dir).await;
    for f in ["mobile-keys.json", "mobile-devices.json"] {
        let mode = std::fs::metadata(dir.path().join(f)).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "{f}");
    }
    // 重新載入：同一把 Mac 金鑰、裝置還在
    let again = MobileBridge::load(ctx.handle.clone(), Arc::new(StaticKeyProvider(test_key())), dir.path(), RELAY).unwrap();
    assert_eq!(again.status().devices.len(), 1);
}

#[tokio::test]
async fn rejected_pairing_adds_nothing() {
    let ctx = TestCtx::new().await;
    let dir = TempDir::new().unwrap();
    let bridge = MobileBridge::load(ctx.handle.clone(), Arc::new(StaticKeyProvider(test_key())), dir.path(), RELAY).unwrap();
    let p = parse_qr_payload(&bridge.start_pairing(unix(now())).payload).unwrap();
    let keys = DeviceKeys::from_seed([3; 32], [4; 32]);
    let wire = keys.pair_request(&p.kx, &p.mac, &p.code, "x", unix(now()), &mut OsRng);
    bridge.handle_relay_message(&json!({"from": keys.device_id(), "body": wire}).to_string(), now()).await;
    let id = bridge.status().pending[0].id.clone();
    let out = bridge.decide_pairing(&id, false, now()).await.unwrap();
    let v: Value = serde_json::from_str(&out[0]).unwrap();
    assert_eq!(v["body"]["t"], "pair_rejected");
    assert!(bridge.status().devices.is_empty());
}

#[tokio::test]
async fn sync_returns_pending_questions_and_mobile_answer_closes_them() {
    let ctx = TestCtx::new().await;
    ctx.handle.create_question(base_req("q-plain"), now()).await.unwrap();
    let dir = TempDir::new().unwrap();
    let (bridge, mut phone) = paired(&ctx, &dir).await;

    let out = bridge.handle_relay_message(&phone.frame(json!({"type": "sync"}), unix(now())), now()).await;
    let snap = phone.read(&out[0]);
    assert_eq!(snap["type"], "snapshot");
    let q = snap["questions"].as_array().unwrap().iter().find(|q| q["id"] == "q-plain").unwrap().clone();
    assert!(!q["options"].as_array().unwrap().is_empty());

    let choice = q["options"][0]["id"].as_str().unwrap().to_string();
    let out = bridge
        .handle_relay_message(&phone.frame(json!({"type": "answer", "question_id": "q-plain", "choice_id": choice}), unix(now())), now())
        .await;
    let ack = phone.read(&out[0]);
    assert_eq!(ack["type"], "ack");
    assert_eq!(ack["ok"], true, "{ack}");
    let q = ctx.handle.get_question("q-plain").await.unwrap().unwrap();
    assert_eq!(q.state, State::Answered);
    assert_eq!(q.answer.unwrap().channel, Channel::Mobile);
}

#[tokio::test]
async fn money_needs_the_confirm_sheet() {
    let ctx = TestCtx::new().await;
    ctx.handle.create_question(money_t0_req("q-money"), now()).await.unwrap();
    let dir = TempDir::new().unwrap();
    let (bridge, mut phone) = paired(&ctx, &dir).await;

    let out = bridge
        .handle_relay_message(&phone.frame(json!({"type": "answer", "question_id": "q-money", "choice_id": "a"}), unix(now())), now())
        .await;
    let ack = phone.read(&out[0]);
    assert_eq!(ack["ok"], false);
    assert!(ack["error"].as_str().unwrap().contains("核對"), "{ack}");
    assert_eq!(ctx.handle.get_question("q-money").await.unwrap().unwrap().state, State::Pending);

    let out = bridge
        .handle_relay_message(
            &phone.frame(json!({"type": "answer", "question_id": "q-money", "choice_id": "a", "money_confirmed": true}), unix(now())),
            now(),
        )
        .await;
    assert_eq!(phone.read(&out[0])["ok"], true);
    let q = ctx.handle.get_question("q-money").await.unwrap().unwrap();
    assert_eq!(q.state, State::Answered);
    let a = q.answer.unwrap();
    assert_eq!(a.channel, Channel::Mobile);
    assert!(a.signature.is_some(), "手機批准的錢題同樣帶簽章，錢鏈才會走");
}

#[tokio::test]
async fn inv_mobile_envelope_verified_by_the_bridge() {
    let ctx = TestCtx::new().await;
    ctx.handle.create_question(base_req("q-r"), now()).await.unwrap();
    let dir = TempDir::new().unwrap();
    let (bridge, mut phone) = paired(&ctx, &dir).await;
    let frame = phone.frame(json!({"type": "answer", "question_id": "q-r", "choice_id": "a"}), unix(now()) - 500);
    assert!(bridge.handle_relay_message(&frame, now()).await.is_empty(), "時間差太大：不回、不作答");
    assert_eq!(ctx.handle.get_question("q-r").await.unwrap().unwrap().state, State::Pending);

    let frame = phone.frame(json!({"type": "sync"}), unix(now()));
    assert_eq!(bridge.handle_relay_message(&frame, now()).await.len(), 1);
    assert!(bridge.handle_relay_message(&frame, now()).await.is_empty(), "重放：不回");

    let id = bridge.status().devices[0].id.clone();
    bridge.revoke(&id).unwrap();
    let frame = phone.frame(json!({"type": "answer", "question_id": "q-r", "choice_id": "a"}), unix(now()));
    let out = bridge.handle_relay_message(&frame, now()).await;
    assert_eq!(out.len(), 1, "已撤銷：只回一則明文通知，請它重新配對");
    let v: Value = serde_json::from_str(&out[0]).unwrap();
    assert_eq!(v["body"]["t"], "revoked");
    assert_eq!(ctx.handle.get_question("q-r").await.unwrap().unwrap().state, State::Pending);
}

#[tokio::test]
async fn relay_agent_setting_is_shared_with_the_main_window() {
    let ctx = TestCtx::new().await;
    let dir = TempDir::new().unwrap();
    let (bridge, mut phone) = paired(&ctx, &dir).await;
    assert_eq!(bridge.status().relay_agent, "claude");
    let out = bridge.handle_relay_message(&phone.frame(json!({"type": "settings", "relay_agent": "codex"}), unix(now())), now()).await;
    assert_eq!(phone.read(&out[0])["ok"], true);
    assert_eq!(bridge.status().relay_agent, "codex");
    bridge.set_relay_agent("claude").unwrap();
    assert_eq!(bridge.status().relay_agent, "claude");
}

// ---------------------------------------------------------------------- 中轉 AI、追問、問本人、開新任務
// BDD docs/bdd/mobile-relay-ai.md；INVARIANTS 31、32。

use std::sync::Mutex as StdMutex;
use std::time::Duration as StdDuration;

use kunsu_server::mcp::{handle_rpc, McpReply};
use kunsu_server::relay_ai::{self, AiRunner, PlannedRun, RunFuture};

/// 假的中轉 AI：記下每次要跑的指令，回固定結果。
struct FakeRunner {
    runs: StdMutex<Vec<PlannedRun>>,
    reply: Result<String, String>,
}

impl FakeRunner {
    fn ok(text: &str) -> Arc<Self> { Arc::new(Self { runs: StdMutex::new(vec![]), reply: Ok(text.into()) }) }
    fn failing(err: &str) -> Arc<Self> { Arc::new(Self { runs: StdMutex::new(vec![]), reply: Err(err.into()) }) }
    fn runs(&self) -> Vec<PlannedRun> { self.runs.lock().unwrap().clone() }
}

impl AiRunner for FakeRunner {
    fn run(&self, plan: PlannedRun, _timeout: StdDuration) -> RunFuture {
        self.runs.lock().unwrap().push(plan);
        let r = self.reply.clone();
        Box::pin(async move { r })
    }
}

async fn wired(ctx: &TestCtx, dir: &TempDir, runner: Arc<FakeRunner>) -> (MobileBridge, Phone, tokio::sync::mpsc::UnboundedReceiver<String>) {
    let (bridge, phone) = paired(ctx, dir).await;
    bridge.set_runner(runner);
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    bridge.attach_outbox(tx);
    (bridge, phone, rx)
}

/// 等橋接從背景送出的下一個訊框（解開後）。
async fn next(phone: &Phone, rx: &mut tokio::sync::mpsc::UnboundedReceiver<String>, ty: &str) -> Value {
    loop {
        let f = tokio::time::timeout(StdDuration::from_secs(5), rx.recv()).await.expect("等不到橋接送出的訊框").unwrap();
        let v = phone.read(&f);
        if v["type"] == ty {
            return v;
        }
    }
}

async fn mcp(handle: &kunsu_server::ServerHandle, name: &str, arguments: Value) -> Result<Value, Value> {
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": name, "arguments": arguments}});
    match handle_rpc(handle, body).await {
        McpReply::Json(v) if v.get("error").is_some() => Err(v["error"].clone()),
        McpReply::Json(v) => Ok(v["result"]["structuredContent"].clone()),
        McpReply::Accepted => panic!("tools/call 不得回 Accepted"),
    }
}

fn question_in(id: &str, cwd: &std::path::Path) -> kunsu_core::types::CreateRequest {
    let mut req = base_req(id);
    req.source.cwd = cwd.to_string_lossy().to_string();
    req
}

#[tokio::test]
async fn ac_ra_01_followup_goes_to_relay_ai_with_question_context() {
    let ctx = TestCtx::new().await;
    let dir = TempDir::new().unwrap();
    ctx.handle.create_question(base_req("q-f"), now()).await.unwrap();
    let before = ctx.handle.get_question("q-f").await.unwrap().unwrap();
    let runner = FakeRunner::ok("因為上週轉換掉了一半。");
    let (bridge, mut phone, mut rx) = wired(&ctx, &dir, runner.clone()).await;

    let out = bridge.handle_relay_message(&phone.frame(json!({"type": "followup", "id": "f1", "question_id": "q-f", "kind": "why"}), unix(now())), now()).await;
    assert!(out.is_empty(), "追問在背景跑，不卡住中轉站連線");
    let v = next(&phone, &mut rx, "followup_answer").await;
    assert_eq!(v["id"], "f1");
    assert_eq!(v["ok"], true);
    assert_eq!(v["by"], "relay");
    assert_eq!(v["text"], "因為上週轉換掉了一半。");

    let runs = runner.runs();
    assert_eq!(runs.len(), 1);
    let prompt = &runs[0].stdin;
    assert!(prompt.contains(&before.title) || prompt.contains(&before.question_line), "提示要有題目：{prompt}");
    for o in &before.options {
        assert!(prompt.contains(&o.label), "提示要有選項 {}", o.label);
    }
    assert!(prompt.contains("為什麼"), "追問種類要寫進提示");

    // 第二次追問：提示帶著前一次的問答
    bridge.handle_relay_message(&phone.frame(json!({"type": "followup", "id": "f2", "question_id": "q-f", "kind": "text", "text": "那如果不做呢？"}), unix(now())), now()).await;
    next(&phone, &mut rx, "followup_answer").await;
    let prompt2 = runner.runs()[1].stdin.clone();
    assert!(prompt2.contains("那如果不做呢？") && prompt2.contains("因為上週轉換掉了一半。"));

    let after = ctx.handle.get_question("q-f").await.unwrap().unwrap();
    assert_eq!(after.state, before.state, "追問不改題目");
    assert!(after.answer.is_none());
}

#[tokio::test]
async fn ac_ra_02_relay_ai_failure_is_a_readable_error() {
    let ctx = TestCtx::new().await;
    let dir = TempDir::new().unwrap();
    ctx.handle.create_question(base_req("q-e"), now()).await.unwrap();
    let (bridge, mut phone, mut rx) = wired(&ctx, &dir, FakeRunner::failing("軍師沒有回應（逾時 180 秒）")).await;
    bridge.handle_relay_message(&phone.frame(json!({"type": "followup", "id": "f1", "question_id": "q-e", "kind": "context"}), unix(now())), now()).await;
    let v = next(&phone, &mut rx, "followup_answer").await;
    assert_eq!(v["ok"], false);
    assert_eq!(v["error"], "軍師沒有回應（逾時 180 秒）");
    bridge.handle_relay_message(&phone.frame(json!({"type": "chat", "id": "c1", "text": "嗨"}), unix(now())), now()).await;
    let v = next(&phone, &mut rx, "chat_reply").await;
    assert_eq!(v["ok"], false);
    assert!(v["error"].as_str().unwrap().contains("逾時"));
}

#[tokio::test]
async fn ac_ra_03_forward_reaches_the_waiting_agent_and_reply_comes_back() {
    let ctx = TestCtx::new().await;
    let dir = TempDir::new().unwrap();
    ctx.handle.create_question(base_req("q-w"), now()).await.unwrap();
    let (bridge, mut phone, mut rx) = wired(&ctx, &dir, FakeRunner::ok("不會用到")).await;

    let h = ctx.handle.clone();
    let waiter = tokio::spawn(async move { mcp(&h, "ask_wait", json!({"question_id": "q-w", "timeout_seconds": 30})).await.unwrap() });
    tokio::time::sleep(StdDuration::from_millis(100)).await;

    let out = bridge.handle_relay_message(&phone.frame(json!({"type": "forward", "id": "f9", "question_id": "q-w", "text": "這個預算是含稅嗎？"}), unix(now())), now()).await;
    assert!(out.is_empty(), "原 AI 在等：收下，不立刻回");
    let got = tokio::time::timeout(StdDuration::from_secs(5), waiter).await.unwrap().unwrap();
    assert_eq!(got["status"], "followup");
    assert_eq!(got["followup"]["id"], "f9");
    assert_eq!(got["followup"]["text"], "這個預算是含稅嗎？");
    assert!(got["hint"].as_str().unwrap().contains("ask_followup_reply"));

    // 同一則只交一次：再等就是 pending_timeout
    let again = mcp(&ctx.handle, "ask_wait", json!({"question_id": "q-w", "timeout_seconds": 1})).await.unwrap();
    assert_eq!(again["status"], "pending_timeout");

    mcp(&ctx.handle, "ask_followup_reply", json!({"question_id": "q-w", "followup_id": "f9", "text": "含稅，總共 5,250。"})).await.unwrap();
    let v = next(&phone, &mut rx, "followup_answer").await;
    assert_eq!(v["id"], "f9");
    assert_eq!(v["ok"], true);
    assert_eq!(v["by"], "agent");
    assert_eq!(v["text"], "含稅，總共 5,250。");
    let q = ctx.handle.get_question("q-w").await.unwrap().unwrap();
    assert!(q.answer.is_none() && !q.state.is_terminal(), "問本人不會變成答案");
}

#[tokio::test]
async fn ac_ra_04_forward_without_waiter_is_refused_and_silence_times_out() {
    let ctx = TestCtx::new().await;
    let dir = TempDir::new().unwrap();
    ctx.handle.create_question(base_req("q-n"), now()).await.unwrap();
    let (bridge, mut phone, _rx) = wired(&ctx, &dir, FakeRunner::ok("x")).await;
    let out = bridge.handle_relay_message(&phone.frame(json!({"type": "forward", "id": "f1", "question_id": "q-n", "text": "在嗎"}), unix(now())), now()).await;
    assert_eq!(out.len(), 1);
    let v = phone.read(&out[0]);
    assert_eq!(v["type"], "followup_answer");
    assert_eq!(v["ok"], false);
    assert!(v["error"].as_str().unwrap().contains("沒在等這題"), "{v}");

    // 在等，但 15 分鐘都沒回
    ctx.handle.waiter_enter("q-n", now());
    let out = bridge.handle_relay_message(&phone.frame(json!({"type": "forward", "id": "f2", "question_id": "q-n", "text": "在嗎"}), unix(now())), now()).await;
    assert!(out.is_empty());
    assert!(bridge.expire_followups(now() + chrono::Duration::minutes(14)).is_empty());
    let out = bridge.expire_followups(now() + chrono::Duration::minutes(15));
    assert_eq!(out.len(), 1);
    let v = phone.read(&out[0]);
    assert_eq!(v["id"], "f2");
    assert_eq!(v["ok"], false);
    assert!(v["error"].as_str().unwrap().contains("15 分鐘"));
    // 逾時後原 AI 不會再拿到這則
    let w = mcp(&ctx.handle, "ask_wait", json!({"question_id": "q-n", "timeout_seconds": 1})).await.unwrap();
    assert_eq!(w["status"], "pending_timeout");
}

#[tokio::test]
async fn ac_ra_06_chat_has_state_summary_and_recent_history() {
    let ctx = TestCtx::new().await;
    let dir = TempDir::new().unwrap();
    ctx.handle.create_question(base_req("q-open"), now()).await.unwrap();
    let open_title = ctx.handle.get_question("q-open").await.unwrap().unwrap().title;
    let runner = FakeRunner::ok("好");
    let (bridge, mut phone, mut rx) = wired(&ctx, &dir, runner.clone()).await;
    let names = ["甲甲", "乙乙", "丙丙", "丁丁", "戊戊", "己己", "庚庚", "辛辛"];
    for (i, n) in names.iter().enumerate() {
        bridge.handle_relay_message(&phone.frame(json!({"type": "chat", "id": format!("c{i}"), "text": n}), unix(now())), now()).await;
        let v = next(&phone, &mut rx, "chat_reply").await;
        assert_eq!(v["id"], format!("c{i}"));
        assert_eq!(v["ok"], true);
    }
    let last = runner.runs().last().unwrap().stdin.clone();
    assert!(last.contains(&open_title), "現況摘要要有待決題");
    assert!(last.contains("辛辛"));
    // 第 8 句之前最近 12 則 = 第 3～7 句的問答＋…；第 1、2 句已經掉出去
    assert!(!last.contains("甲甲") && !last.contains("乙乙"), "{last}");
    assert!(last.contains("丙丙"));
}

#[tokio::test]
async fn ac_ra_07_start_task_runs_only_in_known_projects_with_auto_permission() {
    let ctx = TestCtx::new().await;
    let dir = TempDir::new().unwrap();
    let proj = TempDir::new().unwrap();
    let name = proj.path().file_name().unwrap().to_string_lossy().to_string();
    ctx.handle.create_question(question_in("q-p", proj.path()), now()).await.unwrap();
    let runner = FakeRunner::ok("做完了：改了 2 個檔。");
    let (bridge, mut phone, mut rx) = wired(&ctx, &dir, runner.clone()).await;

    let sync = phone.frame(json!({"type": "sync"}), unix(now()));
    let snap = phone.read(&bridge.handle_relay_message(&sync, now()).await[0]);
    assert!(snap["projects"].as_array().unwrap().iter().any(|p| p == &json!(name)), "{snap}");

    let out = bridge.handle_relay_message(&phone.frame(json!({"type": "start_task", "id": "t0", "agent": "claude", "project": "不存在的專案", "text": "x"}), unix(now())), now()).await;
    let v = phone.read(&out[0]);
    assert_eq!(v["type"], "task_started");
    assert_eq!(v["ok"], false);
    assert!(runner.runs().is_empty(), "不在清單裡的專案不跑");

    for (agent, id) in [("claude", "t1"), ("codex", "t2")] {
        let out = bridge.handle_relay_message(&phone.frame(json!({"type": "start_task", "id": id, "agent": agent, "project": name, "text": "把 README 補齊"}), unix(now())), now()).await;
        let v = phone.read(&out[0]);
        assert_eq!(v["type"], "task_started");
        assert_eq!(v["ok"], true, "{v}");
        let done = next(&phone, &mut rx, "task_done").await;
        assert_eq!(done["id"], id);
        assert_eq!(done["text"], "做完了：改了 2 個檔。");
    }
    let runs = runner.runs();
    assert_eq!(runs.len(), 2);
    for r in &runs {
        assert_eq!(r.cwd, proj.path());
        assert!(r.stdin.contains("把 README 補齊") || r.args.iter().any(|a| a.contains("把 README 補齊")));
    }
    assert!(runs[0].args.windows(2).any(|w| w == ["--permission-mode", "auto"]), "Claude 用 Auto：{:?}", runs[0].args);
    assert!(runs[1].args.iter().any(|a| a == "--approve-for-me"), "Codex 用 Auto：{:?}", runs[1].args);
    for r in &runs {
        assert!(!r.args.iter().any(|a| a.contains("bypass") || a.contains("dangerously")), "不提供繞過權限");
    }
}

/// INV-31：追問回覆只是文字。
#[tokio::test]
async fn inv_followup_reply_never_answers() {
    let ctx = TestCtx::new().await;
    ctx.handle.create_question(base_req("q-i"), now()).await.unwrap();
    let before = ctx.handle.get_question("q-i").await.unwrap().unwrap();
    ctx.handle.waiter_enter("q-i", now());
    ctx.handle.followup_ask("q-i", "f1", "多少錢？", now()).unwrap();
    let w = mcp(&ctx.handle, "ask_wait", json!({"question_id": "q-i", "timeout_seconds": 1})).await.unwrap();
    assert_eq!(w["status"], "followup");
    mcp(&ctx.handle, "ask_followup_reply", json!({"question_id": "q-i", "followup_id": "f1", "text": "選 A 吧"})).await.unwrap();
    // 沒有這則追問：拒絕
    assert!(mcp(&ctx.handle, "ask_followup_reply", json!({"question_id": "q-i", "followup_id": "nope", "text": "x"})).await.is_err());
    let after = ctx.handle.get_question("q-i").await.unwrap().unwrap();
    assert!(after.answer.is_none());
    assert_eq!(after.state, before.state);
    assert_eq!(after.cycle_number, before.cycle_number);
    assert_eq!(after.cycle_started_at, before.cycle_started_at);
}

/// INV-32：中轉 AI 沒有 MCP、只能讀。
#[test]
fn inv_relay_ai_read_only() {
    let dir = std::path::Path::new("/tmp");
    for agent in [relay_ai::RelayAgent::Claude, relay_ai::RelayAgent::Codex] {
        let p = relay_ai::relay_command(agent, dir, "提示", std::path::Path::new("/bin/claude"), std::path::Path::new("/bin/codex"), std::path::Path::new("/tmp/out.txt"));
        assert!(p.stdin.ends_with("提示"), "提示從 stdin 給");
        let a = p.args.join(" ");
        assert!(!a.contains("bypass") && !a.contains("dangerously") && !a.contains("full-auto") && !a.contains("approve-for-me"), "{a}");
        match agent {
            relay_ai::RelayAgent::Claude => {
                assert!(p.args.iter().any(|x| x == "--strict-mcp-config"), "{a}");
                assert!(p.args.windows(2).any(|w| w[0] == "--mcp-config" && w[1].replace(' ', "") == r#"{"mcpServers":{}}"#), "{a}");
                assert!(p.args.windows(2).any(|w| w == ["--allowedTools", "Read Grep Glob"]), "{a}");
                assert!(p.args.windows(2).any(|w| w == ["--permission-mode", "dontAsk"]), "{a}");
                assert!(!a.contains("Bash") && !a.contains("Edit") && !a.contains("Write"), "{a}");
            }
            relay_ai::RelayAgent::Codex => {
                assert!(p.args.windows(2).any(|w| w == ["--sandbox", "read-only"]), "{a}");
            }
        }
    }
    assert!(relay_ai::SYSTEM_PROMPT.contains("不能替使用者作答"));
}
