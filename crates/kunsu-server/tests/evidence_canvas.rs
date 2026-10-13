//! 證據畫布與草稿註冊（BDD evidence-canvas，AC-EC-01～08、12；INV `inv_controls_native`）。
//!
//! renderer 以假的注入：記錄請求、回傳設定好的問題、寫出 1×1 PNG。真正的沙盒渲染在 app 殼層，
//! 由 UI 測試與手動 QA 驗證。

mod common;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use tokio::sync::{mpsc, oneshot};

use chrono::Duration;
use common::*;
use kunsu_core::types::*;
use kunsu_server::mcp::{handle_rpc, McpReply};
use kunsu_server::*;
use serde_json::{json, Value};
use tempfile::TempDir;

const PNG_1X1: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00,
    0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0d, 0x49,
    0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0x00, 0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0d, 0x0a, 0x2d, 0xb4, 0x00, 0x00,
    0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

#[derive(Default)]
struct FakeRenderer {
    requests: Mutex<Vec<RenderRequest>>,
    issues: Mutex<Vec<DraftIssue>>,
}

impl FakeRenderer {
    fn fail_with(&self, issue: DraftIssue) {
        self.issues.lock().unwrap().push(issue);
    }
    fn requests(&self) -> Vec<RenderRequest> {
        self.requests.lock().unwrap().clone()
    }
}

impl CanvasRenderer for FakeRenderer {
    fn render(&self, req: RenderRequest) -> kunsu_server::draft::BoxFuture<'_, RenderReport> {
        Box::pin(async move {
            std::fs::write(&req.png_path, PNG_1X1).unwrap();
            self.requests.lock().unwrap().push(req);
            RenderReport { issues: self.issues.lock().unwrap().clone(), height: Some(420), preview_written: true }
        })
    }
}

struct Env {
    ctx: TestCtx,
    home: TempDir,
    renderer: Arc<FakeRenderer>,
}

impl Env {
    async fn new() -> Env {
        let ctx = TestCtx::new().await;
        let home = TempDir::new().unwrap();
        std::fs::create_dir_all(home.path().join("drafts")).unwrap();
        ctx.handle.set_draft_paths(DraftPaths {
            drafts_root: home.path().join("drafts"),
            revisions_root: home.path().join("revisions"),
        });
        let renderer = Arc::new(FakeRenderer::default());
        ctx.handle.set_canvas_renderer(renderer.clone());
        Env { ctx, home, renderer }
    }

    fn drafts(&self) -> PathBuf {
        self.home.path().join("drafts")
    }

    fn draft(&self, id: &str, question_md: &str) -> PathBuf {
        let dir = self.drafts().join(id);
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        std::fs::write(dir.join("question.md"), question_md).unwrap();
        dir
    }

    async fn register(&self, dir: &Path) -> RegisterOutcome {
        self.ctx.handle.register_draft(dir, RegisterOptions::default(), now()).await.unwrap()
    }

    async fn dry_run(&self, dir: &Path) -> RegisterOutcome {
        self.ctx
            .handle
            .register_draft(dir, RegisterOptions { dry_run: true, source: None }, now())
            .await
            .unwrap()
    }
}

/// 合法草稿：frontmatter 第 1 行是 `---`，`question_line` 在第 4 行，`options` 在第 7 行。
fn question_md(id: &str, question_line: &str, body: &str) -> String {
    format!(
        "---\nid: {id}\ntitle: 素材 B 要不要加碼\nquestion_line: {question_line}\nkind: choice\ntier: t1\noptions:\n  - id: a\n    label: 加碼 20%\n    recommended: true\n  - id: b\n    label: 維持\nwhy: |\n  - 09:00 ROAS 掉到 1.8\n  - 09:20 比較三組素材後 B 最穩\nsource:\n  agent: claude\n  cwd: /tmp\n---\n{body}"
    )
}

/// 前置：frontmatter 共 19 行（含頭尾 `---`），內文從第 20 行開始。
const BODY_START_LINE: u32 = 20;

/// 面板作答：帶使用者當下看到的版本與內容雜湊（AC-EC-05）。
async fn seen_answer(env: &Env, id: &str, choice: &str) -> AnswerInput {
    let d = env.ctx.handle.get_question(id).await.unwrap().unwrap().draft.expect("草稿題");
    let mut a = ui_answer(choice);
    a.revision = Some(d.revision);
    a.content_hash = Some(d.content_hash);
    a
}

fn has_issue<'a>(out: &'a RegisterOutcome, code: &str) -> &'a DraftIssue {
    out.issues
        .iter()
        .find(|i| i.code == code)
        .unwrap_or_else(|| panic!("應回報 {code}：{:#?}", out.issues))
}

fn assert_actionable(out: &RegisterOutcome) {
    assert!(!out.ok, "有問題就不通過");
    assert!(!out.issues.is_empty(), "要逐條回報");
    for issue in &out.issues {
        assert!(!issue.file.is_empty() && !issue.message.is_empty() && !issue.hint.is_empty(), "{issue:?}");
    }
}

// ---------------------------------------------------------------------------
// AC-EC-01：合法草稿註冊成功、開始週期，回傳 question_id／revision／preview_png
// ---------------------------------------------------------------------------

#[tokio::test]
async fn valid_draft_registers_and_starts_the_cycle() {
    let env = Env::new().await;
    let dir = env.draft("ec-ok", &question_md("ec-ok", "要不要加碼素材 B？", "## 數據\n\n![素材 B](./assets/b.png)\n"));
    std::fs::write(dir.join("assets/b.png"), PNG_1X1).unwrap();
    std::fs::write(
        dir.join("canvas.tsx"),
        "import { Stat } from 'kunsu'\nexport default function C() { return <Stat label=\"ROAS\" value=\"1.8\" /> }\n",
    )
    .unwrap();

    let out = env.register(&dir).await;
    assert!(out.ok, "{:#?}", out.issues);
    assert_eq!(out.question_id.as_deref(), Some("ec-ok"));
    assert_eq!(out.revision, Some(1));
    assert_eq!(out.dedup.as_deref(), Some("created"));
    let png = PathBuf::from(out.preview_png.as_deref().expect("回傳預覽截圖路徑"));
    assert_eq!(std::fs::read(&png).unwrap(), PNG_1X1, "預覽是 renderer 畫出來的那張");

    let q = env.ctx.handle.get_question("ec-ok").await.unwrap().unwrap();
    assert_eq!(q.question_line, "要不要加碼素材 B？");
    assert!(q.why_md.as_deref().unwrap().contains("09:00 ROAS 掉到 1.8"), "why 進 why_md");
    assert!(q.body_md.contains("![素材 B](./assets/b.png)"), "內文保留相對路徑，顯示時再換成快照");
    assert_eq!(q.options.len(), 2);
    let draft = q.draft.clone().expect("draft 參照");
    assert_eq!(draft.revision, 1);
    assert!(draft.content_hash.starts_with("sha256:"));
    assert_eq!(draft.canvas, Some(CanvasKind::Tsx));
    assert!(q.cycle_started_at.is_some() && q.force_at.is_some(), "註冊即開始 15 分鐘週期");

    let req = env.renderer.requests().pop().expect("真的渲染了一次");
    assert_eq!(req.question.id, "ec-ok");
    let canvas = req.evidence.canvas.expect("畫布交給 renderer");
    assert_eq!(canvas.kind, CanvasKind::Tsx);
    assert!(canvas.source.contains("<Stat"));
    assert!(req.evidence.assets["./assets/b.png"].starts_with("data:image/png;base64,"));

    let bundle = env.ctx.handle.evidence_bundle("ec-ok", 1).await.unwrap().expect("版本快照可讀");
    assert_eq!(bundle.canvas.unwrap().kind, CanvasKind::Tsx);
    assert!(bundle.assets["./assets/b.png"].starts_with("data:image/png;base64,"));
    assert!(env.home.path().join("revisions/ec-ok/1/question.md").exists());
}

/// 沒有畫布也能註冊；id 預設是資料夾名稱。
#[tokio::test]
async fn markdown_only_draft_uses_folder_name_as_id() {
    let env = Env::new().await;
    let md = question_md("ec-folder", "要不要延長檔期？", "| 指標 | 值 |\n|---|---|\n| CTR | 1.2% |\n")
        .replace("id: ec-folder\n", "");
    let dir = env.draft("ec-folder", &md);
    let out = env.register(&dir).await;
    assert!(out.ok, "{:#?}", out.issues);
    let q = env.ctx.handle.get_question("ec-folder").await.unwrap().unwrap();
    assert_eq!(q.draft.unwrap().canvas, None);
}

// ---------------------------------------------------------------------------
// AC-EC-02：所有檢查失敗都不建立題目，issues 指出檔案、位置、代碼、說明與改法
// ---------------------------------------------------------------------------

#[tokio::test]
async fn yaml_error_points_at_the_line() {
    let env = Env::new().await;
    let md = question_md("ec-yaml", "要不要加碼？", "").replace("kind: choice\n", "kind: [choice\n");
    let dir = env.draft("ec-yaml", &md);
    let out = env.register(&dir).await;
    assert_actionable(&out);
    let issue = has_issue(&out, "frontmatter_yaml");
    assert_eq!(issue.file, "question.md");
    assert!(issue.line.is_some_and(|l| (5..=8).contains(&l)), "指到 kind 附近：{issue:?}");
    assert!(env.ctx.handle.get_question("ec-yaml").await.unwrap().is_none(), "不建立題目");
}

#[tokio::test]
async fn field_rule_violation_points_at_the_key() {
    let env = Env::new().await;
    let dir = env.draft("ec-long", &question_md("ec-long", "這一句決策句寫得太長了遠遠超過二十四個字的上限所以一定要被退回", ""));
    let out = env.register(&dir).await;
    assert_actionable(&out);
    let issue = has_issue(&out, "question_line_too_long");
    assert_eq!((issue.file.as_str(), issue.line), ("question.md", Some(4)));
    assert!(env.ctx.handle.get_question("ec-long").await.unwrap().is_none());

    let md = question_md("ec-opts", "要不要加碼？", "")
        .replace("  - id: b\n    label: 維持\n", "");
    let dir = env.draft("ec-opts", &md);
    let out = env.register(&dir).await;
    let issue = has_issue(&out, "malformed_choice_options");
    assert_eq!(issue.line, Some(7), "指到 options:");
}

#[tokio::test]
async fn unsafe_markdown_and_missing_assets_are_reported_by_line() {
    let env = Env::new().await;
    let body = "第一段\n\n<script>alert(1)</script>\n\n![外部](https://cdn.example.com/x.png)\n\n![不存在](./assets/nope.png)\n\n<div onclick=\"x()\">點我</div>\n";
    let dir = env.draft("ec-md", &question_md("ec-md", "要不要加碼？", body));
    let out = env.register(&dir).await;
    assert_actionable(&out);
    let script = has_issue(&out, "markdown_forbidden_tag");
    assert_eq!((script.line, script.column), (Some(BODY_START_LINE + 2), Some(1)));
    assert_eq!(has_issue(&out, "external_url").line, Some(BODY_START_LINE + 4));
    assert_eq!(has_issue(&out, "asset_missing").line, Some(BODY_START_LINE + 6));
    assert_eq!(has_issue(&out, "markdown_forbidden_attr").line, Some(BODY_START_LINE + 8));
    assert!(env.renderer.requests().is_empty(), "靜態檢查沒過就不渲染");
    assert!(env.ctx.handle.get_question("ec-md").await.unwrap().is_none());
}

#[tokio::test]
async fn renderer_issues_block_registration_but_still_return_preview() {
    let env = Env::new().await;
    let dir = env.draft("ec-tsx", &question_md("ec-tsx", "要不要加碼？", ""));
    std::fs::write(dir.join("canvas.tsx"), "export default function C() {\n  return <div>\n}\n").unwrap();
    env.renderer.fail_with(DraftIssue {
        file: "canvas.tsx".into(),
        line: Some(3),
        column: Some(1),
        code: "canvas_compile".into(),
        message: "Unexpected token".into(),
        hint: "補上 </div>".into(),
    });
    let out = env.register(&dir).await;
    assert_actionable(&out);
    assert_eq!(has_issue(&out, "canvas_compile").line, Some(3));
    assert!(out.preview_png.is_some(), "失敗也給截圖，方便 agent 看出問題");
    assert!(env.ctx.handle.get_question("ec-tsx").await.unwrap().is_none());
}

#[tokio::test]
async fn draft_must_live_in_the_drafts_folder_and_match_its_id() {
    let env = Env::new().await;
    let outside = TempDir::new().unwrap();
    std::fs::write(outside.path().join("question.md"), question_md("ec-out", "要不要加碼？", "")).unwrap();
    let out = env.register(outside.path()).await;
    has_issue(&out, "draft_outside_root");

    // 草稿檔是指向外面的 symlink → 一樣拒絕（不讀 drafts 以外的檔案）。
    let dir = env.drafts().join("ec-link");
    std::fs::create_dir_all(&dir).unwrap();
    std::os::unix::fs::symlink(outside.path().join("question.md"), dir.join("question.md")).unwrap();
    has_issue(&env.register(&dir).await, "draft_outside_root");

    let dir = env.draft("ec-name", &question_md("ec-other", "要不要加碼？", ""));
    has_issue(&env.register(&dir).await, "id_mismatch");

    let dir = env.draft("ec-two", &question_md("ec-two", "要不要加碼？", ""));
    std::fs::write(dir.join("canvas.tsx"), "export default () => null\n").unwrap();
    std::fs::write(dir.join("canvas.html"), "<p>hi</p>\n").unwrap();
    has_issue(&env.register(&dir).await, "canvas_multiple");
}

#[tokio::test]
async fn canvas_without_renderer_is_reported() {
    let ctx = TestCtx::new().await;
    let home = TempDir::new().unwrap();
    ctx.handle.set_draft_paths(DraftPaths {
        drafts_root: home.path().join("drafts"),
        revisions_root: home.path().join("revisions"),
    });
    let dir = home.path().join("drafts/ec-norender");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("question.md"), question_md("ec-norender", "要不要加碼？", "")).unwrap();
    std::fs::write(dir.join("canvas.tsx"), "export default () => null\n").unwrap();
    let out = ctx.handle.register_draft(&dir, RegisterOptions::default(), now()).await.unwrap();
    has_issue(&out, "renderer_unavailable");
}

// ---------------------------------------------------------------------------
// AC-EC-03：dry-run 完整檢查與預覽，不建立題目
// ---------------------------------------------------------------------------

#[tokio::test]
async fn dry_run_previews_without_creating() {
    let env = Env::new().await;
    let dir = env.draft("ec-dry", &question_md("ec-dry", "要不要加碼？", "內文\n"));
    std::fs::write(dir.join("canvas.jsx"), "export default () => <p>hi</p>\n").unwrap();
    let out = env.dry_run(&dir).await;
    assert!(out.ok && out.dry_run, "{:#?}", out.issues);
    assert!(out.preview_png.is_some());
    assert_eq!(env.renderer.requests().len(), 1);
    assert!(env.ctx.handle.get_question("ec-dry").await.unwrap().is_none(), "dry-run 不建題");
    assert!(!env.home.path().join("revisions/ec-dry").exists(), "dry-run 不留版本");

    // 有問題的 dry-run 也要標明是 dry-run。
    let bad = env.draft("ec-dry-bad", &question_md("ec-dry-bad", "這一句決策句寫得太長了遠遠超過二十四個字的上限所以一定要被退回", ""));
    let out = env.dry_run(&bad).await;
    assert!(!out.ok && out.dry_run, "{out:?}");
}

// ---------------------------------------------------------------------------
// AC-EC-04：同 id 重新註冊 → 新 revision，不重設週期、不重發通知；已結案拒絕
// ---------------------------------------------------------------------------

#[tokio::test]
async fn reregistering_updates_content_without_resetting_the_cycle() {
    let env = Env::new().await;
    let dir = env.draft("ec-rev", &question_md("ec-rev", "要不要加碼？", "第一版\n"));
    assert!(env.register(&dir).await.ok);
    let before = env.ctx.handle.get_question("ec-rev").await.unwrap().unwrap();

    let mut rx = env.ctx.handle.subscribe();
    std::fs::write(dir.join("question.md"), question_md("ec-rev", "要不要加碼 30%？", "第二版\n")).unwrap();
    let later = now() + Duration::minutes(4);
    let out = env.ctx.handle.register_draft(&dir, RegisterOptions::default(), later).await.unwrap();
    assert!(out.ok, "{:#?}", out.issues);
    assert_eq!((out.revision, out.dedup.as_deref()), (Some(2), Some("updated")));

    let after = env.ctx.handle.get_question("ec-rev").await.unwrap().unwrap();
    assert_eq!(after.question_line, "要不要加碼 30%？");
    assert!(after.body_md.contains("第二版"));
    assert_eq!(after.draft.as_ref().unwrap().revision, 2);
    assert_ne!(after.draft.as_ref().unwrap().content_hash, before.draft.as_ref().unwrap().content_hash);
    assert_eq!(
        (after.cycle_started_at, after.warn_at, after.force_at, after.cycle_number, after.first_proposed, after.created_at),
        (before.cycle_started_at, before.warn_at, before.force_at, before.cycle_number, before.first_proposed, before.created_at),
        "不重設週期（inv_upsert_no_reset）"
    );
    let events = drain_events(&mut rx);
    assert!(
        !events.iter().any(|e| matches!(e, ServerEvent::QuestionCreated { .. } | ServerEvent::Notify { .. })),
        "不重發通知：{events:?}"
    );
    assert!(events.iter().any(|e| matches!(e, ServerEvent::QuestionTouched { .. })), "UI 要知道內容換了");
    assert!(env.home.path().join("revisions/ec-rev/1/question.md").exists(), "舊版保留");
    assert!(env.home.path().join("revisions/ec-rev/2/question.md").exists());

    // 內容沒變再註冊 → 不升版。
    let out = env.ctx.handle.register_draft(&dir, RegisterOptions::default(), later).await.unwrap();
    assert_eq!((out.revision, out.dedup.as_deref()), (Some(2), Some("unchanged")));

    // 已結案 → 拒絕更新並回傳既有答案。
    let seen = seen_answer(&env, "ec-rev", "a").await;
    env.ctx.handle.answer_question("ec-rev", seen, later).await.unwrap();
    std::fs::write(dir.join("question.md"), question_md("ec-rev", "要不要加碼 40%？", "第三版\n")).unwrap();
    let out = env.ctx.handle.register_draft(&dir, RegisterOptions::default(), later).await.unwrap();
    assert!(!out.ok);
    has_issue(&out, "question_closed");
    assert_eq!(out.answer.unwrap().choice_id.as_deref(), Some("a"));
    assert_eq!(env.ctx.handle.get_question("ec-rev").await.unwrap().unwrap().question_line, "要不要加碼 30%？");
}

// ---------------------------------------------------------------------------
// AC-EC-05：答案綁定 revision 與內容雜湊；看舊版作答會被擋
// ---------------------------------------------------------------------------

#[tokio::test]
async fn answer_binds_the_revision_ray_saw() {
    let env = Env::new().await;
    let dir = env.draft("ec-bind", &question_md("ec-bind", "要不要加碼？", "第一版\n"));
    assert!(env.register(&dir).await.ok);
    std::fs::write(dir.join("question.md"), question_md("ec-bind", "要不要加碼？", "第二版\n")).unwrap();
    assert_eq!(env.register(&dir).await.revision, Some(2));

    let mut stale = seen_answer(&env, "ec-bind", "a").await;
    stale.revision = Some(1);
    let err = env.ctx.handle.answer_question("ec-bind", stale, now()).await.unwrap_err();
    assert!(matches!(err, ServerError::StaleRevision { seen: 1, current: 2, .. }), "{err}");

    let fresh = seen_answer(&env, "ec-bind", "a").await;
    let q = env.ctx.handle.answer_question("ec-bind", fresh, now()).await.unwrap();
    let a = q.answer.unwrap();
    assert_eq!(a.revision, Some(2));
    assert_eq!(a.content_hash.as_deref(), Some(q.draft.unwrap().content_hash.as_str()));

    let rows = read_ledger_rows(&env.ctx, now());
    let answered = rows.iter().find(|r| r.ledger_type == LedgerType::Answered).expect("answered 行");
    assert_eq!(answered.answer.as_ref().unwrap().revision, Some(2), "ledger 可追溯版本");
}

// ---------------------------------------------------------------------------
// AC-EC-08：資產複製進快照，大小上限與外部網址
// ---------------------------------------------------------------------------

#[tokio::test]
async fn assets_are_bounded_local_and_snapshotted() {
    let env = Env::new().await;
    let dir = env.draft("ec-big", &question_md("ec-big", "要不要加碼？", "![大圖](./assets/big.png)\n"));
    std::fs::write(dir.join("assets/big.png"), vec![0u8; 2 * 1024 * 1024 + 1]).unwrap();
    has_issue(&env.register(&dir).await, "asset_too_large");

    let refs: String = (0..5).map(|i| format!("![p{i}](./assets/p{i}.png)\n\n")).collect();
    let dir = env.draft("ec-total", &question_md("ec-total", "要不要加碼？", &refs));
    for i in 0..5 {
        std::fs::write(dir.join(format!("assets/p{i}.png")), vec![0u8; 1900 * 1024]).unwrap();
    }
    has_issue(&env.register(&dir).await, "assets_total_too_large");

    let dir = env.draft("ec-escape", &question_md("ec-escape", "要不要加碼？", "![x](./assets/../../secret.png)\n"));
    has_issue(&env.register(&dir).await, "asset_outside_draft");

    let dir = env.draft("ec-canvas-url", &question_md("ec-canvas-url", "要不要加碼？", ""));
    std::fs::write(
        dir.join("canvas.tsx"),
        "export default () => (\n  <svg xmlns=\"http://www.w3.org/2000/svg\"><image href=\"https://x.test/a.png\" /></svg>\n)\n",
    )
    .unwrap();
    let out = env.register(&dir).await;
    let issue = has_issue(&out, "external_url");
    assert_eq!((issue.file.as_str(), issue.line), ("canvas.tsx", Some(2)));
    assert_eq!(out.issues.iter().filter(|i| i.code == "external_url").count(), 1, "SVG 命名空間不算外部網址");

    // 只複製有引用到的資產。
    let dir = env.draft("ec-snap", &question_md("ec-snap", "要不要加碼？", "![b](./assets/b.png)\n"));
    std::fs::write(dir.join("assets/b.png"), PNG_1X1).unwrap();
    std::fs::write(dir.join("assets/unused.png"), PNG_1X1).unwrap();
    assert!(env.register(&dir).await.ok);
    let snap = env.home.path().join("revisions/ec-snap/1/assets");
    assert!(snap.join("b.png").exists());
    assert!(!snap.join("unused.png").exists());
}

// ---------------------------------------------------------------------------
// AC-EC-12：JSON ask_create 相容並支援 why_md；MCP ask_register
// ---------------------------------------------------------------------------

async fn mcp(handle: &ServerHandle, name: &str, arguments: Value) -> Result<Value, Value> {
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": name, "arguments": arguments}});
    match handle_rpc(handle, body).await {
        McpReply::Json(v) => match v.get("error") {
            Some(e) => Err(e.clone()),
            None => Ok(v["result"]["structuredContent"].clone()),
        },
        McpReply::Accepted => panic!("tools/call 不得回 Accepted"),
    }
}

#[tokio::test]
async fn json_ask_create_still_works_without_a_draft() {
    let env = Env::new().await;
    let mut req = base_req("ec-json");
    req.why_md = Some("- 10:00 發現 CPC 飆高".into());
    env.ctx.handle.create_question(req, now()).await.unwrap();
    let q = env.ctx.handle.get_question("ec-json").await.unwrap().unwrap();
    assert!(q.draft.is_none());
    assert_eq!(q.why_md.as_deref(), Some("- 10:00 發現 CPC 飆高"));
}

#[tokio::test]
async fn mcp_ask_register_reports_issues_and_registers() {
    let env = Env::new().await;
    assert!(tool_specs().iter().any(|t| t["name"] == "ask_register"));
    assert_eq!(tool_specs().len(), 14);

    let dir = env.draft("ec-mcp", &question_md("ec-mcp", "這一句決策句寫得太長了遠遠超過二十四個字的上限所以一定要被退回", ""));
    let out = mcp(&env.ctx.handle, "ask_register", json!({"draft_dir": dir})).await.unwrap();
    assert_eq!(out["ok"], false);
    assert_eq!(out["issues"][0]["code"], "question_line_too_long");
    assert_eq!(out["issues"][0]["line"], 4);

    std::fs::write(dir.join("question.md"), question_md("ec-mcp", "要不要加碼？", "")).unwrap();
    let out = mcp(&env.ctx.handle, "ask_register", json!({"draft_dir": dir, "dry_run": true})).await.unwrap();
    assert_eq!((out["ok"].clone(), out["dry_run"].clone()), (json!(true), json!(true)));
    let source = json!({"agent": "codex", "cwd": "/tmp", "conversation_id": "019a0000-0000-7000-8000-000000000004"});
    let out = mcp(&env.ctx.handle, "ask_register", json!({"draft_dir": dir, "source": source})).await.unwrap();
    assert_eq!(out["ok"], true, "{out}");
    let q = env.ctx.handle.get_question("ec-mcp").await.unwrap().unwrap();
    assert_eq!(q.source.agent, AgentKind::Codex, "呼叫端身分優先於 frontmatter");
    assert_eq!(q.source.conversation_id.as_deref(), Some("019a0000-0000-7000-8000-000000000004"));
    assert_eq!(q.wake_on_answer.mode, WakeMode::Continue, "有原對話 → 接續");
}

// ---------------------------------------------------------------------------
// INV：決策控制永遠原生（inv_controls_native）
// ---------------------------------------------------------------------------

/// 畫布畫出再像按鈕的東西都不是選項：選項、送出與錢參數只來自 frontmatter 並由原生介面呈現；
/// 註冊永遠不產生答案。
#[tokio::test]
async fn inv_controls_native() {
    let env = Env::new().await;
    let canvas = "export default () => (\n  <div><button>批准加碼</button><form><input name=\"choice\" value=\"a\" /></form></div>\n)\n";
    let md = question_md("ec-native", "要不要加碼？", "").replace(
        "options:\n  - id: a\n    label: 加碼 20%\n    recommended: true\n  - id: b\n    label: 維持\n",
        "",
    );
    let dir = env.draft("ec-native", &md);
    std::fs::write(dir.join("canvas.tsx"), canvas).unwrap();
    let out = env.register(&dir).await;
    has_issue(&out, "malformed_choice_options");
    assert!(env.renderer.requests().is_empty(), "選項不完整就不渲染、不建題");

    std::fs::write(dir.join("question.md"), question_md("ec-native", "要不要加碼？", "")).unwrap();
    let out = env.register(&dir).await;
    assert!(out.ok, "{:#?}", out.issues);
    let q = env.ctx.handle.get_question("ec-native").await.unwrap().unwrap();
    assert_eq!(q.options.iter().map(|o| o.id.as_str()).collect::<Vec<_>>(), ["a", "b"], "選項只來自 frontmatter");
    assert!(q.answer.is_none(), "註冊永不產生答案");
    let bundle = env.ctx.handle.evidence_bundle("ec-native", 1).await.unwrap().unwrap();
    assert_eq!(bundle.canvas.unwrap().source, canvas, "畫布原樣進沙盒，Kunsu 不從裡面讀任何控制");
}

/// 已結案題重新註冊時回傳的既有答案也要去敏（不含簽章）。
#[tokio::test]
async fn closed_registration_answer_is_redacted() {
    let env = Env::new().await;
    let dir = env.draft("ec-closed-redact", &question_md("ec-closed-redact", "要不要加碼？", ""));
    assert!(env.register(&dir).await.ok);
    let mut input = seen_answer(&env, "ec-closed-redact", "a").await;
    input.signature = Some("sig-SECRET-7".into());
    env.ctx.handle.answer_question("ec-closed-redact", input, now()).await.unwrap();
    let out = mcp(&env.ctx.handle, "ask_register", json!({"draft_dir": dir})).await.unwrap();
    assert_eq!(out["ok"], false);
    assert_eq!(out["answer"]["choice_id"], "a");
    assert!(!out.to_string().contains("sig-SECRET-7"), "{out}");
}

// ---------------------------------------------------------------------------
// QA（2026-10-08）證據畫布第一輪
// ---------------------------------------------------------------------------

/// #3：重新註冊換了選項，預設動作跟著新草稿（並以新選項驗證），舊的 pick_option 不會指到意思已變的選項。
#[tokio::test]
async fn reregistration_takes_the_new_default_action() {
    let env = Env::new().await;
    let with_default = |line: &str, b_label: &str| {
        question_md("ec-default", line, "")
            .replace("  - id: b\n    label: 維持\n", &format!("  - id: b\n    label: {b_label}\n"))
            .replace("source:\n", "default_action:\n  on_expire: pick_option\n  option_id: b\nsource:\n")
    };
    let dir = env.draft("ec-default", &with_default("要不要加碼？", "維持"));
    assert!(env.register(&dir).await.ok);
    // 第二版：b 的意思變了，而且不再寫預設動作（預設 noop_log）。
    std::fs::write(dir.join("question.md"), question_md("ec-default", "要不要加碼？", "").replace("  - id: b\n    label: 維持\n", "  - id: b\n    label: 下架全部素材\n")).unwrap();
    assert!(env.register(&dir).await.ok);
    let q = env.ctx.handle.get_question("ec-default").await.unwrap().unwrap();
    assert_eq!(q.default_action.on_expire, OnExpire::NoopLog, "預設動作跟著新草稿，不沿用舊的 pick_option");
}

/// #4a：草稿題不能用 ask_update 或 JSON 重送偷改內容（不升版就繞過所見即所答）。
#[tokio::test]
async fn draft_questions_only_change_through_registration() {
    let env = Env::new().await;
    let dir = env.draft("ec-locked", &question_md("ec-locked", "要不要加碼？", "內文\n"));
    assert!(env.register(&dir).await.ok);
    let err = mcp(&env.ctx.handle, "ask_update", json!({"question_id": "ec-locked", "question_line": "全部停掉？"})).await;
    assert!(err.is_err(), "草稿題要改草稿再註冊");
    let mut req = base_req("ec-locked");
    req.evidence = vec![EvidenceLine { text: "偷換的證據".into(), source: "x".into() }];
    env.ctx.handle.create_question(req, now()).await.unwrap();
    let q = env.ctx.handle.get_question("ec-locked").await.unwrap().unwrap();
    assert_eq!(q.question_line, "要不要加碼？");
    assert!(q.evidence.iter().all(|e| e.text != "偷換的證據"), "JSON 重送不改草稿題內容");
    // 鎖的是內容，不是使用者的操作：稍後再問照常可用。
    env.ctx.handle.dismiss_question("ec-locked", now()).await.unwrap();
}

/// #4b：草稿題從面板作答一定要帶看到的版本。
#[tokio::test]
async fn ui_answers_to_drafts_must_name_the_revision() {
    let env = Env::new().await;
    let dir = env.draft("ec-need-rev", &question_md("ec-need-rev", "要不要加碼？", ""));
    assert!(env.register(&dir).await.ok);
    let err = env.ctx.handle.answer_question("ec-need-rev", ui_answer("a"), now()).await.unwrap_err();
    assert!(matches!(err, ServerError::StaleRevision { .. } | ServerError::BadAnswer(_)), "{err}");
    let ok = seen_answer(&env, "ec-need-rev", "a").await;
    env.ctx.handle.answer_question("ec-need-rev", ok, now()).await.unwrap();
}

/// #6：參照式圖片與跨行 <img> 的外部網址一樣擋。
#[tokio::test]
async fn external_images_are_caught_in_any_markdown_form() {
    let env = Env::new().await;
    let body = "![追蹤][r]\n\n[r]: https://tracker.test/p.png\n\n<img\n  src=\"https://tracker.test/q.png\">\n";
    let dir = env.draft("ec-ref", &question_md("ec-ref", "要不要加碼？", body));
    let out = env.register(&dir).await;
    assert_eq!(out.issues.iter().filter(|i| i.code == "external_url").count(), 2, "{:#?}", out.issues);
}

/// #2／#7：HTML 畫布不能用 meta refresh 換頁；assets 資料夾本身是 symlink 也要擋。
#[tokio::test]
async fn html_canvas_meta_refresh_and_symlinked_assets_are_rejected() {
    let env = Env::new().await;
    let dir = env.draft("ec-meta", &question_md("ec-meta", "要不要加碼？", ""));
    std::fs::write(dir.join("canvas.html"), "<p>hi</p>\n<meta http-equiv=\"refresh\" content=\"0;url=ht&#116;p://x.test/\">\n").unwrap();
    let out = env.register(&dir).await;
    assert_eq!(has_issue(&out, "canvas_navigation").line, Some(2));

    let outside = TempDir::new().unwrap();
    std::fs::write(outside.path().join("b.png"), PNG_1X1).unwrap();
    let dir = env.drafts().join("ec-assets-link");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("question.md"), question_md("ec-assets-link", "要不要加碼？", "![b](./assets/b.png)\n")).unwrap();
    std::os::unix::fs::symlink(outside.path(), dir.join("assets")).unwrap();
    has_issue(&env.register(&dir).await, "asset_outside_draft");
}

/// #8：MCP dry_run 一定是布林，不能因為寫成字串就變成正式註冊。
#[tokio::test]
async fn dry_run_must_be_a_boolean() {
    let env = Env::new().await;
    let dir = env.draft("ec-dry-str", &question_md("ec-dry-str", "要不要加碼？", ""));
    assert!(mcp(&env.ctx.handle, "ask_register", json!({"draft_dir": dir, "dry_run": "true"})).await.is_err());
    assert!(env.ctx.handle.get_question("ec-dry-str").await.unwrap().is_none());
}

/// BDD 場景 9：畫布失敗留下診斷事件。
#[tokio::test]
async fn canvas_failures_are_recorded() {
    let env = Env::new().await;
    let dir = env.draft("ec-diag", &question_md("ec-diag", "要不要加碼？", ""));
    assert!(env.register(&dir).await.ok);
    env.ctx.handle.record_canvas_failure("ec-diag", 1, "x is not defined（canvas.tsx 第 2 行）", now()).await.unwrap();
    assert!(env.ctx.handle.canvas_failures("ec-diag").await.unwrap().iter().any(|r| r.contains("x is not defined")));
}

// ---------------------------------------------------------------------------
// QA 第 2 輪（AC-EC-04／05／08）
// ---------------------------------------------------------------------------

/// AC-EC-05：面板作答要同時帶版本與內容雜湊；沒帶雜湊拒收、雜湊不符視為過期。
#[tokio::test]
async fn panel_answers_need_matching_revision_and_hash() {
    let env = Env::new().await;
    let dir = env.draft("ec-hash", &question_md("ec-hash", "要不要加碼？", "內文\n"));
    assert!(env.register(&dir).await.ok);
    let mut no_hash = ui_answer("a");
    no_hash.revision = Some(1);
    let err = env.ctx.handle.answer_question("ec-hash", no_hash, now()).await.unwrap_err();
    assert!(matches!(err, ServerError::BadAnswer(_)), "{err}");
    let mut wrong = seen_answer(&env, "ec-hash", "a").await;
    wrong.content_hash = Some("sha256:not-what-ray-saw".into());
    let err = env.ctx.handle.answer_question("ec-hash", wrong, now()).await.unwrap_err();
    assert!(matches!(err, ServerError::StaleRevision { .. }), "{err}");
    let ok = seen_answer(&env, "ec-hash", "a").await;
    env.ctx.handle.answer_question("ec-hash", ok, now()).await.unwrap();
}

/// AC-EC-08：frontmatter 裡會被渲染成 Markdown 的欄位（證據、選項、補充說明、決策句）也不能引用外部圖片；
/// 選項引用的本地圖片一樣進快照。
#[tokio::test]
async fn frontmatter_markdown_fields_cannot_load_external_images() {
    let env = Env::new().await;
    let base = question_md("ID", "要不要加碼？", "內文\n");
    let cases = [
        ("ec-fm-ev", base.replace("source:\n  agent", "evidence:\n  - text: '![t](https://tracker.test/e.png)'\n    source: Meta 後台\nsource:\n  agent")),
        ("ec-fm-opt", base.replace("label: 維持", "label: '維持 ![t](https://tracker.test/o.png)'")),
        ("ec-fm-detail", base.replace("label: 維持", "label: 維持\n    detail: '<img src=\"https://tracker.test/d.png\">'")),
        ("ec-fm-ql", base.replace("question_line: 要不要加碼？", "question_line: '![](https://t.test/q)'")),
    ];
    for (id, md) in cases {
        let out = env.register(&env.draft(id, &md.replace("id: ID", &format!("id: {id}")))).await;
        assert!(!out.ok, "{id} 註冊成功了：{out:?}");
        assert!(out.issues.iter().any(|i| i.code == "external_url"), "{id}：{:?}", out.issues);
    }
    let local = base.replace("id: ID", "id: ec-fm-local").replace("label: 維持", "label: '維持 ![b](./assets/b.png)'");
    let dir = env.draft("ec-fm-local", &local);
    std::fs::write(dir.join("assets/b.png"), PNG_1X1).unwrap();
    let out = env.register(&dir).await;
    assert!(out.ok, "{out:?}");
    let bundle = env.ctx.handle.evidence_bundle("ec-fm-local", 1).await.unwrap().unwrap();
    assert!(bundle.assets.contains_key("./assets/b.png"), "選項引用的圖片要進快照");
}

/// AC-EC-08：<picture>／<source srcset>／<img srcset> 會繞過「只顯示快照」，一律擋。
#[tokio::test]
async fn picture_and_srcset_are_rejected() {
    let env = Env::new().await;
    for (id, body) in [
        ("ec-pic", "<picture><source srcset=\"https://tracker.test/s.png\"><img src=\"./assets/b.png\" alt=\"b\"></picture>\n"),
        ("ec-srcset", "<img src=\"./assets/b.png\" srcset=\"https://tracker.test/s.png 2x\" alt=\"b\">\n"),
    ] {
        let dir = env.draft(id, &question_md(id, "要不要加碼？", body));
        std::fs::write(dir.join("assets/b.png"), PNG_1X1).unwrap();
        let out = env.register(&dir).await;
        assert!(!out.ok, "{id}：{out:?}");
    }
}

/// 每次渲染都卡住，直到測試放行（模擬真實渲染要 2～12 秒）。
struct GatedRenderer {
    tx: mpsc::UnboundedSender<oneshot::Sender<()>>,
}
impl CanvasRenderer for GatedRenderer {
    fn render(&self, _req: RenderRequest) -> kunsu_server::draft::BoxFuture<'_, RenderReport> {
        Box::pin(async move {
            let (gate, wait) = oneshot::channel();
            let _ = self.tx.send(gate);
            let _ = wait.await;
            RenderReport { issues: vec![], height: Some(100), preview_written: false }
        })
    }
}

async fn gated() -> (Env, mpsc::UnboundedReceiver<oneshot::Sender<()>>) {
    let env = Env::new().await;
    let (tx, rx) = mpsc::unbounded_channel();
    env.ctx.handle.set_canvas_renderer(Arc::new(GatedRenderer { tx }));
    (env, rx)
}

fn spawn_register(env: &Env, dir: &Path) -> tokio::task::JoinHandle<RegisterOutcome> {
    let h = env.ctx.handle.clone();
    let d = dir.to_path_buf();
    tokio::spawn(async move { h.register_draft(&d, RegisterOptions::default(), now()).await.unwrap() })
}

/// AC-EC-04：兩次重新註冊重疊時，不同內容拿到不同 revision。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_reregistration_gets_distinct_revisions() {
    let (env, mut rx) = gated().await;
    let dir = env.draft("ec-race", &question_md("ec-race", "要不要加碼？", "第一版\n"));
    let t = spawn_register(&env, &dir);
    rx.recv().await.unwrap().send(()).unwrap();
    assert!(t.await.unwrap().ok);
    std::fs::write(dir.join("question.md"), question_md("ec-race", "要不要加碼？", "第二版\n")).unwrap();
    let tb = spawn_register(&env, &dir);
    let gate_b = rx.recv().await.unwrap();
    std::fs::write(dir.join("question.md"), question_md("ec-race", "要不要加碼？", "第三版\n")).unwrap();
    let tc = spawn_register(&env, &dir);
    let gate_c = rx.recv().await.unwrap();
    gate_b.send(()).unwrap();
    let b = tb.await.unwrap();
    gate_c.send(()).unwrap();
    let c = tc.await.unwrap();
    assert!(b.ok && c.ok, "{b:?} {c:?}");
    assert_ne!(b.revision, c.revision, "不同內容不能共用同一個 revision");
    let q = env.ctx.handle.get_question("ec-race").await.unwrap().unwrap();
    assert_eq!(q.draft.unwrap().revision, 3);
    assert!(q.body_md.contains("第三版"));
    let snap2 = std::fs::read_to_string(env.home.path().join("revisions/ec-race/2/question.md")).unwrap();
    assert!(snap2.contains("第二版"), "{snap2}");
}

/// AC-EC-05：使用者答了第二版之後，重疊中的另一次註冊不能把第二版的快照蓋掉。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn answered_revision_snapshot_is_never_overwritten() {
    let (env, mut rx) = gated().await;
    let dir = env.draft("ec-snap", &question_md("ec-snap", "要不要加碼？", "第一版\n"));
    let t = spawn_register(&env, &dir);
    rx.recv().await.unwrap().send(()).unwrap();
    assert!(t.await.unwrap().ok);
    std::fs::write(dir.join("question.md"), question_md("ec-snap", "要不要加碼？", "第二版\n")).unwrap();
    let tb = spawn_register(&env, &dir);
    let gate_b = rx.recv().await.unwrap();
    std::fs::write(dir.join("question.md"), question_md("ec-snap", "要不要加碼？", "第三版\n")).unwrap();
    let tc = spawn_register(&env, &dir);
    let gate_c = rx.recv().await.unwrap();
    gate_b.send(()).unwrap();
    assert_eq!(tb.await.unwrap().revision, Some(2));
    let seen = seen_answer(&env, "ec-snap", "a").await;
    env.ctx.handle.answer_question("ec-snap", seen, now()).await.unwrap();
    gate_c.send(()).unwrap();
    let c = tc.await.unwrap();
    assert!(!c.ok, "已結案的題不能再更新：{c:?}");
    let snap = std::fs::read_to_string(env.home.path().join("revisions/ec-snap/2/question.md")).unwrap();
    assert!(snap.contains("第二版") && !snap.contains("第三版"), "{snap}");
    assert!(!env.home.path().join("revisions/ec-snap/3").exists(), "結案後不留新版快照");
}

/// AC-EC-04：上次中斷留下的版本資料夾不會被覆寫，改用下一個版本號。
#[tokio::test]
async fn leftover_revision_folder_is_never_overwritten() {
    let env = Env::new().await;
    let dir = env.draft("ec-left", &question_md("ec-left", "要不要加碼？", "第一版\n"));
    assert!(env.register(&dir).await.ok);
    let leftover = env.home.path().join("revisions/ec-left/2");
    std::fs::create_dir_all(&leftover).unwrap();
    std::fs::write(leftover.join("question.md"), "中斷留下的").unwrap();
    std::fs::write(dir.join("question.md"), question_md("ec-left", "要不要加碼？", "第二版\n")).unwrap();
    let out = env.register(&dir).await;
    assert!(out.ok, "{out:?}");
    assert_eq!(out.revision, Some(3));
    assert_eq!(std::fs::read_to_string(leftover.join("question.md")).unwrap(), "中斷留下的");
    let snap = std::fs::read_to_string(env.home.path().join("revisions/ec-left/3/question.md")).unwrap();
    assert!(snap.contains("第二版"));
}

/// AC-EC-08（QA 第 3 輪）：逐行比對漏掉的載圖寫法——`<image>`、屬性值含 `>`、下一行的 srcset——註冊時一樣擋。
#[tokio::test]
async fn image_tags_hidden_from_a_line_scan_are_rejected() {
    let env = Env::new().await;
    for (id, body) in [
        ("ec-image-tag", "<image src=\"https://tracker.test/a.png\">\n"),
        ("ec-gt-attr", "<img alt=\"a>b\" src=\"https://tracker.test/b.png\">\n"),
        ("ec-srcset-next-line", "<img src=\"./assets/x.png\"\n  srcset=\"https://tracker.test/c.png 2x\">\n"),
    ] {
        let dir = env.draft(id, &question_md(id, "要不要加碼？", body));
        std::fs::write(dir.join("assets/x.png"), PNG_1X1).unwrap();
        let out = env.register(&dir).await;
        assert!(!out.ok, "{id} 註冊成功了：{out:?}");
    }
}
