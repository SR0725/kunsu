//! Reactor + 簽章鏈 + inbox ingester 整合測試（CONTRACT §6/§8；
//! BDD harness-roundtrip 場景 5–10、money-approval 場景 2–5）。

mod common;

use chrono::Utc;
use common::*;
use kunsu_core::types::{AgentKind, Money, Wake, WakeMode};
use kunsu_server::{
    normalized_param_hash, InboxIngester, Reactor, ReactorConfig, ServerHandle,
    StaticKeyProvider,
};
use serde_json::{json, Value};
use std::sync::Arc;

fn dry_cfg(ctx: &TestCtx) -> ReactorConfig {
    let mut cfg = ReactorConfig::new(ctx.vault.path().to_path_buf());
    cfg.dry_run = true;
    cfg.claude_bin = Some("/opt/fake/claude".into());
    cfg.retry_base_ms = 1;
    cfg
}

fn reactor_with(ctx: &TestCtx, cfg: ReactorConfig) -> Arc<Reactor> {
    Reactor::new(ctx.handle.clone(), cfg, Arc::new(StaticKeyProvider(test_key())))
}

fn write_playbook(ctx: &TestCtx, rel: &str, content: &str) {
    let path = ctx.vault.path().join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

async fn answered_wake_question(
    handle: &ServerHandle,
    id: &str,
    agent: AgentKind,
    playbook: &str,
) {
    let now = Utc::now();
    let mut req = base_req(id);
    req.source.agent = agent;
    req.wake_on_answer =
        Wake { mode: WakeMode::Fresh, session_id: None, playbook: Some(playbook.into()) };
    handle.create_question(req, now).await.unwrap();
    handle.answer_question(id, ui_answer("a"), now).await.unwrap();
}

// ---------------------------------------------------------------------------
// dry_run 命令行組裝（claude 與 codex 兩型；CONTRACT §8 逐字）
// ---------------------------------------------------------------------------

#[tokio::test]
async fn reactor_dry_run_assembles_claude_fresh_command() {
    let ctx = TestCtx::new().await;
    write_playbook(&ctx, "playbooks/kongko-published.md", "# playbook 內容\n照收據回寫。");
    answered_wake_question(&ctx.handle, "pa-wake-c", AgentKind::Claude, "playbooks/kongko-published.md")
        .await;

    let reactor = reactor_with(&ctx, dry_cfg(&ctx));
    assert_eq!(reactor.process_pending().await.unwrap(), 1);

    let planned = reactor.planned();
    let wake = planned.iter().find(|c| c.kind == "wake_claude").expect("應組裝 claude 喚醒命令");
    assert_eq!(wake.program, "/opt/fake/claude");
    assert!(!wake.executed, "dry_run 不真 spawn");
    assert_eq!(wake.cwd, ctx.vault.path().to_string_lossy(), "cwd = vault 根");
    // 旗標與 CONTRACT §8 逐字一致；prompt 走 stdin，不放進命令列參數（2026-10-09：最大題組的 prompt 超過 ARG_MAX）。
    assert_eq!(
        &wake.args[..],
        &[
            "-p".to_string(),
            "--input-format".into(),
            "text".into(),
            "--output-format".into(),
            "stream-json".into(),
            "--verbose".into(),
            "--permission-mode".into(),
            "bypassPermissions".into(),
            "--max-budget-usd".into(),
            "2".into(),
        ]
    );
    let prompt = wake.stdin.as_deref().expect("prompt 走 stdin");
    assert!(prompt.contains("# playbook 內容"), "prompt 含 playbook 檔內容");
    assert!(prompt.contains("## 題目與答案"), "prompt 含題目與答案段");
    assert!(prompt.contains("pa-wake-c"), "prompt 含題目 JSON");

    // 回寫：dry_run 也要留下 DRY_RUN 收據（doctor 檢視面）
    let q = ctx.handle.get_question("pa-wake-c").await.unwrap().unwrap();
    let result = q.answer.unwrap().execution_result.unwrap();
    assert!(result.starts_with("DRY_RUN:"), "{result}");
}

#[tokio::test]
async fn reactor_dry_run_assembles_codex_command_with_stdin() {
    let ctx = TestCtx::new().await;
    write_playbook(&ctx, "playbooks/x.md", "codex 收據 playbook");
    answered_wake_question(&ctx.handle, "pa-wake-x", AgentKind::Codex, "playbooks/x.md").await;

    // AC-LP-08: fresh uses the same explicit runtime override as resume; do not depend on host PATH.
    let mut cfg = dry_cfg(&ctx);
    let codex = ctx.vault.path().join("injected-codex");
    cfg.codex_bin = Some(codex.clone());
    let reactor = reactor_with(&ctx, cfg);
    reactor.process_pending().await.unwrap();

    let planned = reactor.planned();
    let wake = planned.iter().find(|c| c.kind == "wake_codex").expect("codex 題走 codex exec");
    assert_eq!(wake.program, codex.to_string_lossy());
    assert_eq!(
        wake.args,
        vec![
            "exec".to_string(),
            "-".into(),
            "--json".into(),
            "-s".into(),
            "workspace-write".into(),
            "-C".into(),
            ctx.vault.path().to_string_lossy().to_string(),
        ]
    );
    let stdin = wake.stdin.as_ref().expect("codex prompt 走 stdin");
    assert!(stdin.contains("codex 收據 playbook"));
    assert!(stdin.contains("## 題目與答案"));
}

// ---------------------------------------------------------------------------
// 真 spawn：EXEC_OK 解析回寫 / 重試死信
// ---------------------------------------------------------------------------

#[tokio::test]
async fn reactor_executes_and_parses_exec_ok_marker() {
    let ctx = TestCtx::new().await;
    let script = write_script(
        ctx.vault.path(),
        "fake_claude.sh",
        "#!/bin/sh\necho '{\"type\":\"noise\"}'\necho 'EXEC_OK: 收據已回寫 test'\n",
    );
    write_playbook(&ctx, "playbooks/ok.md", "ok");
    answered_wake_question(&ctx.handle, "pa-exec-ok", AgentKind::Claude, "playbooks/ok.md").await;

    let mut cfg = ReactorConfig::new(ctx.vault.path().to_path_buf());
    cfg.claude_bin = Some(script);
    cfg.retry_base_ms = 1;
    let reactor = reactor_with(&ctx, cfg);
    reactor.process_pending().await.unwrap();

    let q = ctx.handle.get_question("pa-exec-ok").await.unwrap().unwrap();
    let ans = q.answer.unwrap();
    assert_eq!(ans.execution_result.as_deref(), Some("EXEC_OK: 收據已回寫 test"));
    assert!(ans.executed_at.is_some(), "executed_at 必須回寫");
    assert!(ans.acked_at.is_some(), "reactor 起跑即標 acked_at");

    // 冪等：再掃一輪不重複執行（reactor_runs 已 done）
    assert_eq!(reactor.process_pending().await.unwrap(), 0, "同一 answer 絕不重複執行");
}

#[tokio::test]
async fn reactor_retries_then_dead_letters_with_discord() {
    let ctx = TestCtx::new().await;
    let script =
        write_script(ctx.vault.path(), "fail_claude.sh", "#!/bin/sh\necho boom >&2\nexit 2\n");
    write_playbook(&ctx, "playbooks/f.md", "f");
    answered_wake_question(&ctx.handle, "pa-dead", AgentKind::Claude, "playbooks/f.md").await;

    let mut cfg = ReactorConfig::new(ctx.vault.path().to_path_buf());
    cfg.claude_bin = Some(script);
    cfg.retry_base_ms = 1;
    let reactor = reactor_with(&ctx, cfg);
    reactor.process_pending().await.unwrap();

    let q = ctx.handle.get_question("pa-dead").await.unwrap().unwrap();
    let result = q.answer.unwrap().execution_result.unwrap();
    assert!(result.starts_with("EXEC_FAILED: 死信"), "{result}");
    assert!(result.contains("3 次"), "重試 3 次後才進死信：{result}");
    let msgs = ctx.discord.messages();
    assert!(
        msgs.iter().any(|m| m.reason == "reactor_dead_letter"),
        "死信必推 Discord：{msgs:?}"
    );
    // 死信不再被重掃（status=dead）
    assert_eq!(reactor.process_pending().await.unwrap(), 0);
}

// ---------------------------------------------------------------------------
// 批准簽章鏈：假 executor prepare → record 落地 → verify → dry-run commit
// ---------------------------------------------------------------------------

const FAKE_TOKEN: &str = FAKE_EXECUTOR_TOKEN;

async fn answered_money_wake_question(ctx: &TestCtx, id: &str, prepare_cmd: &str) {
    let now = Utc::now();
    let mut req = base_req(id);
    req.money = Some(Money {
        is_money_op: true,
        amount_twd: Some(600),
        entity_id: Some("120000000000005".into()),
        executor_prepare_cmd: Some(prepare_cmd.to_string()),
    });
    req.prediction_ref = Some("projects/meta-ads/predictions/2026-07-31-chain.md".into());
    req.params_line = Some("120000000000005 日預算 500 → 600 (+20%)".into());
    req.wake_on_answer = Wake {
        mode: WakeMode::Fresh,
        session_id: None,
        playbook: Some("playbooks/approved-money-op.md".into()),
    };
    ctx.handle.create_question(req, now).await.unwrap();
    // 選執行型選項（recommended=true 的 "a"）→ 錢鏈觸發
    ctx.handle.answer_question(id, ui_answer("a"), now).await.unwrap();
}

#[tokio::test]
async fn approval_chain_signs_record_and_prepares_commit() {
    let ctx = TestCtx::new().await;
    let base = spawn_http(&ctx).await;
    let client = reqwest::Client::new();
    write_playbook(&ctx, "playbooks/approved-money-op.md", "# 錢 playbook");
    write_fake_executor(&ctx);
    answered_money_wake_question(
        &ctx,
        "pa-chain",
        &whitelisted_prepare_cmd("120000000000005", 600, "nova"),
    )
    .await;

    let reactor = reactor_with(&ctx, dry_cfg(&ctx)); // dry_run：prepare 真跑、commit 只記錄
    reactor.process_pending().await.unwrap();

    // 1) approval record 落地，欄位齊備（executor _approval_gate 要求的鍵全在）
    let approval_path = ctx
        .vault
        .path()
        .join("projects/meta-ads/data/executor_pending/approvals/pa-chain.json");
    assert!(approval_path.exists(), "approval record 必須落檔");
    let record: Value =
        serde_json::from_str(&std::fs::read_to_string(&approval_path).unwrap()).unwrap();
    for k in ["question_id", "executor_token", "param_hash", "answered_at", "nonce", "hmac"] {
        assert!(record.get(k).is_some(), "record 缺 {k}：{record}");
    }
    assert_eq!(record["question_id"], "pa-chain");
    assert_eq!(record["executor_token"], FAKE_TOKEN);

    // 2) param_hash 與 executor.py normalized_param_hash 同演算法（讀 pending spec 重算）
    let spec: Value = serde_json::from_str(
        &std::fs::read_to_string(
            ctx.vault
                .path()
                .join(format!("projects/meta-ads/data/executor_pending/{FAKE_TOKEN}.json")),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(record["param_hash"].as_str().unwrap(), normalized_param_hash(&spec));

    // 3) reactor dry_run 拿到 commit 命令：**server 寫死組裝**（program/script
    //    固定、只信驗過白名單的 token）——stdout 印什麼都改寫不了要執行的程式。
    let planned = reactor.planned();
    let commit = planned.iter().find(|c| c.kind == "commit").expect("應組裝 commit 命令");
    assert!(!commit.executed, "dry_run 不真 commit");
    assert_eq!(commit.program, "python3");
    assert_eq!(
        commit.args,
        vec![
            ctx.vault
                .path()
                .join("projects/meta-ads/scripts/executor.py")
                .to_string_lossy()
                .to_string(),
            "commit".into(),
            "--token".into(),
            FAKE_TOKEN.into(),
            "--approval-file".into(),
            approval_path.to_string_lossy().to_string(),
        ]
    );

    // 4) 簽出的 record 過 /approvals/verify：首次 valid+consumed，重放必拒（單次）
    let out: Value = client
        .post(format!("{base}/approvals/verify"))
        .bearer_auth(TEST_TOKEN)
        .json(&json!({"approval_record": record}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(out["valid"], true, "chain 簽出的 record 必須可驗：{out}");
    assert_eq!(out["consumed_now"], true);
    let replay: Value = client
        .post(format!("{base}/approvals/verify"))
        .bearer_auth(TEST_TOKEN)
        .json(&json!({"approval_record": record}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(replay["valid"], false);

    // 5) 收據：execution_result 記 APPROVAL_SIGNED + DRY_RUN commit
    let q = ctx.handle.get_question("pa-chain").await.unwrap().unwrap();
    let result = q.answer.unwrap().execution_result.unwrap();
    assert!(result.contains("APPROVAL_SIGNED"), "{result}");
    assert!(result.contains("DRY_RUN commit"), "{result}");
}

#[tokio::test]
async fn approval_chain_blocks_loudly_when_prepare_has_no_token() {
    let ctx = TestCtx::new().await;
    // guard 升紅：executor 不印 token、不落 pending spec
    let scripts_dir = ctx.vault.path().join("projects/meta-ads/scripts");
    std::fs::create_dir_all(&scripts_dir).unwrap();
    write_script(
        &scripts_dir,
        "executor.py",
        "#!/usr/bin/env python3\nprint('  🔴 RED：需完整人工複核，不提供一鍵 commit。')\n",
    );
    write_playbook(&ctx, "playbooks/approved-money-op.md", "# 錢 playbook");
    answered_money_wake_question(
        &ctx,
        "pa-chain-red",
        &whitelisted_prepare_cmd("120000000000005", 600, "nova"),
    )
    .await;

    let reactor = reactor_with(&ctx, dry_cfg(&ctx));
    reactor.process_pending().await.unwrap();

    let q = ctx.handle.get_question("pa-chain-red").await.unwrap().unwrap();
    let result = q.answer.unwrap().execution_result.unwrap();
    assert!(result.starts_with("EXEC_BLOCKED:"), "解析不到 token 必須標 execution_blocked：{result}");
    assert!(
        ctx.vault
            .path()
            .join("projects/meta-ads/data/executor_pending/approvals/pa-chain-red.json")
            .exists()
            .eq(&false),
        "封鎖時不得產生懸空授權檔"
    );
    let msgs = ctx.discord.messages();
    assert!(
        msgs.iter().any(|m| m.reason == "execution_blocked"),
        "封鎖必推 Discord，絕不靜默：{msgs:?}"
    );
}

/// gate=red 的錢題永不進一鍵通道（money-approval 場景 10）：作答只落 ledger，
/// 不 prepare、不簽 record。
#[tokio::test]
async fn red_gate_money_question_never_enters_one_click_chain() {
    let ctx = TestCtx::new().await;
    write_fake_executor(&ctx);
    write_playbook(&ctx, "playbooks/approved-money-op.md", "# 錢 playbook");
    let now = Utc::now();
    let mut req = base_req("pa-red");
    req.gate = Some(kunsu_core::types::Gate::Red);
    req.money = Some(Money {
        is_money_op: true,
        amount_twd: Some(9000),
        entity_id: None,
        executor_prepare_cmd: Some(whitelisted_prepare_cmd("120000000000005", 9000, "nova")),
    });
    req.prediction_ref = Some("projects/meta-ads/predictions/2026-07-31-red.md".into());
    req.params_line = Some("日預算 5000 → 9000".into());
    req.wake_on_answer =
        Wake { mode: WakeMode::Fresh, session_id: None, playbook: Some("playbooks/approved-money-op.md".into()) };
    ctx.handle.create_question(req, now).await.unwrap();
    ctx.handle.answer_question("pa-red", ui_answer("a"), now).await.unwrap();

    let reactor = reactor_with(&ctx, dry_cfg(&ctx));
    reactor.process_pending().await.unwrap();

    assert!(
        !ctx.vault
            .path()
            .join("projects/meta-ads/data/executor_pending/approvals/pa-red.json")
            .exists(),
        "red 題不得產 approval record"
    );
    assert!(
        reactor.planned().iter().all(|c| c.kind != "commit"),
        "red 題不得組 commit 命令"
    );
}

// ---------------------------------------------------------------------------
// inbox ingester：正常入列 / 壞檔隔離（P1 定案 6）
// ---------------------------------------------------------------------------

#[tokio::test]
async fn inbox_ingests_good_files_and_quarantines_bad_ones() {
    let ctx = TestCtx::new().await;
    let inbox_dir = tempfile::tempdir().unwrap();

    // 合法檔：與 ask_create arguments 完全同格式（含 requested_tier 正規化）
    let good = json!({
        "id": "pa-inbox-1",
        "title": "app 死掉時落檔的題",
        "question_line": "要繼續嗎？",
        "kind": "choice",
        "requested_tier": "t1",
        "options": [
            {"id": "go", "label": "繼續"},
            {"id": "stop", "label": "先停"},
        ],
        "default_action": {"on_expire": "noop_log"},
        "wake_on_answer": {"mode": "none"},
        "source": {"agent": "claude", "cwd": "/Users/you/notes"},
    });
    std::fs::write(
        inbox_dir.path().join("20260731-pa-inbox-1.json"),
        serde_json::to_string(&good).unwrap(),
    )
    .unwrap();
    // 壞檔 1：非法 JSON；壞檔 2：schema 違規（choice 無 options → malformed）
    std::fs::write(inbox_dir.path().join("20260731-broken.json"), "{not json").unwrap();
    let malformed = json!({
        "id": "pa-inbox-bad",
        "title": "t",
        "question_line": "q",
        "kind": "choice",
        "tier": "t1",
        "options": [],
        "default_action": {"on_expire": "noop_log"},
        "source": {"agent": "claude", "cwd": "/x"},
    });
    std::fs::write(
        inbox_dir.path().join("20260731-zz-malformed.json"),
        serde_json::to_string(&malformed).unwrap(),
    )
    .unwrap();

    let ingester = InboxIngester::new(ctx.handle.clone(), inbox_dir.path().to_path_buf());
    let report = ingester.ingest_once().await;
    assert_eq!(report.ingested, 1);
    assert_eq!(report.quarantined, 2, "壞檔不阻塞其他檔且逐檔隔離");

    // 合法題已入列（走完整 ask_create 驗證與 upsert 語義）
    let q = ctx.handle.get_question("pa-inbox-1").await.unwrap().unwrap();
    assert_eq!(q.tier, kunsu_core::types::Tier::T1);
    // 檔案分流：processed/ 與 quarantine/
    assert!(inbox_dir.path().join("processed/20260731-pa-inbox-1.json").exists());
    assert!(inbox_dir.path().join("quarantine/20260731-broken.json").exists());
    assert!(inbox_dir.path().join("quarantine/20260731-zz-malformed.json").exists());
    assert!(!inbox_dir.path().join("20260731-pa-inbox-1.json").exists());
    // 壞檔 Discord 告警
    let msgs = ctx.discord.messages();
    assert_eq!(msgs.iter().filter(|m| m.reason == "inbox_quarantine").count(), 2);

    // 再掃一輪：目錄已空，冪等
    let report = ingester.ingest_once().await;
    assert_eq!(report.ingested + report.quarantined, 0);

    // 同 id 再落檔（app 死掉期間 agent 重試）→ upsert touch，不重置（INV-8）
    std::fs::write(
        inbox_dir.path().join("20260731-pa-inbox-1b.json"),
        serde_json::to_string(&good).unwrap(),
    )
    .unwrap();
    let report = ingester.ingest_once().await;
    assert_eq!(report.ingested, 1);
    let q2 = ctx.handle.get_question("pa-inbox-1").await.unwrap().unwrap();
    assert_eq!(q2.created_at, q.created_at, "upsert 不重置倒數起點");
}
