//! QA 題組第 2 輪（BDD batch-review AC-BR-06；CONTRACT §8 第 5 步、§14）：獨立 QA 的重現與回歸測試。
//!
//! 重點：開新對話的 prompt 真的走 stdin——假 `claude`／`codex` 腳本把收到的命令列參數（NUL 分隔）與 stdin 原樣存檔，
//! 測試直接比對檔案內容，而不是只看 `PlannedCommand`。不連真實 server（4777）、不啟動真實 Claude／Codex。
//! 回原對話追加（resume）的訊息在命令列參數：上限內最大的題組也要塞得進去、真的回到原對話（決策面板 QA 第 3 輪把原本標
//! ignore 的觀察測試改成正式測試，2026-10-09 主線已替建議與副標訂上限）。

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use common::*;
use kunsu_core::continuity::DeliveryState;
use kunsu_server::*;
use serde_json::{json, Value};
use tempfile::TempDir;

/// macOS `kern.argmax`（命令列參數＋環境變數的總上限）。
const ARG_MAX: usize = 1_048_576;

/// 記錄參數與 stdin 的假 CLI：`<dir>/<name>.args`（每個參數以 NUL 結尾）、`<dir>/<name>.stdin`。
fn recording_cli(dir: &Path, name: &str) -> PathBuf {
    let args = dir.join(format!("{name}.args"));
    let stdin = dir.join(format!("{name}.stdin"));
    let body = format!(
        "#!/bin/sh\n: > '{a}'\nfor x in \"$@\"; do printf '%s\\0' \"$x\" >> '{a}'; done\ncat > '{s}'\necho 'EXEC_OK: 已處理'\n",
        a = args.display(),
        s = stdin.display()
    );
    write_script(dir, &format!("{name}.sh"), &body)
}

fn recorded_args(dir: &Path, name: &str) -> Vec<String> {
    let raw = std::fs::read(dir.join(format!("{name}.args"))).expect("假 CLI 沒有被呼叫");
    raw.split(|b| *b == 0)
        .filter(|chunk| !chunk.is_empty())
        .map(|chunk| String::from_utf8_lossy(chunk).to_string())
        .collect()
}

fn recorded_stdin(dir: &Path, name: &str) -> String {
    std::fs::read_to_string(dir.join(format!("{name}.stdin"))).expect("假 CLI 沒有收到 stdin")
}

/// 合法的最大題組：20 項可編輯、每項草稿 5,000 字、每封來信 4,000 字（context_md 沒有上限）。
fn max_batch_json(id: &str) -> Value {
    let mut v = serde_json::to_value(base_req(id)).unwrap();
    let o = v.as_object_mut().unwrap();
    o.insert("title".into(), json!("今天 20 封信的回覆"));
    o.insert("question_line".into(), json!("今天 20 封信的回覆"));
    o.insert("kind".into(), json!("batch"));
    o.insert("gate".into(), Value::Null);
    o.insert("options".into(), json!([]));
    let draft = "你".repeat(5000);
    let letter = "來".repeat(4000);
    o.insert(
        "items".into(),
        Value::Array(
            (1..=20)
                .map(|n| json!({"id": format!("i{n}"), "title": format!("第 {n} 封"), "context_md": letter, "proposal": draft, "editable": true}))
                .collect(),
        ),
    );
    o.insert("wake_on_answer".into(), json!({"mode": "fresh"}));
    v
}

/// 每一項都採用、最終文字各不相同（第 n 項以「#n#」開頭再補到 5,000 字），方便逐項確認有沒有完整送到。
fn final_text(n: usize) -> String {
    let head = format!("#{n}#");
    format!("{head}{}", "好".repeat(5000 - head.chars().count()))
}

async fn answer_all_approved(ctx: &TestCtx, id: &str) {
    let q = ctx.handle.get_question(id).await.unwrap().unwrap();
    let decisions: Vec<Value> = q
        .items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            json!({"item_id": item.id, "action": "approve", "text": final_text(i + 1), "content_hash": item.content_hash})
        })
        .collect();
    let input: AnswerInput = serde_json::from_value(json!({"channel": "ui", "items": decisions})).unwrap();
    ctx.handle.answer_question(id, input, now()).await.expect("合法的題組答案");
}

fn reactor(ctx: &TestCtx, claude: Option<PathBuf>, codex: Option<PathBuf>, root: &Path) -> Arc<Reactor> {
    let mut cfg = ReactorConfig::new(ctx.vault.path().to_path_buf());
    cfg.claude_bin = claude;
    cfg.codex_bin = codex;
    cfg.sessions_dir = root.join("registry");
    cfg.desktop_sessions_dir = root.join("desktop");
    cfg.retry_base_ms = 1;
    Reactor::new(ctx.handle.clone(), cfg, Arc::new(StaticKeyProvider(test_key())))
}

async fn delivery(ctx: &TestCtx, id: &str) -> Option<DeliveryState> {
    ctx.handle.get_question(id).await.unwrap().unwrap().answer.and_then(|a| a.delivery).map(|d| d.state)
}

// ---------------------------------------------------------------------------
// AC-BR-06：開新對話的 prompt 走 stdin（Claude）
// ---------------------------------------------------------------------------

/// 期望：`claude -p --input-format text …` 的命令列參數裡沒有 prompt；prompt（題目 JSON＋逐項決定段落）完整從 stdin 進去，
/// 20 項的最終文字逐字都在；最大題組交得出去（交付狀態 fresh）。
#[tokio::test]
async fn qa2_br06_fresh_claude_prompt_arrives_on_stdin_and_never_in_argv() {
    let ctx = TestCtx::new().await;
    let bin = TempDir::new().unwrap();
    let claude = recording_cli(bin.path(), "claude");
    ctx.handle
        .create_question(serde_json::from_value(max_batch_json("mail-max2")).unwrap(), now())
        .await
        .expect("20 項 × 5,000 字是合法題組");
    answer_all_approved(&ctx, "mail-max2").await;

    let r = reactor(&ctx, Some(claude), None, bin.path());
    r.process_pending().await.unwrap();

    let args = recorded_args(bin.path(), "claude");
    assert_eq!(
        args,
        ["-p", "--input-format", "text", "--output-format", "stream-json", "--verbose", "--permission-mode", "bypassPermissions", "--max-budget-usd", "2"],
        "開新對話的命令列參數只有旗標、不含 prompt"
    );
    let arg_bytes: usize = args.iter().map(|a| a.len() + 1).sum();
    assert!(arg_bytes < 1024, "命令列參數應該很小；實際 {arg_bytes} bytes");

    let stdin = recorded_stdin(bin.path(), "claude");
    assert!(stdin.contains("## 題目與答案"), "stdin 要有題目 JSON 段落");
    assert!(stdin.contains("## 題組：使用者的逐項決定（照這裡做）"), "stdin 要有逐項決定段落");
    assert!(stdin.contains("一字不改"), "stdin 要寫明一字不改送出");
    for n in 1..=20 {
        assert!(stdin.contains(&final_text(n)), "第 {n} 項的最終文字要完整出現在 stdin");
    }
    let planned = r.planned();
    let fresh = planned.iter().find(|c| c.kind == "wake_claude").expect("開新對話的命令");
    assert_eq!(
        Some(stdin.len()),
        fresh.stdin.as_ref().map(String::len),
        "子行程收到的 stdin 與計畫的 prompt 一樣長（沒有被截斷）"
    );
    assert!(stdin.len() > ARG_MAX, "這個題組的 prompt（{} bytes）本來就塞不進命令列，才能證明走的是 stdin", stdin.len());
    assert_eq!(delivery(&ctx, "mail-max2").await, Some(DeliveryState::Fresh));
}

// ---------------------------------------------------------------------------
// AC-BR-06：開新對話的 prompt 走 stdin（Codex）
// ---------------------------------------------------------------------------

/// 期望：Codex 的開新對話是 `codex exec - --json -s workspace-write -C <vault>`，prompt 只從 stdin 進去。
#[tokio::test]
async fn qa2_br06_fresh_codex_prompt_arrives_on_stdin_and_never_in_argv() {
    let ctx = TestCtx::new().await;
    let bin = TempDir::new().unwrap();
    let codex = recording_cli(bin.path(), "codex");
    let mut v = max_batch_json("mail-max-codex");
    v["source"]["agent"] = json!("codex");
    ctx.handle.create_question(serde_json::from_value(v).unwrap(), now()).await.expect("合法題組");
    answer_all_approved(&ctx, "mail-max-codex").await;

    let r = reactor(&ctx, None, Some(codex), bin.path());
    r.process_pending().await.unwrap();

    let vault = ctx.vault.path().to_string_lossy().to_string();
    let args = recorded_args(bin.path(), "codex");
    assert_eq!(args, ["exec", "-", "--json", "-s", "workspace-write", "-C", vault.as_str()], "命令列參數不含 prompt");
    let stdin = recorded_stdin(bin.path(), "codex");
    for n in 1..=20 {
        assert!(stdin.contains(&final_text(n)), "第 {n} 項的最終文字要完整出現在 stdin");
    }
    assert_eq!(delivery(&ctx, "mail-max-codex").await, Some(DeliveryState::Fresh));
}

// ---------------------------------------------------------------------------
// AC-BR-06：上限內最大的題組回原對話追加（resume）——訊息塞得進命令列參數、真的回到原對話
// ---------------------------------------------------------------------------

/// CONTRACT §8 第 3 步：Claude 回原對話追加是 `<cli> -p <prompt> --resume <id>`（prompt 在命令列參數）。2026-10-09 主線把只核可
/// 項目的 `proposal` 上限訂為 5,000 字、`subtitle` ≤200 字（`batch_item_subtitle_too_long`），所以通過建題驗證的題組，追加訊息
/// 一定塞得進 macOS ARG_MAX。這裡用上限內最大的只核可題組（20 項、標題 80 字、副標 200 字、建議 5,000 字，全部照建議）確認：
/// 訊息遠小於 ARG_MAX（留 64 KB 給環境變數）、假 `claude` 真的以 `--resume <原對話>` 收到完整訊息、交付狀態 resumed（PRD §20：
/// 只有確定無法接續才另開新對話）。（取代 QA 第 2 輪標 ignore 的觀察測試：當時建議與副標沒有上限，30,000 字的建議會超過
/// ARG_MAX、spawn 失敗改開新對話。）
#[tokio::test]
async fn qa2_br06_resume_prompt_for_the_largest_valid_approve_only_batch_fits_the_command_line() {
    let ctx = TestCtx::new().await;
    let bin = TempDir::new().unwrap();
    let work = bin.path().join("work");
    std::fs::create_dir_all(&work).unwrap();
    for dir in ["registry", "desktop/acct/org"] {
        std::fs::create_dir_all(bin.path().join(dir)).unwrap();
    }
    let claude = recording_cli(bin.path(), "claude");

    let mut v = serde_json::to_value(base_req("refund-big")).unwrap();
    let o = v.as_object_mut().unwrap();
    o.insert("kind".into(), json!("batch"));
    o.insert("gate".into(), Value::Null);
    o.insert("options".into(), json!([]));
    let proposal = |n: usize| {
        let head = format!("#{n}# 符合退費規則，建議全額退款：");
        format!("{head}{}", "審".repeat(5_000 - head.chars().count()))
    };
    o.insert(
        "items".into(),
        Value::Array(
            (1..=20)
                .map(|n| {
                    let title = format!("第 {n} 筆退款{}", "款".repeat(80 - format!("第 {n} 筆退款").chars().count()));
                    json!({"id": format!("r{n}"), "title": title, "subtitle": "退".repeat(200), "context_md": "申請內容", "proposal": proposal(n), "editable": false})
                })
                .collect(),
        ),
    );
    o.insert(
        "source".into(),
        json!({"agent": "claude", "cwd": work.to_string_lossy(), "host": "claude_desktop", "conversation_id": "conv-refund-big", "host_session_id": "local_refund-big", "attended": false}),
    );
    ctx.handle
        .create_question(serde_json::from_value(v).unwrap(), now())
        .await
        .expect("標題 80 字、副標 200 字、建議 5,000 字的 20 項只核可題組是合法題組");
    let q = ctx.handle.get_question("refund-big").await.unwrap().unwrap();
    let decisions: Vec<Value> =
        q.items.iter().map(|i| json!({"item_id": i.id, "action": "approve", "content_hash": i.content_hash})).collect();
    let input: AnswerInput = serde_json::from_value(json!({"channel": "ui", "items": decisions})).unwrap();
    let answered = ctx.handle.answer_question("refund-big", input, now()).await.expect("合法答案");

    let prompt = kunsu_server::reactor::continuation_prompt(&answered, None);
    assert!(
        prompt.len() < ARG_MAX - 64 * 1024,
        "上限內最大的題組，回原對話的訊息 {} bytes 要塞得進 macOS ARG_MAX（{ARG_MAX}，留 64 KB 給環境變數）",
        prompt.len()
    );

    let r = reactor(&ctx, Some(claude), None, bin.path());
    r.process_pending().await.unwrap();
    let args = recorded_args(bin.path(), "claude");
    let resume = args.iter().position(|a| a == "--resume").expect("要以 --resume 回到原對話");
    assert_eq!(args.get(resume + 1).map(String::as_str), Some("conv-refund-big"), "接續的是發問的那段對話");
    let p = args.iter().position(|a| a == "-p").expect("追加訊息在 -p 之後");
    let sent = &args[p + 1];
    assert_eq!(sent, &prompt, "子行程收到的追加訊息與 continuation_prompt 一字不差（沒有被截斷）");
    for n in 1..=20 {
        assert!(sent.contains(&proposal(n)), "第 {n} 項的完整建議要在追加訊息裡");
    }
    let arg_bytes: usize = args.iter().map(|a| a.len() + 1).sum();
    assert!(arg_bytes < ARG_MAX - 64 * 1024, "命令列參數總共 {arg_bytes} bytes");
    assert_eq!(delivery(&ctx, "refund-big").await, Some(DeliveryState::Resumed), "回到原對話，沒有改開新對話");
}
