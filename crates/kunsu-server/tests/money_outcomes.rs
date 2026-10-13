//! 錢的結果以每一次授權（nonce）追蹤、使用者人工對帳解除結果不明、非紅燈錢題必帶 prepare 命令
//! （CONTRACT §2、§7；BDD money-approval AC-MA-16..18、conversation-continuity AC-CC-14）。源自 QA continuity 第 10 輪。
//! 錢 QA 第 11 輪（AC-MA-20..25）：伺服器算的 `money_state`、舊資料補結果、驗證前中斷的舊資料、重新授權看每一次授權的結果。

mod common;

use chrono::{Duration, Utc};
use common::*;
use kunsu_core::types::Gate;
use kunsu_server::mcp::{handle_rpc, McpReply};
use kunsu_server::MoneyState;
use serde_json::json;

async fn agent_money(ctx: &TestCtx, id: &str) -> String {
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "ask_get", "arguments": {"question_id": id}}});
    let McpReply::Json(v) = handle_rpc(&ctx.handle, body).await else { panic!("要回 JSON") };
    v["result"]["structuredContent"]["question"]["answer"]["execution_result"].as_str().unwrap_or_default().to_string()
}

/// 答一題黃燈錢題、簽出並消耗一次授權，但沒有任何錢鏈結果（executor 驗證後當掉）。
async fn consumed_without_outcome(ctx: &TestCtx, id: &str) {
    let t = Utc::now();
    ctx.handle.create_question(money_t0_req(id), now()).await.unwrap();
    let aid = ctx.handle.answer_question(id, ui_answer("a"), t).await.unwrap().answer.unwrap().answer_id;
    let nonce = format!("n-{id}");
    ctx.handle.record_approval_issued(id, &aid, &nonce, FAKE_EXECUTOR_TOKEN, "h", t.timestamp(), t).await.unwrap();
    let record = json!({"question_id": id, "nonce": nonce, "param_hash": "h"});
    ctx.handle.consume_approval(id, &nonce, &record, t).await.unwrap();
    // 真實情境（同 8/6 的 pa-ios-cbo-scale-6000）：executor 用掉授權後沒有錢鏈結果，交付最後進死信，紀錄沒有錢鏈開頭。
    ctx.handle.record_execution(id, "EXEC_FAILED: 死信（重試 3 次仍失敗）", t).await.unwrap();
}

/// AC-MA-16：非紅燈錢題缺 prepare 命令建題就拒收；紅燈錢題可以沒有（只記錄決定）。
#[tokio::test]
async fn yellow_money_needs_a_prepare_command() {
    let ctx = TestCtx::new().await;
    let mut req = money_t0_req("mo-no-prepare");
    req.money.as_mut().unwrap().executor_prepare_cmd = None;
    let err = ctx.handle.create_question(req.clone(), now()).await.expect_err("黃燈錢題缺 prepare 命令要拒收");
    assert!(err.to_string().contains("prepare"), "{err}");
    let mut no_gate = req.clone();
    no_gate.id = "mo-no-prepare-nogate".into();
    no_gate.gate = None;
    assert!(ctx.handle.create_question(no_gate, now()).await.is_err(), "沒有 gate 的錢題也算一鍵，要拒收");
    let mut red = req;
    red.id = "mo-no-prepare-red".into();
    red.gate = Some(Gate::Red);
    ctx.handle.create_question(red, now()).await.expect("紅燈錢題可以沒有 prepare 命令");
}

/// AC-MA-17：判斷看 nonce，不看時間——之後改寫執行紀錄（例如交付追加）不會讓結果不明消失。
#[tokio::test]
async fn unknown_outcome_survives_later_execution_rewrites() {
    let ctx = TestCtx::new().await;
    let id = "mo-rewrite";
    consumed_without_outcome(&ctx, id).await;
    assert!(ctx.handle.reauthorize_question(id, Utc::now()).await.is_err(), "前提：結果不明");
    // 交付路徑之後寫了一筆（沒有錢鏈開頭的）執行紀錄，executed_at 比消耗晚。
    ctx.handle.record_execution(id, "EXEC_BLOCKED: 舊的被擋；交付：已在原對話追加", Utc::now() + Duration::minutes(5)).await.unwrap();
    assert!(
        ctx.handle.reauthorize_question(id, Utc::now() + Duration::minutes(6)).await.is_err(),
        "已消耗卻沒有結果的授權，不會因為之後的執行紀錄而算有結果"
    );
    let (h2, _db) = ctx.fresh_db_same_vault().await;
    let report = h2.rebuild_db_at(Utc::now() + Duration::minutes(7)).await.unwrap();
    assert_eq!(report.reconcile, 1, "重建後仍列人工對帳：{report:?}");
    assert!(h2.reauthorize_question(id, Utc::now() + Duration::minutes(8)).await.is_err(), "重建後仍拒絕");
}

/// AC-MA-18：使用者標「錢沒有動」→ 可以重新授權；agent 讀到人工對帳結果；帳本有行、重建後一致。
#[tokio::test]
async fn ray_resolves_unknown_as_not_executed_then_may_reauthorize() {
    let ctx = TestCtx::new().await;
    let id = "mo-resolve-no";
    consumed_without_outcome(&ctx, id).await;
    let q = ctx.handle.resolve_money_outcome(id, false, Utc::now()).await.unwrap();
    assert!(q.answer.unwrap().execution_result.unwrap().starts_with("RECONCILED_NOT_EXECUTED"));
    assert!(agent_money(&ctx, id).await.contains("沒有執行"), "agent 讀到人工對帳確認沒有執行");
    let (h2, _db) = ctx.fresh_db_same_vault().await;
    let report = h2.rebuild_db_at(Utc::now()).await.unwrap();
    assert_eq!(report.reconcile, 0, "已對帳的不再列：{report:?}");
    ctx.handle.reauthorize_question(id, Utc::now()).await.expect("標「沒有動」後可以重新授權");
    h2.reauthorize_question(id, Utc::now()).await.expect("重建後也可以重新授權");
}

/// AC-MA-18：使用者標「錢已經動了」→ 等同已執行完成，永不能重新授權。
#[tokio::test]
async fn ray_resolves_unknown_as_executed_and_it_is_never_reauthorized() {
    let ctx = TestCtx::new().await;
    let id = "mo-resolve-yes";
    consumed_without_outcome(&ctx, id).await;
    ctx.handle.resolve_money_outcome(id, true, Utc::now()).await.unwrap();
    assert!(agent_money(&ctx, id).await.contains("已執行完成"), "agent 讀到已執行完成（人工對帳）");
    assert!(ctx.handle.reauthorize_question(id, Utc::now()).await.is_err(), "錢已經動了就不能再授權");
    let (h2, _db) = ctx.fresh_db_same_vault().await;
    h2.rebuild_db_at(Utc::now()).await.unwrap();
    assert!(h2.reauthorize_question(id, Utc::now()).await.is_err(), "重建後一樣不能");
}

/// 沒有結果不明（也沒有待對帳）時，人工對帳被拒，不能拿來繞過錢鏈。
#[tokio::test]
async fn resolving_without_an_unknown_outcome_is_refused() {
    let ctx = TestCtx::new().await;
    let id = "mo-resolve-nothing";
    ctx.handle.create_question(money_t0_req(id), now()).await.unwrap();
    ctx.handle.answer_question(id, ui_answer("a"), Utc::now()).await.unwrap();
    assert!(ctx.handle.resolve_money_outcome(id, false, Utc::now()).await.is_err());
    // 人工對帳只有 UI（Tauri IPC）能叫：MCP 沒有這個工具。
    assert!(!kunsu_server::tool_specs().iter().any(|t| t["name"].as_str().is_some_and(|n| n.contains("resolve"))));
}

// ---------------------------------------------------------------------------
// 2026-10-09 錢 QA 第 11 輪（F1／F2／F5／F7；BDD money-approval AC-MA-20..25）
// ---------------------------------------------------------------------------

async fn side_pool(ctx: &TestCtx) -> sqlx::SqlitePool {
    kunsu_server::db::open_pool(&ctx.db_path).await.unwrap()
}

async fn state_of(h: &kunsu_server::ServerHandle, id: &str) -> Option<MoneyState> {
    let q = h.get_question(id).await.unwrap().unwrap();
    h.money_state(&q).await.unwrap()
}

async fn agent_money_on(h: &kunsu_server::ServerHandle, id: &str) -> String {
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "ask_get", "arguments": {"question_id": id}}});
    let McpReply::Json(v) = handle_rpc(h, body).await else { panic!("要回 JSON") };
    v["result"]["structuredContent"]["question"]["answer"]["execution_result"].as_str().unwrap_or_default().to_string()
}

/// AC-MA-20（F2）：`money_state` 依錢鏈列、每一次授權的結果與重新授權的所有檢查算出。
#[tokio::test]
async fn money_state_follows_the_chain_and_the_reauthorize_rules() {
    let ctx = TestCtx::new().await;
    let h = &ctx.handle;
    // 非錢題、還沒作答的錢題：沒有 money_state。
    h.create_question(base_req("ms-plain"), now()).await.unwrap();
    h.answer_question("ms-plain", ui_answer("a"), Utc::now()).await.unwrap();
    assert_eq!(state_of(h, "ms-plain").await, None);
    h.create_question(money_t0_req("ms-open"), now()).await.unwrap();
    assert_eq!(state_of(h, "ms-open").await, None);

    // 剛批准：錢鏈列 queued → running。
    let id = "ms-chain";
    h.create_question(money_t0_req(id), now()).await.unwrap();
    let aid = h.answer_question(id, ui_answer("a"), Utc::now()).await.unwrap().answer.unwrap().answer_id;
    assert_eq!(state_of(h, id).await, Some(MoneyState::Running), "錢鏈列 queued");
    // 被擋（沒有消耗）→ 可重新授權。
    h.record_execution(id, "EXEC_BLOCKED: prepare 失敗", Utc::now()).await.unwrap();
    assert_eq!(state_of(h, id).await, Some(MoneyState::Reauthorizable));
    // 重新授權 → 又在跑；授權被 executor 消耗卻沒有結果、列結束 → 結果不明。
    h.reauthorize_question(id, Utc::now()).await.unwrap();
    assert_eq!(state_of(h, id).await, Some(MoneyState::Running));
    let t = Utc::now();
    h.record_approval_issued(id, &aid, "n-ms-1", FAKE_EXECUTOR_TOKEN, "h", t.timestamp(), t).await.unwrap();
    h.consume_approval(id, "n-ms-1", &json!({"question_id": id, "nonce": "n-ms-1"}), t).await.unwrap();
    assert_eq!(state_of(h, id).await, Some(MoneyState::Running), "列還在跑時不是結果不明");
    h.record_execution(id, "EXEC_FAILED: 死信（重試 3 次仍失敗）", Utc::now()).await.unwrap();
    assert_eq!(state_of(h, id).await, Some(MoneyState::Unknown), "8/6 型：死信但授權已被消耗");
    h.resolve_money_outcome(id, false, Utc::now()).await.unwrap();
    assert_eq!(state_of(h, id).await, Some(MoneyState::Reauthorizable), "對帳確認沒動");

    // 對帳確認已經動了 → settled；紅燈、安全選項 → settled。
    let done = "ms-done";
    consumed_without_outcome(&ctx, done).await;
    h.resolve_money_outcome(done, true, Utc::now()).await.unwrap();
    assert_eq!(state_of(h, done).await, Some(MoneyState::Settled));
    let mut red = money_t0_req("ms-red");
    red.gate = Some(Gate::Red);
    h.create_question(red, now()).await.unwrap();
    h.answer_question("ms-red", ui_answer("a"), Utc::now()).await.unwrap();
    assert_eq!(state_of(h, "ms-red").await, Some(MoneyState::Settled), "紅燈不走錢鏈");
    h.create_question(money_t0_req("ms-safe"), now()).await.unwrap();
    h.answer_question("ms-safe", ui_answer("b"), Utc::now()).await.unwrap();
    assert_eq!(state_of(h, "ms-safe").await, Some(MoneyState::Settled), "安全選項不走錢鏈");
}

/// AC-MA-25（F7）：重新授權看每一次授權的結果——某個 nonce 已是 COMMIT_OK，執行紀錄被改寫成別的開頭也一樣拒絕；沒有結果不明的
/// 授權也不能對帳。
#[tokio::test]
async fn reauthorize_checks_every_nonce_not_only_the_execution_text() {
    let ctx = TestCtx::new().await;
    let h = &ctx.handle;
    let id = "mo-per-nonce-ok";
    h.create_question(money_t0_req(id), now()).await.unwrap();
    let aid = h.answer_question(id, ui_answer("a"), Utc::now()).await.unwrap().answer.unwrap().answer_id;
    let t = Utc::now();
    h.record_approval_issued(id, &aid, "n-ok", FAKE_EXECUTOR_TOKEN, "h", t.timestamp(), t).await.unwrap();
    h.consume_approval(id, "n-ok", &json!({"question_id": id, "nonce": "n-ok"}), t).await.unwrap();
    h.record_money_outcome(id, "n-ok", "COMMIT_OK: commit 完成", t).await.unwrap();
    // 執行紀錄被改寫成別的開頭（舊版的 needs_reconcile 對帳路徑曾把 COMMIT_OK 改成 RECONCILED_NOT_EXECUTED）。
    h.record_execution(id, "RECONCILED_NOT_EXECUTED: 舊版對帳", Utc::now()).await.unwrap();
    assert!(h.reauthorize_question(id, Utc::now()).await.is_err(), "有一次授權已執行完成：不能再授權");
    assert!(h.resolve_money_outcome(id, false, Utc::now()).await.is_err(), "沒有結果不明的授權：不能對帳");
    assert_eq!(state_of(h, id).await, Some(MoneyState::Settled));
}

/// AC-MA-19：重新授權只給錢鏈明確沒有動錢的題（被擋、對帳確認沒動）；乾跑或從沒跑過的舊答案不給——money_state 同一套判斷。
#[tokio::test]
async fn reauthorize_needs_a_chain_that_clearly_did_not_move_money() {
    let ctx = TestCtx::new().await;
    let h = &ctx.handle;
    let id = "mo-dry-run";
    h.create_question(money_t0_req(id), now()).await.unwrap();
    h.answer_question(id, ui_answer("a"), Utc::now()).await.unwrap();
    h.record_execution(id, "APPROVAL_SIGNED: ‹approval 檔›；DRY_RUN commit: python3 executor.py commit", Utc::now()).await.unwrap();
    let err = h.reauthorize_question(id, Utc::now()).await.expect_err("乾跑不能重新授權").to_string();
    assert!(err.contains("被擋下或對帳確認錢沒有動"), "{err}");
    assert_eq!(state_of(h, id).await, Some(MoneyState::Settled));
    // 從沒跑過（重建後沒有列、沒有執行紀錄）的舊答案。
    let old = "mo-never-ran";
    h.create_question(money_t0_req(old), now()).await.unwrap();
    h.answer_question(old, ui_answer("a"), Utc::now() - Duration::days(3)).await.unwrap();
    let (h2, _db) = ctx.fresh_db_same_vault().await;
    h2.rebuild_db_at(Utc::now()).await.unwrap();
    assert!(h2.reauthorize_question(old, Utc::now()).await.is_err(), "從沒跑過的舊答案不能重新授權");
    assert_eq!(state_of(&h2, old).await, Some(MoneyState::Settled));
}

/// AC-MA-21（F1）：舊資料補結果——只補執行紀錄開頭是 COMMIT_OK／RECONCILED_EXECUTED 的；冪等；其他開頭維持結果不明。
#[tokio::test]
async fn startup_backfills_legacy_executed_money_only() {
    let ctx = TestCtx::new().await;
    let h = &ctx.handle;
    let setups = [
        ("mo-legacy-ok", "COMMIT_OK: commit 完成；已在原對話追加處理"),
        ("mo-legacy-reconciled", "RECONCILED_EXECUTED: 使用者人工對帳確認錢已經動了"),
        ("mo-legacy-dead", "EXEC_FAILED: 死信（重試 3 次仍失敗）"),
    ];
    for (id, result) in setups {
        h.create_question(money_t0_req(id), now()).await.unwrap();
        let aid = h.answer_question(id, ui_answer("a"), Utc::now()).await.unwrap().answer.unwrap().answer_id;
        let t = Utc::now();
        let nonce = format!("n-{id}");
        h.record_approval_issued(id, &aid, &nonce, FAKE_EXECUTOR_TOKEN, "h", t.timestamp(), t).await.unwrap();
        h.consume_approval(id, &nonce, &json!({"question_id": id, "nonce": nonce}), t).await.unwrap();
        h.record_execution(id, result, Utc::now()).await.unwrap();
    }
    // 升級前的樣子：money_outcomes 是新表、空的。
    let pool = side_pool(&ctx).await;
    sqlx::query("DELETE FROM money_outcomes").execute(&pool).await.unwrap();
    let h2 = ctx.reopen().await;
    assert_eq!(agent_money_on(&h2, "mo-legacy-ok").await, "已執行完成");
    assert_eq!(agent_money_on(&h2, "mo-legacy-reconciled").await, "已執行完成（使用者人工對帳確認）");
    assert!(agent_money_on(&h2, "mo-legacy-dead").await.starts_with("結果不明"), "死信的不補");
    assert_eq!(state_of(&h2, "mo-legacy-dead").await, Some(MoneyState::Unknown));
    let rows = || {
        let mut out = Vec::new();
        for entry in std::fs::read_dir(ctx.handle.ledger().asks_dir()).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                for line in std::fs::read_to_string(&path).unwrap().lines() {
                    let v: serde_json::Value = serde_json::from_str(line).unwrap_or_default();
                    if v["execution"]["money_outcome"]["backfill"] == true {
                        out.push(v["execution"]["money_outcome"]["result"].as_str().unwrap_or_default().to_string());
                    }
                }
            }
        }
        out
    };
    let first = rows();
    assert_eq!(first.len(), 2, "兩題補資料、各一行：{first:?}");
    assert!(first.iter().any(|r| r.starts_with("COMMIT_OK")) && first.iter().any(|r| r.starts_with("RECONCILED_EXECUTED")), "{first:?}");
    ctx.reopen().await;
    assert_eq!(rows().len(), 2, "再啟動不重複補");
}

/// AC-MA-21（自我稽核補充）：補資料的推論只對「產生那個結果的那一次授權」安全——同一題有兩次以上已消耗卻沒有結果的授權時（舊版
/// round 9 之前，驗證後失敗被記成被擋、又重新授權），較早那次的結果真的不明，不能一起補成已執行；整題維持結果不明，由使用者對帳。
#[tokio::test]
async fn backfill_leaves_questions_with_several_unrecorded_authorizations_unknown() {
    let ctx = TestCtx::new().await;
    let h = &ctx.handle;
    let id = "mo-legacy-two";
    h.create_question(money_t0_req(id), now()).await.unwrap();
    let aid = h.answer_question(id, ui_answer("a"), Utc::now()).await.unwrap().answer.unwrap().answer_id;
    let t = Utc::now();
    for nonce in ["n-two-a", "n-two-b"] {
        h.record_approval_issued(id, &aid, nonce, FAKE_EXECUTOR_TOKEN, "h", t.timestamp(), t).await.unwrap();
        h.consume_approval(id, nonce, &json!({"question_id": id, "nonce": nonce}), t).await.unwrap();
    }
    h.record_execution(id, "COMMIT_OK: commit 完成", Utc::now()).await.unwrap();
    let pool = side_pool(&ctx).await;
    sqlx::query("DELETE FROM money_outcomes").execute(&pool).await.unwrap();
    let h2 = ctx.reopen().await;
    let filled: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM money_outcomes WHERE question_id = ?1").bind(id).fetch_one(&pool).await.unwrap();
    assert_eq!(filled, 0, "兩次未記結果的授權：不補");
    assert_eq!(state_of(&h2, id).await, Some(MoneyState::Unknown), "整題結果不明，使用者對帳");
    assert!(agent_money_on(&h2, id).await.starts_with("結果不明"));
}

/// AC-MA-23（F5 舊資料）：舊版重啟恢復把「executor 驗證前中斷」也標成 needs_reconcile——啟動補資料時轉成被擋（錢沒有動）、列改
/// dead、可以重新授權；有結果不明 nonce 的 needs_reconcile 列不動。
#[tokio::test]
async fn legacy_needs_reconcile_before_verify_becomes_reauthorizable() {
    let ctx = TestCtx::new().await;
    let h = &ctx.handle;
    let pool = side_pool(&ctx).await;
    async fn mark(pool: &sqlx::SqlitePool, aid: &str) {
        sqlx::query("UPDATE reactor_runs SET status = 'needs_reconcile', result = 'NEEDS_RECONCILE: 舊版' WHERE answer_id = ?1")
            .bind(aid)
            .execute(pool)
            .await
            .unwrap();
    }
    // 驗證前中斷：簽發了、沒被消耗、沒有結果。
    let id = "mo-legacy-before-verify";
    h.create_question(money_t0_req(id), now()).await.unwrap();
    let aid = h.answer_question(id, ui_answer("a"), Utc::now()).await.unwrap().answer.unwrap().answer_id;
    let t = Utc::now();
    h.record_approval_issued(id, &aid, "n-before", FAKE_EXECUTOR_TOKEN, "h", t.timestamp(), t).await.unwrap();
    mark(&pool, &aid).await;
    // 對照：驗證後中斷（結果不明）。
    let unknown = "mo-legacy-after-verify";
    h.create_question(money_t0_req(unknown), now()).await.unwrap();
    let aid2 = h.answer_question(unknown, ui_answer("a"), Utc::now()).await.unwrap().answer.unwrap().answer_id;
    h.record_approval_issued(unknown, &aid2, "n-after", FAKE_EXECUTOR_TOKEN, "h", t.timestamp(), t).await.unwrap();
    h.consume_approval(unknown, "n-after", &json!({"question_id": unknown, "nonce": "n-after"}), t).await.unwrap();
    mark(&pool, &aid2).await;

    let h2 = ctx.reopen().await;
    let raw = h2.get_question(id).await.unwrap().unwrap().answer.unwrap().execution_result.unwrap_or_default();
    let outcome: Option<String> =
        sqlx::query_scalar("SELECT result FROM money_outcomes WHERE nonce = 'n-before'").fetch_optional(&pool).await.unwrap();
    let row: String = sqlx::query_scalar("SELECT status FROM reactor_runs WHERE question_id = ?1").bind(id).fetch_one(&pool).await.unwrap();
    assert!(raw.starts_with("EXEC_BLOCKED") && raw.contains("錢沒有動"), "執行紀錄：{raw:?}");
    assert!(outcome.as_deref().is_some_and(|o| o.starts_with("EXEC_BLOCKED")), "nonce 記成被擋：{outcome:?}");
    assert_eq!(row, "dead");
    assert_eq!(state_of(&h2, id).await, Some(MoneyState::Reauthorizable));
    assert_eq!(state_of(&h2, unknown).await, Some(MoneyState::Unknown), "結果不明的不動");
    let row2: String =
        sqlx::query_scalar("SELECT status FROM reactor_runs WHERE question_id = ?1").bind(unknown).fetch_one(&pool).await.unwrap();
    assert_eq!(row2, "needs_reconcile");
    h2.reauthorize_question(id, Utc::now()).await.expect("轉成被擋後可以重新授權");
}
