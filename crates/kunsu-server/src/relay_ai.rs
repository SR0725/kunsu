//! 中轉 AI「軍師」（PRD §30.7；CONTRACT §17；INVARIANTS 32）。
//!
//! 手機上的追問、答完後的聊天由 Mac 上一次性的唯讀 Claude／Codex 工作階段回答；開新任務在指定專案資料夾以 Auto 權限開新對話。
//! 這裡只有純函數（組提示、組指令）與執行介面；`MobileBridge` 決定什麼時候跑、結果送去哪裡。

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::Stdio;
use std::time::Duration;

use kunsu_core::types::Question;
use tokio::io::AsyncWriteExt;

/// 追問與聊天的逾時。
pub const RELAY_TIMEOUT: Duration = Duration::from_secs(180);
/// 從手機開的新任務的逾時。
pub const TASK_TIMEOUT: Duration = Duration::from_secs(30 * 60);
/// 聊天記得同一支手機最近幾則（只在記憶體）。
pub const CHAT_MEMORY: usize = 12;

/// 軍師守則。這是說明，不是安全邊界——安全邊界是它沒有 MCP、只能讀（INV-32），以及 MCP 永遠不能作答（INV-7）。
pub const SYSTEM_PROMPT: &str = "你是「軍師」，使用者手機上 Kunsu 的中轉 AI。使用者正在用手機處理 AI 卡在他身上的事。\n\
規則：\n\
- 你只解釋與整理：為什麼要問、前因後果、各選項的後果、大家在做什麼。\n\
- 你不能替使用者作答、不能批准、不能動錢、不能建議繞過 Kunsu；正式答案只能由使用者在手機的按鈕上給。\n\
- 你只能讀檔，不能改檔或跑指令。\n\
- 用繁體中文、台灣用語、講人話；短，先講結論，手機一屏看得完（大約 200 字內），不用 Markdown 標題。\n\
- 不知道就說不知道，並建議使用者按「問本人」直接問發問的 AI。";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelayAgent {
    Claude,
    Codex,
}

impl RelayAgent {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "claude" => Some(Self::Claude),
            "codex" => Some(Self::Codex),
            _ => None,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Claude => "Claude",
            Self::Codex => "Codex",
        }
    }
}

/// 一次要跑的子行程。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedRun {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub stdin: String,
    pub cwd: PathBuf,
    /// Codex 把最後一則訊息寫在這個檔；None＝讀 stdout。
    pub output_file: Option<PathBuf>,
}

pub type RunFuture = Pin<Box<dyn Future<Output = Result<String, String>> + Send>>;

/// 跑一次 AI，回最後的文字或人話錯誤。測試換成假的。
pub trait AiRunner: Send + Sync {
    fn run(&self, plan: PlannedRun, timeout: Duration) -> RunFuture;
}

/// 追問與聊天：唯讀、沒有 MCP（INV-32）。提示從 stdin 給。
pub fn relay_command(agent: RelayAgent, cwd: &Path, prompt: &str, claude: &Path, codex: &Path, out: &Path) -> PlannedRun {
    match agent {
        RelayAgent::Claude => PlannedRun {
            program: claude.to_path_buf(),
            args: [
                "-p",
                "--output-format",
                "text",
                "--permission-mode",
                "dontAsk",
                "--allowedTools",
                "Read Grep Glob",
                "--strict-mcp-config",
                "--mcp-config",
                r#"{"mcpServers":{}}"#,
                "--append-system-prompt",
                SYSTEM_PROMPT,
            ]
            .iter()
            .map(|s| s.to_string())
            .collect(),
            stdin: prompt.to_string(),
            cwd: cwd.to_path_buf(),
            output_file: None,
        },
        RelayAgent::Codex => PlannedRun {
            program: codex.to_path_buf(),
            args: vec![
                "exec".into(),
                "--sandbox".into(),
                "read-only".into(),
                "--skip-git-repo-check".into(),
                "-C".into(),
                cwd.to_string_lossy().to_string(),
                "-o".into(),
                out.to_string_lossy().to_string(),
                "-".into(),
            ],
            // Codex 沒有附加系統提示的旗標：守則放在提示最前面。
            stdin: format!("{SYSTEM_PROMPT}\n\n{prompt}"),
            cwd: cwd.to_path_buf(),
            output_file: Some(out.to_path_buf()),
        },
    }
}

/// 從手機開新任務：在專案資料夾以 Auto 權限開新對話（PRD §30.3；不提供繞過權限）。
pub fn task_command(agent: RelayAgent, cwd: &Path, text: &str, claude: &Path, codex: &Path, out: &Path) -> PlannedRun {
    match agent {
        RelayAgent::Claude => PlannedRun {
            program: claude.to_path_buf(),
            args: vec!["-p".into(), "--output-format".into(), "text".into(), "--permission-mode".into(), "auto".into()],
            stdin: text.to_string(),
            cwd: cwd.to_path_buf(),
            output_file: None,
        },
        RelayAgent::Codex => PlannedRun {
            program: codex.to_path_buf(),
            args: vec![
                "exec".into(),
                "--approve-for-me".into(),
                "-C".into(),
                cwd.to_string_lossy().to_string(),
                "-o".into(),
                out.to_string_lossy().to_string(),
                "-".into(),
            ],
            stdin: text.to_string(),
            cwd: cwd.to_path_buf(),
            output_file: Some(out.to_path_buf()),
        },
    }
}

fn kind_label(kind: &str) -> &'static str {
    match kind {
        "why" => "為什麼要問我這題？",
        "context" => "這題的前因後果是什麼？",
        _ => "",
    }
}

/// 追問的提示：題目、原因、證據、選項與這題之前的追問（AC-RA-01）。
pub fn followup_prompt(q: &Question, kind: &str, text: Option<&str>, earlier: &[(String, String)]) -> String {
    let mut p = String::from("使用者正在手機上看這一題，還沒回答。\n\n");
    p.push_str(&format!("題目：{}\n", q.title));
    if !q.question_line.trim().is_empty() && q.question_line != q.title {
        p.push_str(&format!("問句：{}\n", q.question_line));
    }
    p.push_str(&format!("發問的 AI：{:?}；工作資料夾：{}\n", q.source.agent, q.source.cwd));
    if let Some(w) = q.why_md.as_deref().filter(|w| !w.trim().is_empty()) {
        p.push_str(&format!("為什麼要問（AI 寫的）：\n{w}\n"));
    }
    if !q.body_md.trim().is_empty() {
        p.push_str(&format!("說明：\n{}\n", q.body_md.chars().take(4000).collect::<String>()));
    }
    if !q.evidence.is_empty() {
        p.push_str("證據：\n");
        for e in &q.evidence {
            p.push_str(&format!("- {}（{}）\n", e.text, e.source));
        }
    }
    if !q.options.is_empty() {
        p.push_str("選項：\n");
        for o in &q.options {
            let rec = if o.recommended == Some(true) { "（AI 建議）" } else { "" };
            let detail = o.detail.as_deref().map(|d| format!("：{d}")).unwrap_or_default();
            p.push_str(&format!("- {}{rec}{detail}\n", o.label));
        }
    }
    if let Some(m) = q.money.as_ref().filter(|m| m.is_money_op) {
        p.push_str(&format!("這題會動錢（金額 {:?}，參數 {:?}）。\n", m.amount_twd, q.params_line));
    }
    if !earlier.is_empty() {
        p.push_str("\n使用者之前對這題的追問：\n");
        for (ask, ans) in earlier {
            p.push_str(&format!("問：{ask}\n答：{ans}\n"));
        }
    }
    let ask = match (kind_label(kind), text.map(str::trim).filter(|t| !t.is_empty())) {
        (_, Some(t)) => t.to_string(),
        (label, None) if !label.is_empty() => label.to_string(),
        _ => "請幫我看懂這題。".to_string(),
    };
    p.push_str(&format!("\n使用者現在問：{ask}\n需要的話可以讀工作資料夾裡的檔案。"));
    p
}

/// Kunsu 現況摘要（聊天用）：待決題與 48 小時內的答案。
pub fn state_summary(open: &[Question], recent: &[Question]) -> String {
    let mut s = String::new();
    if open.is_empty() {
        s.push_str("目前沒有題目在等使用者。\n");
    } else {
        s.push_str(&format!("在等使用者的題目（{} 題）：\n", open.len()));
        for q in open.iter().take(20) {
            s.push_str(&format!("- [{}] {}：{}\n", project_name(q), format!("{:?}", q.source.agent), q.title));
        }
    }
    if !recent.is_empty() {
        s.push_str("48 小時內使用者回答過的：\n");
        for q in recent.iter().take(20) {
            let a = q.answer.as_ref();
            let label = a
                .and_then(|a| a.choice_id.as_ref())
                .and_then(|c| q.options.iter().find(|o| &o.id == c).map(|o| o.label.clone()))
                .or_else(|| a.and_then(|a| a.free_text.clone()))
                .unwrap_or_else(|| "已回答".into());
            let result = a
                .and_then(|a| a.report.as_ref().map(|r| r.summary_md.chars().take(200).collect::<String>()))
                .or_else(|| a.and_then(|a| a.execution_result.clone()).filter(|r| !crate::reactor::is_money_result(r)))
                .unwrap_or_else(|| "還沒有回報".into());
            s.push_str(&format!("- [{}] {} → {}；結果：{}\n", project_name(q), q.title, label, result));
        }
    }
    s
}

/// 聊天的提示：現況摘要＋這支手機最近幾則＋這一句（AC-RA-06）。
pub fn chat_prompt(summary: &str, history: &[(String, String)], text: &str) -> String {
    let mut p = format!("Kunsu 現況：\n{summary}\n");
    if !history.is_empty() {
        p.push_str("最近的對話：\n");
        for (who, line) in history {
            p.push_str(&format!("{who}：{line}\n"));
        }
    }
    p.push_str(&format!("\n使用者：{text}\n請回覆使用者。要開新任務請叫他按輸入框左邊的「＋」。"));
    p
}

pub fn project_name(q: &Question) -> String {
    Path::new(&q.source.cwd)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .filter(|n| !n.is_empty())
        .or_else(|| q.product.clone())
        .unwrap_or_else(|| "Kunsu".into())
}

/// 真的跑 CLI：清掉繼承的對話身分（同 reactor）、逾時就殺掉。
pub struct CliRunner;

impl AiRunner for CliRunner {
    fn run(&self, plan: PlannedRun, timeout: Duration) -> RunFuture {
        Box::pin(async move {
            let mut cmd = tokio::process::Command::new(&plan.program);
            cmd.args(&plan.args)
                .current_dir(&plan.cwd)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true);
            for (k, _) in std::env::vars() {
                if crate::reactor::AGENT_SCRUB_ENV.iter().any(|p| k.starts_with(p)) {
                    cmd.env_remove(&k);
                }
            }
            let mut child = cmd.spawn().map_err(|e| format!("叫不起 {}：{e}", plan.program.display()))?;
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(plan.stdin.as_bytes()).await;
            }
            let out = match tokio::time::timeout(timeout, child.wait_with_output()).await {
                Ok(Ok(o)) => o,
                Ok(Err(e)) => return Err(format!("AI 執行失敗：{e}")),
                Err(_) => return Err(format!("軍師沒有回應（逾時 {} 秒）", timeout.as_secs())),
            };
            let text = match &plan.output_file {
                Some(f) => {
                    let t = std::fs::read_to_string(f).unwrap_or_default();
                    let _ = std::fs::remove_file(f);
                    t
                }
                None => String::from_utf8_lossy(&out.stdout).to_string(),
            };
            if !out.status.success() {
                let err = String::from_utf8_lossy(&out.stderr);
                let last = err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("沒有錯誤訊息");
                return Err(format!("AI 結束碼 {}：{}", out.status.code().unwrap_or(-1), last.chars().take(200).collect::<String>()));
            }
            let text = text.trim().to_string();
            if text.is_empty() { Err("AI 沒有輸出".into()) } else { Ok(text) }
        })
    }
}
