//! 手機端對端驗證台（不碰使用者真正的 Kunsu）：暫存資料庫＋一組新的 Mac 金鑰，連真的 Zeabur 中轉站，
//! 建兩題測試題、印出配對連結、自動允許配對、印出手機送來的答案。
//! 另外：一個暫存的「e2e-sandbox」專案（從手機開新任務只會在這裡跑）、一個假的原 AI 一直在等 e2e-plain，
//! 收到「問本人」就用 ask_followup_reply 回（中轉 AI 用真的 claude／codex）。
//!
//!   cargo run -p kunsu-server --example mobile_e2e -- [秒數]
//!
//! 需要 ~/.kunsu/relay-token（Mac 上線中轉站用的 token）。

#[path = "../tests/common/mod.rs"]
#[allow(dead_code)]
mod common;

use std::sync::Arc;
use std::time::Duration;

use kunsu_server::mobile::{MobileBridge, DEFAULT_RELAY_URL};
use kunsu_server::StaticKeyProvider;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt().with_writer(std::io::stderr).with_env_filter("kunsu_server=info").init();
    let secs: u64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(300);
    let ctx = common::TestCtx::new().await;
    let now = chrono::Utc::now();
    let mut plain = common::base_req("e2e-plain");
    plain.question_line = "端對端測試：要吸附成 L 形嗎？".into();
    plain.source.conversation_id = Some("e2e-conversation".into()); // 有原對話，手機才有「問本人」
    ctx.handle.create_question(plain, now).await.expect("plain question");
    ctx.handle.create_question(common::money_t0_req("e2e-money"), common::now()).await.expect("money question");
    let sandbox = tempfile::Builder::new().prefix("e2e-sandbox").tempdir().unwrap();
    let mut sb = common::base_req("e2e-sandbox-q");
    sb.source.cwd = sandbox.path().to_string_lossy().to_string();
    ctx.handle.create_question(sb, now).await.expect("sandbox question");
    println!("SANDBOX {}", sandbox.path().display());

    // 假的原 AI：一直在等 e2e-plain，收到追問就回。
    let h = ctx.handle.clone();
    tokio::spawn(async move {
        use kunsu_server::mcp::{handle_rpc, McpReply};
        use serde_json::json;
        loop {
            let call = |name: &str, args: serde_json::Value| json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": name, "arguments": args}});
            let McpReply::Json(v) = handle_rpc(&h, call("ask_wait", json!({"question_id": "e2e-plain", "timeout_seconds": 30}))).await else { continue };
            let r = &v["result"]["structuredContent"];
            match r["status"].as_str() {
                Some("followup") => {
                    let fid = r["followup"]["id"].as_str().unwrap_or("").to_string();
                    let text = r["followup"]["text"].as_str().unwrap_or("").to_string();
                    println!("FOLLOWUP {fid} {text}");
                    handle_rpc(&h, call("ask_followup_reply", json!({"question_id": "e2e-plain", "followup_id": fid, "text": format!("（e2e 原 AI）收到你問「{text}」：我會吸附成 L 形，因為桌面版也是。")}))).await;
                }
                Some("pending_timeout") => {}
                _ => break,
            }
        }
    });

    let dir = tempfile::TempDir::new().unwrap();
    let home = std::env::var("HOME").unwrap();
    std::fs::copy(format!("{home}/.kunsu/relay-token"), dir.path().join("relay-token")).expect("~/.kunsu/relay-token");
    let bridge = MobileBridge::load(ctx.handle.clone(), Arc::new(StaticKeyProvider(common::test_key())), dir.path(), DEFAULT_RELAY_URL).unwrap();
    bridge.spawn();
    let qr = bridge.start_pairing(now.timestamp());
    std::fs::write("/tmp/kunsu-e2e-link.txt", &qr.payload).ok();
    println!("PAIR_LINK {}", qr.payload);

    let deadline = tokio::time::Instant::now() + Duration::from_secs(secs);
    let mut seen = std::collections::HashSet::new();
    while tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(500)).await;
        let s = bridge.status();
        if !s.connected {
            continue;
        }
        if seen.insert("connected") {
            println!("RELAY connected mac_id={}", s.mac_id);
        }
        for p in s.pending {
            println!("PAIR_REQUEST {} → 自動允許", p.device_name);
            bridge.decide_pairing(&p.id, true, chrono::Utc::now()).await.unwrap();
        }
        for id in ["e2e-plain", "e2e-money"] {
            if let Some(q) = ctx.handle.get_question(id).await.unwrap()
                && let Some(a) = q.answer
                && seen.insert(id)
            {
                println!("ANSWER {id} choice={:?} channel={:?} signed={}", a.choice_id, a.channel, a.signature.is_some());
            }
        }
        // 兩題都答完仍繼續跑到時間到：之後還要測聊天與開新任務
        if seen.contains("e2e-plain") && seen.contains("e2e-money") && seen.insert("done") {
            println!("DONE");
        }
    }
}
