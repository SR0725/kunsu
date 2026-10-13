//! 草稿註冊與證據畫布（PRD §24；BDD evidence-canvas）。
//!
//! AI 在 `~/.kunsu/drafts/<id>/` 寫 `question.md`（YAML frontmatter＋Markdown 證據）、可選的
//! `canvas.tsx|jsx|html` 與 `assets/`；註冊時 Kunsu 逐項檢查並真的渲染一次（renderer 由 app 殼層注入），
//! 有問題逐行回報，通過才建題或更新成新版本。每版快照在 `~/.kunsu/revisions/<id>/<revision>/`。

use std::collections::BTreeMap;
use std::future::Future;
use std::path::{Component, Path, PathBuf};
use std::pin::Pin;

use base64::Engine as _;
use kunsu_core::types::{Answer, CanvasKind, CreateRequest, DraftRef, Question, State};
use kunsu_core::validate::{validate_create, RejectReason};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

/// 草稿與版本快照的根目錄。
#[derive(Debug, Clone)]
pub struct DraftPaths {
    pub drafts_root: PathBuf,
    pub revisions_root: PathBuf,
}

impl DraftPaths {
    /// `~/.kunsu/drafts` 與 `~/.kunsu/revisions`。
    pub fn default_home() -> DraftPaths {
        let home = std::env::var("HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("/"));
        DraftPaths {
            drafts_root: home.join(".kunsu/drafts"),
            revisions_root: home.join(".kunsu/revisions"),
        }
    }
}

/// 單檔資產上限 2 MB、合計 8 MB（AC-EC-08）。
pub const MAX_ASSET_BYTES: u64 = 2 * 1024 * 1024;
pub const MAX_ASSETS_TOTAL_BYTES: u64 = 8 * 1024 * 1024;
/// 畫布原始碼上限。
pub const MAX_CANVAS_BYTES: u64 = 200 * 1024;
/// 畫布內容高度上限（px）。
pub const MAX_CANVAS_HEIGHT: u32 = 2400;
/// 預覽截圖檔名（寫在草稿資料夾內，讓 agent 自己看畫面）。
pub const PREVIEW_FILE: &str = ".kunsu-preview.png";

/// 一個要 agent 修正的問題：檔案、位置、代碼、說明與改法。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DraftIssue {
    pub file: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub column: Option<u32>,
    pub code: String,
    pub message: String,
    pub hint: String,
}

/// 畫布原始碼。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanvasSource {
    pub kind: CanvasKind,
    pub source: String,
}

/// 面板顯示證據所需的一切：畫布與資產（`./assets/x.png` → data URI）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceBundle {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canvas: Option<CanvasSource>,
    #[serde(default)]
    pub assets: BTreeMap<String, String>,
}

/// 交給 renderer 的渲染檢查請求：候選題目（使用者會看到的樣子）＋證據；截圖寫到 `png_path`。
#[derive(Debug, Clone)]
pub struct RenderRequest {
    pub question: Question,
    pub evidence: EvidenceBundle,
    pub png_path: PathBuf,
}

/// renderer 的回報：畫布編譯／執行／版面問題，與是否寫出截圖。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderReport {
    #[serde(default)]
    pub issues: Vec<DraftIssue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    #[serde(default)]
    pub preview_written: bool,
}

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// 沙盒渲染檢查（app 殼層以隱藏 webview 實作；測試注入假的）。
pub trait CanvasRenderer: Send + Sync {
    fn render(&self, req: RenderRequest) -> BoxFuture<'_, RenderReport>;
}

/// `ask_register` / `kunsu ask <dir>` 的選項。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct RegisterOptions {
    #[serde(default)]
    pub dry_run: bool,
    /// 呼叫端補的來源身分（CLI 依環境補）；與 frontmatter 的 `source` 合併，呼叫端有填的欄位優先。
    #[serde(default)]
    pub source: Option<Value>,
}

/// 註冊結果。`ok=false` 時題目沒有建立或更新。
#[derive(Debug, Clone, Default, Serialize)]
pub struct RegisterOutcome {
    pub ok: bool,
    pub dry_run: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub question_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<u32>,
    /// created｜updated｜unchanged
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dedup: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<State>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview_png: Option<String>,
    pub issues: Vec<DraftIssue>,
    /// 題目已結案時回傳既有答案（AC-EC-04）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answer: Option<Answer>,
}

impl RegisterOutcome {
    pub(crate) fn failed(question_id: Option<String>, issues: Vec<DraftIssue>, preview_png: Option<String>) -> Self {
        RegisterOutcome { ok: false, question_id, issues, preview_png, ..Default::default() }
    }
}

impl DraftIssue {
    pub(crate) fn new(file: &str, line: Option<u32>, column: Option<u32>, code: &str, message: impl Into<String>, hint: impl Into<String>) -> Self {
        DraftIssue {
            file: file.to_string(),
            line,
            column,
            code: code.to_string(),
            message: message.into(),
            hint: hint.into(),
        }
    }
}

const QUESTION_FILE: &str = "question.md";
const CANVAS_KINDS: [CanvasKind; 3] = [CanvasKind::Tsx, CanvasKind::Jsx, CanvasKind::Html];

/// 一個被引用到的資產。
#[derive(Debug, Clone)]
pub(crate) struct AssetFile {
    /// `assets/` 底下的相對路徑（`b.png`、`img/c.png`）。
    pub rel: String,
    pub bytes: Vec<u8>,
    pub mime: &'static str,
}

impl AssetFile {
    fn key(&self) -> String {
        format!("./assets/{}", self.rel)
    }
    fn data_uri(&self) -> String {
        format!("data:{};base64,{}", self.mime, base64::engine::general_purpose::STANDARD.encode(&self.bytes))
    }
}

/// 讀好、檢查過的草稿。
#[derive(Debug, Clone)]
pub(crate) struct LoadedDraft {
    pub dir: PathBuf,
    pub req: CreateRequest,
    pub question_md: Vec<u8>,
    pub canvas: Option<CanvasSource>,
    pub assets: Vec<AssetFile>,
    pub content_hash: String,
}

impl LoadedDraft {
    pub fn evidence(&self) -> EvidenceBundle {
        EvidenceBundle {
            canvas: self.canvas.clone(),
            assets: self.assets.iter().map(|a| (a.key(), a.data_uri())).collect(),
        }
    }

    pub fn draft_ref(&self, revision: u32, now: kunsu_core::time::Ts) -> DraftRef {
        DraftRef {
            revision,
            content_hash: self.content_hash.clone(),
            canvas: self.canvas.as_ref().map(|c| c.kind),
            registered_at: now,
        }
    }

    pub fn preview_path(&self) -> PathBuf {
        self.dir.join(PREVIEW_FILE)
    }
}

/// `~/` 開頭的路徑展開成家目錄。
pub fn expand_home(path: &str) -> PathBuf {
    match path.strip_prefix("~/") {
        Some(rest) => std::env::var("HOME").map(|h| PathBuf::from(h).join(rest)).unwrap_or_else(|_| PathBuf::from(path)),
        None => PathBuf::from(path),
    }
}

fn outside(dir: &Path, root: &Path) -> DraftIssue {
    DraftIssue::new(
        QUESTION_FILE,
        None,
        None,
        "draft_outside_root",
        format!("草稿 {} 不在 {} 底下（或是指向外面的連結）", dir.display(), root.display()),
        format!("把草稿寫在 {}/<題目 id>/，檔案不要用 symlink", root.display()),
    )
}

/// 讀取並檢查草稿（不含沙盒渲染）。有任何問題 → Err(issues)。
pub(crate) fn load(dir: &Path, paths: &DraftPaths, caller_source: Option<&Value>) -> Result<LoadedDraft, Vec<DraftIssue>> {
    let root = paths.drafts_root.canonicalize().map_err(|_| vec![outside(dir, &paths.drafts_root)])?;
    let dir = dir.canonicalize().map_err(|_| vec![outside(dir, &root)])?;
    if !dir.starts_with(&root) || dir == root {
        return Err(vec![outside(&dir, &root)]);
    }
    let folder = dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let mut issues = Vec::new();

    // question.md（必須在草稿資料夾內，不跟 symlink 出去）。
    let question_path = dir.join(QUESTION_FILE);
    let question_md = match read_inside(&question_path, &dir) {
        Ok(Some(bytes)) => bytes,
        Ok(None) => {
            return Err(vec![DraftIssue::new(
                QUESTION_FILE,
                None,
                None,
                "question_missing",
                "草稿資料夾裡沒有 question.md",
                "建立 question.md：開頭是 --- 包住的 YAML（id、title、question_line、kind、options…），之後是 Markdown 證據",
            )]);
        }
        Err(()) => return Err(vec![outside(&question_path, &root)]),
    };
    let Ok(text) = String::from_utf8(question_md.clone()) else {
        return Err(vec![DraftIssue::new(QUESTION_FILE, None, None, "encoding", "question.md 不是 UTF-8", "用 UTF-8 存檔")]);
    };

    // 畫布：最多一個。
    let mut canvas: Option<CanvasSource> = None;
    for kind in CANVAS_KINDS {
        let path = dir.join(kind.file_name());
        let bytes = match read_inside(&path, &dir) {
            Ok(Some(b)) => b,
            Ok(None) => continue,
            Err(()) => return Err(vec![outside(&path, &root)]),
        };
        if let Some(first) = &canvas {
            issues.push(DraftIssue::new(
                kind.file_name(),
                None,
                None,
                "canvas_multiple",
                format!("同時有 {} 與 {}，畫布只能有一個", first.kind.file_name(), kind.file_name()),
                "留下一個 canvas.tsx／canvas.jsx／canvas.html，其他刪掉或改名",
            ));
            continue;
        }
        if bytes.len() as u64 > MAX_CANVAS_BYTES {
            issues.push(DraftIssue::new(
                kind.file_name(),
                None,
                None,
                "canvas_too_large",
                format!("畫布 {} KB，超過 {} KB 上限", bytes.len() / 1024, MAX_CANVAS_BYTES / 1024),
                "資料只放決策需要的彙總，原始明細留在 vault",
            ));
            continue;
        }
        match String::from_utf8(bytes) {
            Ok(source) => canvas = Some(CanvasSource { kind, source }),
            Err(_) => issues.push(DraftIssue::new(kind.file_name(), None, None, "encoding", "畫布不是 UTF-8", "用 UTF-8 存檔")),
        }
    }

    // frontmatter ＋ 內文。
    let fm = match split_frontmatter(&text) {
        Ok(fm) => fm,
        Err(issue) => {
            issues.push(issue);
            return Err(issues);
        }
    };
    let yaml: serde_yaml_ng::Value = match serde_yaml_ng::from_str(&fm.yaml) {
        Ok(v) => v,
        Err(e) => {
            let (line, column) = e
                .location()
                .map(|l| (Some(l.line() as u32 + fm.yaml_start_line - 1), Some(l.column() as u32)))
                .unwrap_or((Some(fm.yaml_start_line), None));
            issues.push(DraftIssue::new(
                QUESTION_FILE,
                line,
                column,
                "frontmatter_yaml",
                format!("YAML 格式錯誤：{e}"),
                "檢查縮排、冒號後要有空格、清單用 `- `、含冒號或 # 的字串加引號",
            ));
            return Err(issues);
        }
    };
    let mut args = match serde_json::to_value(&yaml) {
        Ok(Value::Object(m)) => m,
        _ => {
            issues.push(DraftIssue::new(
                QUESTION_FILE,
                Some(fm.yaml_start_line),
                None,
                "frontmatter_not_mapping",
                "frontmatter 必須是 key: value 的對照表",
                "照 ask_create 的欄位寫，例如 `question_line: 要不要加碼？`",
            ));
            return Err(issues);
        }
    };
    let key_line = |key: &str| fm.key_line(key);

    // id 預設是資料夾名稱；寫了就必須一致。
    match args.get("id").and_then(Value::as_str) {
        Some(id) if id != folder => issues.push(DraftIssue::new(
            QUESTION_FILE,
            key_line("id"),
            None,
            "id_mismatch",
            format!("frontmatter 的 id {id:?} 和資料夾名稱 {folder:?} 不同"),
            "資料夾名稱就是題目 id：改其中一個讓兩者相同，或刪掉 id 這行",
        )),
        Some(_) => {}
        None => {
            args.insert("id".into(), json!(folder));
        }
    }
    if let Some(why) = args.remove("why")
        && !args.contains_key("why_md")
    {
        args.insert("why_md".into(), why);
    }
    args.insert("body_md".into(), json!(fm.body.trim_end()));
    if !args.contains_key("title")
        && let Some(line) = args.get("question_line").cloned()
    {
        args.insert("title".into(), line);
    }
    if !args.contains_key("tier") && !args.contains_key("requested_tier") {
        args.insert("tier".into(), json!("t1"));
    }
    args.entry("default_action").or_insert_with(|| json!({"on_expire": "noop_log"}));
    let mut source = match args.remove("source") {
        Some(Value::Object(m)) => m,
        _ => Map::new(),
    };
    if let Some(Value::Object(caller)) = caller_source {
        for (k, v) in caller {
            if !v.is_null() {
                source.insert(k.clone(), v.clone());
            }
        }
    }
    source.entry("agent").or_insert_with(|| json!("other"));
    source.entry("cwd").or_insert_with(|| json!(dir.to_string_lossy()));
    args.insert("source".into(), Value::Object(source));

    let args = crate::mcp::normalize_create_args(Value::Object(args));
    // 錯誤訊息帶欄位路徑（例如 source.expected_every_hours），agent 才知道要改哪裡。
    let req: CreateRequest = match serde_path_to_error::deserialize(args) {
        Ok(r) => r,
        Err(e) => {
            let path = e.path().to_string();
            let inner = e.into_inner().to_string();
            let token = inner.split('`').nth(1).unwrap_or("");
            let msg = if path.is_empty() || path == "." { inner.clone() } else { format!("{path}：{inner}") };
            let token = if token.is_empty() { path.rsplit('.').next().unwrap_or("") } else { token };
            let line = if token.is_empty() { None } else { fm.key_line(token).or_else(|| fm.value_line(token)) };
            issues.push(DraftIssue::new(
                QUESTION_FILE,
                line.or(Some(fm.yaml_start_line)),
                None,
                "frontmatter_field",
                format!("欄位不合法：{msg}"),
                "欄位與 ask_create 相同：id、title、question_line、kind（choice｜confirm｜text…）、options[{id,label}]、money…",
            ));
            return Err(issues);
        }
    };
    if let Err(reason) = validate_create(&req) {
        let (key, fallback) = reject_key(&reason);
        // 題組單項的問題對到該項 `- id:` 那一行。
        let item_line = match &reason {
            RejectReason::BatchItem { item_id, .. } => fm.item_line(item_id),
            _ => None,
        };
        issues.push(DraftIssue::new(
            QUESTION_FILE,
            item_line.or_else(|| key_line(key)).or_else(|| key_line(fallback)).or(Some(fm.yaml_start_line)),
            None,
            reason.code(),
            reason.to_string(),
            reject_hint(&reason),
        ));
    }

    // Markdown（why 與內文）安全與資產引用。
    let mut refs: Vec<AssetRef> = Vec::new();
    if let Some(why) = req.why_md.as_deref() {
        let why_line = key_line("why").or_else(|| key_line("why_md")).unwrap_or(fm.yaml_start_line);
        for mut issue in lint_markdown(why, QUESTION_FILE, 0, &mut refs) {
            issue.line = Some(why_line);
            issue.column = None;
            issues.push(issue);
        }
    }
    issues.extend(lint_markdown(&fm.body, QUESTION_FILE, fm.body_start_line - 1, &mut refs));
    // 面板也用 Markdown 渲染這些 frontmatter 欄位：同樣的安全與圖片檢查，錯誤對到欄位的行號（AC-EC-08）。
    let mut field_texts: Vec<(&str, &str)> = vec![("question_line", req.question_line.as_str())];
    field_texts.extend(req.evidence.iter().map(|e| ("evidence", e.text.as_str())));
    for o in &req.options {
        field_texts.push(("options", o.label.as_str()));
        if let Some(d) = o.detail.as_deref() {
            field_texts.push(("options", d));
        }
    }
    for (key, text) in field_texts {
        let line = key_line(key).unwrap_or(fm.yaml_start_line);
        for mut issue in lint_markdown(text, QUESTION_FILE, 0, &mut refs) {
            issue.line = Some(line);
            issue.column = None;
            issues.push(issue);
        }
    }
    // 題組每項的原文（CONTRACT §14）：同樣的安全與圖片檢查，問題對到該項 `- id:` 那一行；引用的圖片收進快照。
    for (index, item) in req.items.iter().enumerate() {
        let line = fm.item_line(&item.id).or_else(|| key_line("items")).unwrap_or(fm.yaml_start_line);
        let first_ref = refs.len();
        for mut issue in lint_markdown(&item.context_md, QUESTION_FILE, 0, &mut refs) {
            issue.line = Some(line);
            issue.column = None;
            issue.message = format!("items 第 {} 項（{}）的 context_md：{}", index + 1, item.id, issue.message);
            issues.push(issue);
        }
        for r in &mut refs[first_ref..] {
            r.line = line;
            r.column = 1;
        }
    }
    if let Some(c) = &canvas {
        issues.extend(scan_canvas(c, &mut refs));
    }

    // 資產：只收有引用到的；路徑在 assets/ 內、存在、大小上限、圖片格式。
    let mut assets: Vec<AssetFile> = Vec::new();
    let assets_root = dir.join("assets");
    let assets_linked = std::fs::symlink_metadata(&assets_root).is_ok_and(|m| m.file_type().is_symlink());
    let mut total: u64 = 0;
    for r in &refs {
        if assets.iter().any(|a| a.rel == r.rel) {
            continue;
        }
        let issue = |code: &str, message: String, hint: &str| DraftIssue::new(&r.file, Some(r.line), Some(r.column), code, message, hint);
        if Path::new(&r.rel).components().any(|c| !matches!(c, Component::Normal(_))) {
            issues.push(issue("asset_outside_draft", format!("資產路徑 ./assets/{} 跑出 assets 資料夾", r.rel), "資產只能放在草稿的 assets/ 裡，用 ./assets/<檔名> 引用"));
            continue;
        }
        if assets_linked {
            issues.push(issue("asset_outside_draft", "assets/ 是指向草稿外面的連結".to_string(), "直接在草稿裡建 assets/ 資料夾並把圖片複製進去"));
            continue;
        }
        let path = assets_root.join(&r.rel);
        if !path.is_file() {
            issues.push(issue("asset_missing", format!("找不到 ./assets/{}", r.rel), "把檔案放進草稿的 assets/，或修正檔名（大小寫要一致）"));
            continue;
        }
        let inside = path.canonicalize().ok().zip(assets_root.canonicalize().ok()).is_some_and(|(p, a)| p.starts_with(a));
        if !inside {
            issues.push(issue("asset_outside_draft", format!("./assets/{} 指向草稿以外的位置", r.rel), "直接把檔案複製進 assets/，不要用 symlink"));
            continue;
        }
        let Some(mime) = image_mime(&r.rel) else {
            issues.push(issue("asset_type", format!("./assets/{} 不是支援的圖片格式", r.rel), "支援 png、jpg、gif、webp、svg；數據請直接寫進畫布或 Markdown 表格"));
            continue;
        };
        let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        if size > MAX_ASSET_BYTES {
            issues.push(issue("asset_too_large", format!("./assets/{} 有 {:.1} MB，單檔上限 2 MB", r.rel, size as f64 / 1048576.0), "壓縮或縮小圖片（寬 1600 px 以內通常就夠）"));
            continue;
        }
        total += size;
        match std::fs::read(&path) {
            Ok(bytes) => assets.push(AssetFile { rel: r.rel.clone(), bytes, mime }),
            Err(e) => issues.push(issue("asset_unreadable", format!("讀不到 ./assets/{}：{e}", r.rel), "確認檔案權限")),
        }
    }
    if total > MAX_ASSETS_TOTAL_BYTES {
        issues.push(DraftIssue::new(
            "assets/",
            None,
            None,
            "assets_total_too_large",
            format!("引用的資產合計 {:.1} MB，上限 8 MB", total as f64 / 1048576.0),
            "只放決策需要的圖，或壓縮圖片",
        ));
    }

    if !issues.is_empty() {
        return Err(issues);
    }
    assets.sort_by(|a, b| a.rel.cmp(&b.rel));
    let content_hash = hash_content(&question_md, canvas.as_ref(), &assets);
    Ok(LoadedDraft { dir, req, question_md, canvas, assets, content_hash })
}

/// 讀檔：不存在 → None；跟 symlink 跑出 `dir` → Err。
fn read_inside(path: &Path, dir: &Path) -> Result<Option<Vec<u8>>, ()> {
    if std::fs::symlink_metadata(path).is_err() {
        return Ok(None);
    }
    let real = path.canonicalize().map_err(|_| ())?;
    if !real.starts_with(dir) || !real.is_file() {
        return Err(());
    }
    std::fs::read(&real).map(Some).map_err(|_| ())
}

struct Frontmatter {
    yaml: String,
    body: String,
    /// YAML 第一行在檔案中的行號（1-based）。
    yaml_start_line: u32,
    body_start_line: u32,
}

impl Frontmatter {
    /// 頂層 `key:` 所在的檔案行號。
    fn key_line(&self, key: &str) -> Option<u32> {
        self.yaml.lines().position(|l| {
            l.strip_prefix(key).is_some_and(|rest| rest.trim_start().starts_with(':'))
        })
        .map(|i| i as u32 + self.yaml_start_line)
    }

    /// 題組某一項（`- id: <id>`，縮排在 items 底下）所在的檔案行號。
    fn item_line(&self, id: &str) -> Option<u32> {
        self.yaml
            .lines()
            .position(|l| {
                if !l.starts_with([' ', '\t', '-']) {
                    return false;
                }
                let t = l.trim_start();
                let t = t.strip_prefix('-').map(str::trim_start).unwrap_or(t);
                t.strip_prefix("id:").is_some_and(|v| v.trim().trim_matches(['"', '\'']) == id)
            })
            .map(|i| i as u32 + self.yaml_start_line)
    }

    /// 某個值第一次出現的行號（serde 錯誤只給值時用）。
    fn value_line(&self, value: &str) -> Option<u32> {
        self.yaml.lines().position(|l| l.contains(value)).map(|i| i as u32 + self.yaml_start_line)
    }
}

fn split_frontmatter(text: &str) -> Result<Frontmatter, DraftIssue> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let lines: Vec<&str> = text.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l)).collect();
    let missing = || {
        DraftIssue::new(
            QUESTION_FILE,
            Some(1),
            Some(1),
            "frontmatter_missing",
            "question.md 要以 --- 開頭的 YAML frontmatter 起頭，並以另一行 --- 結束",
            "第一行寫 ---，接著 id／title／question_line／kind／options…，再一行 ---，之後寫 Markdown 證據",
        )
    };
    if lines.first().map(|l| l.trim_end()) != Some("---") {
        return Err(missing());
    }
    let close = lines.iter().skip(1).position(|l| matches!(l.trim_end(), "---" | "...")).map(|i| i + 1).ok_or_else(missing)?;
    Ok(Frontmatter {
        yaml: lines[1..close].join("\n"),
        body: lines[close + 1..].join("\n"),
        yaml_start_line: 2,
        body_start_line: close as u32 + 2,
    })
}

fn reject_key(reason: &RejectReason) -> (&'static str, &'static str) {
    use RejectReason::*;
    match reason {
        IdEmpty | IdInvalidFormat { .. } => ("id", "id"),
        BadPrepareCmd { .. } | PrepareCmdEntityMismatch { .. } => ("money", "money"),
        TitleTooLong { .. } => ("title", "question_line"),
        QuestionLineTooLong { .. } => ("question_line", "question_line"),
        VisualSymbolTooLong { .. } => ("visual_symbol", "visual_symbol"),
        FreeTextMustBeAllowed => ("allow_free_text", "allow_free_text"),
        TooManyEvidence { .. } | EvidenceMissingSource { .. } => ("evidence", "evidence"),
        MoneyRequiresPrediction => ("prediction_ref", "money"),
        MoneyNeverAutoExpire | PickOptionNeedsOptionId | PickOptionUnknownOptionId { .. } => ("default_action", "default_action"),
        RedGateNeedsChoice => ("gate", "kind"),
        ChoiceOptionsOutOfRange { .. } => ("options", "kind"),
        MoneyNeedsParamsLine => ("params_line", "money"),
        MoneyNeedsExecuteOption => ("options", "money"),
        MoneyNeedsPrepareCmd => ("money", "money"),
        BadConversationIdentity { .. } | BadRoutineCadence => ("source", "source"),
        BatchItemsOutOfRange { .. } | BatchItem { .. } => ("items", "kind"),
        ItemsOnlyForBatch { .. } | BatchMoneyForbidden => ("kind", "items"),
        BatchNoOptions { .. } => ("options", "kind"),
        BatchKongkoRef => ("kongko_ref", "kind"),
        BatchIdTooLong { .. } => ("id", "id"),
    }
}

fn reject_hint(reason: &RejectReason) -> String {
    use RejectReason::*;
    match reason {
        QuestionLineTooLong { .. } => "決策句 24 字以內，一句話講完要使用者決定什麼；細節放 why 或內文".into(),
        TitleTooLong { .. } => "title 80 字以內".into(),
        ChoiceOptionsOutOfRange { .. } => "choice 題要有 2～4 個 options，每個寫 id 與 label；看起來像按鈕的畫布不算選項".into(),
        MoneyNeedsParamsLine => "動錢題要有 params_line：一行寫清楚改什麼（例如 adset X 日預算 500 → 600）".into(),
        MoneyRequiresPrediction => "動錢題要附 prediction_ref（vault 預測檔路徑）".into(),
        MoneyNeedsPrepareCmd => "黃燈錢題要在 money.executor_prepare_cmd 寫完整 executor.py prepare 命令；不能一鍵執行就設 gate: red".into(),
        MoneyNeedsExecuteOption => "動錢題用 kind: choice，執行的那個選項加 recommended: true，另給一個安全選項（不動錢）".into(),
        EvidenceMissingSource { .. } => "evidence 每條都要 source（資料出處）".into(),
        TooManyEvidence { .. } => "evidence 最多 3 條；其他數據放內文或畫布".into(),
        BatchItemsOutOfRange { .. } => "題組放 1–20 項".into(),
        BatchMoneyForbidden => "動錢的事一題一題建（走批准授權鏈），不要放進題組".into(),
        BatchKongkoRef => "題組拿掉 kongko_ref：題組只能在 Kunsu 面板逐項作答".into(),
        BatchIdTooLong { .. } => "題組 id 縮短到 116 字以內（不含 -rN），留空間給第 2 輪以後的 -r2".into(),
        BatchItem { .. } => "照訊息修正這一項後重新註冊".into(),
        _ => "照訊息修正這個欄位後重新註冊".into(),
    }
}

/// 一個資產引用的位置。
#[derive(Debug, Clone)]
struct AssetRef {
    rel: String,
    file: String,
    line: u32,
    column: u32,
}

const FORBIDDEN_TAGS: [&str; 24] = [
    "script", "style", "iframe", "form", "button", "input", "textarea", "select", "object", "embed", "link", "meta",
    "base", "frame", "frameset", "svg", "math", "template", "video", "audio", "canvas", "portal", "picture", "source",
];

fn char_col(line: &str, byte: usize) -> u32 {
    line[..byte].chars().count() as u32 + 1
}

/// JSON 建題的 Markdown 安全檢查（題組原文，CONTRACT §14）：沿用草稿規則；JSON 題沒有草稿資料夾，所以不能引用圖片。
/// 有問題回第一個問題的說明。
pub(crate) fn markdown_problem(text: &str) -> Option<String> {
    let mut refs = Vec::new();
    let issues = lint_markdown(text, QUESTION_FILE, 0, &mut refs);
    if let Some(issue) = issues.first() {
        return Some(format!("{}（{}）", issue.message, issue.hint));
    }
    (!refs.is_empty()).then(|| "JSON 建題的原文不能引用圖片；要附圖請用草稿註冊（kunsu ask <草稿資料夾>）".to_string())
}

/// Markdown 安全檢查與資產引用收集。`line_offset` 加到 1-based 行號上。
fn lint_markdown(text: &str, file: &str, line_offset: u32, refs: &mut Vec<AssetRef>) -> Vec<DraftIssue> {
    let mut issues = Vec::new();
    let mut fenced = false;
    for (i, line) in text.split('\n').enumerate() {
        let ln = i as u32 + 1 + line_offset;
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        let lower = line.to_ascii_lowercase();
        // 禁用標籤。
        let mut from = 0;
        while let Some(pos) = lower[from..].find('<') {
            let at = from + pos;
            let rest = &lower[at + 1..];
            let rest = rest.strip_prefix('/').unwrap_or(rest);
            if let Some(tag) = FORBIDDEN_TAGS.iter().find(|t| {
                rest.starts_with(**t) && rest[t.len()..].chars().next().is_none_or(|c| !c.is_ascii_alphanumeric())
            }) {
                issues.push(DraftIssue::new(
                    file,
                    Some(ln),
                    Some(char_col(line, at)),
                    "markdown_forbidden_tag",
                    format!("內文不能用 <{tag}>"),
                    "互動、圖表與排版請寫在 canvas.tsx；內文只放文字、表格、清單與圖片",
                ));
                break;
            }
            // 標籤內的事件處理、style、class。
            if rest.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
                let end = rest.find('>').unwrap_or(rest.len());
                let inner = &rest[..end];
                let bad = inner.split_whitespace().skip(1).find(|attr| {
                    let name = attr.split('=').next().unwrap_or("");
                    // srcset 會繞過「只顯示快照」載入外部圖片（AC-EC-08）。
                    (name.starts_with("on") && name.len() > 2) || name == "style" || name == "class" || name == "srcset"
                });
                if let Some(attr) = bad {
                    let name = attr.split('=').next().unwrap_or(attr);
                    issues.push(DraftIssue::new(
                        file,
                        Some(ln),
                        Some(char_col(line, at)),
                        "markdown_forbidden_attr",
                        format!("內文的 HTML 不能帶 {name} 屬性"),
                        "Kunsu 決定版面與樣式；要視覺化就寫 canvas.tsx",
                    ));
                    break;
                }
            }
            from = at + 1;
        }
        if let Some(pos) = lower.find("javascript:") {
            issues.push(DraftIssue::new(file, Some(ln), Some(char_col(line, pos)), "markdown_forbidden_url", "內文不能有 javascript: 連結", "連結只用 https://"));
        }
    }
    // 圖片：用真正的 Markdown 解析（含參照式圖片 `![x][r]` 與跨行 `<img>`），不靠逐行字串比對。
    for (offset, url) in markdown_images(text) {
        let (ln, col) = line_col(text, offset);
        check_image(file, ln + line_offset, col, &url, refs, &mut issues);
    }
    issues
}

/// 位元組位移 → (1-based 行, 1-based 字元欄)。
fn line_col(text: &str, offset: usize) -> (u32, u32) {
    let offset = offset.min(text.len());
    let before = &text[..offset];
    let line = before.matches('\n').count() as u32 + 1;
    let line_start = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
    (line, text[line_start..offset].chars().count() as u32 + 1)
}

/// 內文所有圖片網址與位置：Markdown 圖片（行內與參照式）以及 HTML `<img src>`（可跨行）。
fn markdown_images(text: &str) -> Vec<(usize, String)> {
    use pulldown_cmark::{Event, Options, Parser, Tag};
    let mut out = Vec::new();
    let mut html_spans: Vec<(usize, String)> = Vec::new();
    for (event, range) in Parser::new_ext(text, Options::all()).into_offset_iter() {
        match event {
            Event::Start(Tag::Image { dest_url, .. }) => out.push((range.start, dest_url.to_string())),
            Event::Html(h) | Event::InlineHtml(h) => html_spans.push((range.start, h.to_string())),
            _ => {}
        }
    }
    // HTML 區塊可能被切成好幾段，合併連續段落再找 <img … src=…>。
    let mut merged: Vec<(usize, String)> = Vec::new();
    for (start, h) in html_spans {
        match merged.last_mut() {
            Some((s0, acc)) if *s0 + acc.len() >= start => acc.push_str(&h),
            _ => merged.push((start, h)),
        }
    }
    for (start, h) in merged {
        for (at, url) in html_image_urls(&h) {
            out.push((start + at, url));
        }
    }
    out.sort_by_key(|(o, _)| *o);
    out
}

/// HTML 片段裡 `<img>`／`<image>` 的圖片網址（`src` 與 `srcset` 的每個候選），引號感知：屬性值裡的 `>` 不會提早結束
/// 標籤，屬性可以跨行（AC-EC-08）。回傳（標籤在片段中的位移, 網址）。
fn html_image_urls(h: &str) -> Vec<(usize, String)> {
    let bytes = h.as_bytes();
    let lower = h.to_ascii_lowercase();
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(p) = lower[i..].find('<') {
        let at = i + p;
        let rest = &lower[at + 1..];
        let name_len = rest.find(|c: char| !c.is_ascii_alphanumeric()).unwrap_or(rest.len());
        let name = &rest[..name_len];
        i = at + 1;
        if name != "img" && name != "image" {
            continue;
        }
        // 解析屬性直到標籤結束（引號內的 `>` 不算）。
        let mut k = at + 1 + name_len;
        let mut attrs: Vec<(String, String)> = Vec::new();
        while k < bytes.len() {
            while k < bytes.len() && (bytes[k] as char).is_ascii_whitespace() {
                k += 1;
            }
            if k >= bytes.len() || bytes[k] == b'>' {
                break;
            }
            if bytes[k] == b'/' {
                k += 1;
                continue;
            }
            let ns = k;
            while k < bytes.len() && !(bytes[k] as char).is_ascii_whitespace() && !matches!(bytes[k], b'=' | b'>' | b'/') {
                k += 1;
            }
            let attr = lower[ns..k].to_string();
            while k < bytes.len() && (bytes[k] as char).is_ascii_whitespace() {
                k += 1;
            }
            let mut value = String::new();
            if k < bytes.len() && bytes[k] == b'=' {
                k += 1;
                while k < bytes.len() && (bytes[k] as char).is_ascii_whitespace() {
                    k += 1;
                }
                if k < bytes.len() && matches!(bytes[k], b'"' | b'\'') {
                    let q = bytes[k];
                    let vs = k + 1;
                    k = vs;
                    while k < bytes.len() && bytes[k] != q {
                        k += 1;
                    }
                    value = h[vs..k.min(h.len())].to_string();
                    k += 1;
                } else {
                    let vs = k;
                    while k < bytes.len() && !(bytes[k] as char).is_ascii_whitespace() && bytes[k] != b'>' {
                        k += 1;
                    }
                    value = h[vs..k].to_string();
                }
            }
            if !attr.is_empty() {
                attrs.push((attr, value));
            }
        }
        for (attr, value) in attrs {
            match attr.as_str() {
                "src" => out.push((at, value.trim().to_string())),
                "srcset" => {
                    for candidate in value.split(',') {
                        if let Some(url) = candidate.split_whitespace().next() {
                            out.push((at, url.to_string()));
                        }
                    }
                }
                _ => {}
            }
        }
        i = k.max(at + 1);
    }
    out
}

fn check_image(file: &str, ln: u32, col: u32, url: &str, refs: &mut Vec<AssetRef>, issues: &mut Vec<DraftIssue>) {
    let lower = url.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("//") {
        issues.push(DraftIssue::new(
            file,
            Some(ln),
            Some(col),
            "external_url",
            format!("圖片不能引用外部網址 {url}"),
            "下載到草稿的 assets/，改用 ./assets/<檔名>",
        ));
    } else if lower.starts_with("data:") {
        issues.push(DraftIssue::new(file, Some(ln), Some(col), "markdown_data_url", "內文不要直接嵌 data: 圖片", "把圖片存進 assets/，用 ./assets/<檔名> 引用"));
    } else if let Some(rel) = asset_rel(url) {
        refs.push(AssetRef { rel, file: file.to_string(), line: ln, column: col });
    } else {
        issues.push(DraftIssue::new(file, Some(ln), Some(col), "asset_outside_draft", format!("圖片 {url} 不在 ./assets/ 裡"), "圖片放進草稿的 assets/，用 ./assets/<檔名> 引用"));
    }
}

/// `./assets/x.png`／`assets/x.png` → `x.png`。
fn asset_rel(url: &str) -> Option<String> {
    let u = url.strip_prefix("./").unwrap_or(url);
    u.strip_prefix("assets/").filter(|r| !r.is_empty()).map(str::to_string)
}

/// 畫布：外部網址與資產引用。
fn scan_canvas(canvas: &CanvasSource, refs: &mut Vec<AssetRef>) -> Vec<DraftIssue> {
    let file = canvas.kind.file_name();
    let mut issues = Vec::new();
    for (i, line) in canvas.source.split('\n').enumerate() {
        let ln = i as u32 + 1;
        let lower = line.to_ascii_lowercase();
        let external = ["https://", "http://"].iter().filter_map(|p| lower.find(p)).filter(|pos| {
            !lower[*pos..].starts_with("http://www.w3.org/")
        }).min();
        if let Some(pos) = external {
            issues.push(DraftIssue::new(
                file,
                Some(ln),
                Some(char_col(line, pos)),
                "external_url",
                "畫布不能連外部網址（沙盒沒有網路）",
                "圖片放進 assets/ 用 './assets/<檔名>'；數據直接寫進畫布",
            ));
        }
        // 畫布不能自己換頁（meta refresh）：導覽白名單也會擋，這裡先告訴 agent。
        if let Some(pos) = lower.find("http-equiv").or_else(|| lower.find("httpequiv")) {
            issues.push(DraftIssue::new(
                file,
                Some(ln),
                Some(char_col(line, pos)),
                "canvas_navigation",
                "畫布不能用 meta http-equiv（refresh 會換頁）",
                "畫布只負責顯示；拿掉這個 meta",
            ));
        }
        let mut from = 0;
        while let Some(p) = line[from..].find("assets/") {
            let at = from + p;
            let start = if line[..at].ends_with("./") { at - 2 } else { at };
            let quoted = line[..start].ends_with(['"', '\'', '`']);
            let end = line[at..].find(['"', '\'', '`', ')', ' ']).map(|e| at + e).unwrap_or(line.len());
            if quoted && let Some(rel) = asset_rel(&line[start..end]) {
                refs.push(AssetRef { rel, file: file.to_string(), line: ln, column: char_col(line, start) });
            }
            from = end.max(at + 7);
        }
    }
    issues
}

fn image_mime(rel: &str) -> Option<&'static str> {
    let ext = rel.rsplit('.').next()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        _ => return None,
    })
}

fn hash_content(question_md: &[u8], canvas: Option<&CanvasSource>, assets: &[AssetFile]) -> String {
    let mut h = Sha256::new();
    h.update(b"question.md\0");
    h.update(question_md);
    if let Some(c) = canvas {
        h.update(b"\0");
        h.update(c.kind.file_name().as_bytes());
        h.update(b"\0");
        h.update(c.source.as_bytes());
    }
    for a in assets {
        h.update(b"\0assets/");
        h.update(a.rel.as_bytes());
        h.update(b"\0");
        h.update(&a.bytes);
    }
    format!("sha256:{}", hex::encode(h.finalize()))
}

/// 題目 id 能不能當資料夾名稱（與 core 的 id 白名單一致：ASCII 英數 . _ -、不以 . 開頭）。
fn safe_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && !id.starts_with('.')
        && id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

#[derive(Debug, Serialize, Deserialize)]
struct SnapshotMeta {
    revision: u32,
    content_hash: String,
    #[serde(default)]
    canvas: Option<CanvasKind>,
    registered_at: kunsu_core::time::Ts,
}

/// 寫入版本快照 `<revisions>/<id>/<revision>/`。
/// 版本快照原子寫入（呼叫端持有寫入鎖）：先寫暫存資料夾，再改名成第一個還不存在的版本號（≥ `first_revision`）；
/// 已存在的版本資料夾（包括上次中斷留下的）永不覆寫（AC-EC-04／05）。
pub(crate) fn commit_snapshot(
    paths: &DraftPaths,
    draft: &LoadedDraft,
    first_revision: u32,
    now: kunsu_core::time::Ts,
) -> std::io::Result<DraftRef> {
    if !safe_id(&draft.req.id) {
        return Err(std::io::Error::other("題目 id 不能當資料夾名稱"));
    }
    let root = paths.revisions_root.join(&draft.req.id);
    std::fs::create_dir_all(&root)?;
    let mut revision = first_revision.max(1);
    while std::fs::symlink_metadata(root.join(revision.to_string())).is_ok() {
        revision += 1;
    }
    let draft_ref = draft.draft_ref(revision, now);
    let tmp = root.join(format!(".tmp-{}", uuid::Uuid::new_v4()));
    let written = write_snapshot_into(&tmp, draft, &draft_ref).and_then(|()| std::fs::rename(&tmp, root.join(revision.to_string())));
    if written.is_err() {
        let _ = std::fs::remove_dir_all(&tmp);
    }
    written.map(|()| draft_ref)
}

fn write_snapshot_into(dir: &Path, draft: &LoadedDraft, draft_ref: &DraftRef) -> std::io::Result<()> {
    std::fs::create_dir_all(dir.join("assets"))?;
    std::fs::write(dir.join(QUESTION_FILE), &draft.question_md)?;
    if let Some(c) = &draft.canvas {
        std::fs::write(dir.join(c.kind.file_name()), &c.source)?;
    }
    for a in &draft.assets {
        let target = dir.join("assets").join(&a.rel);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(target, &a.bytes)?;
    }
    let meta = SnapshotMeta {
        revision: draft_ref.revision,
        content_hash: draft_ref.content_hash.clone(),
        canvas: draft_ref.canvas,
        registered_at: draft_ref.registered_at,
    };
    std::fs::write(dir.join("meta.json"), serde_json::to_vec_pretty(&meta).unwrap_or_default())
}

/// 讀版本快照成面板要的證據。
pub(crate) fn read_snapshot(paths: &DraftPaths, question_id: &str, revision: u32) -> std::io::Result<Option<EvidenceBundle>> {
    if !safe_id(question_id) {
        return Ok(None);
    }
    let dir = paths.revisions_root.join(question_id).join(revision.to_string());
    let Ok(meta) = std::fs::read(dir.join("meta.json")) else { return Ok(None) };
    let meta: SnapshotMeta = serde_json::from_slice(&meta).map_err(std::io::Error::other)?;
    let canvas = match meta.canvas {
        Some(kind) => Some(CanvasSource { kind, source: std::fs::read_to_string(dir.join(kind.file_name()))? }),
        None => None,
    };
    let mut assets = BTreeMap::new();
    let mut stack = vec![dir.join("assets")];
    let assets_root = dir.join("assets");
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let rel = path.strip_prefix(&assets_root).unwrap_or(&path).to_string_lossy().to_string();
            if let Some(mime) = image_mime(&rel) {
                let file = AssetFile { rel, bytes: std::fs::read(&path)?, mime };
                assets.insert(file.key(), file.data_uri());
            }
        }
    }
    Ok(Some(EvidenceBundle { canvas, assets }))
}
