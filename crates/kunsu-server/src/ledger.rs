//! Vault 真相層（CONTRACT §7）：`logs/asks/YYYY-MM-DD.jsonl` append-only +
//! `OPEN.json` 整檔重寫。vault 根路徑可注入（測試 tempdir；prod 由 [`vault_root_for_this_mac`] 決定，
//! PRD §31.3、BDD multi-user AC-MU-01）。
//!
//! 鐵則（inv_ledger_append_only）：jsonl 只走 `OpenOptions::append`，
//! 既有位元組永不被修改；OPEN.json 是唯一允許重寫的檔（temp+rename 原子換）。

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use kunsu_core::state_machine::TerminalKind;
use kunsu_core::time::{taipei_date, Ts};
use kunsu_core::types::{Answer, Question};
use serde::{Deserialize, Serialize};

use crate::error::ServerError;

/// 這台 Mac 的資料位置：環境變數 → `~/.kunsu/config.json` 的 `data_dir` → 舊資料夾（存在時）→ `~/Kunsu`
/// （`kunsu_core::paths::resolve_vault_root`）。
pub fn vault_root_for_this_mac() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"));
    let env = std::env::var("KUNSU_VAULT_ROOT").ok();
    let config = std::fs::read_to_string(home.join(".kunsu/config.json")).ok();
    // 只看存不存在（stat），不開檔：受保護的桌面資料夾不會因此卡住
    let legacy = home.join(kunsu_core::paths::LEGACY_VAULT).is_dir();
    kunsu_core::paths::resolve_vault_root(env.as_deref(), config.as_deref(), &home, legacy)
}

/// Ledger type 白名單六值（P1 定案 7；與 vault logs/asks/ASKS-SCHEMA.md 一致）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LedgerType {
    Answered,
    Expired,
    Cancelled,
    Superseded,
    ApprovalConsumed,
    Execution,
    /// 交付狀態改變（INV-13：重建後交付一致、不重複交付）。
    Delivery,
}

impl From<TerminalKind> for LedgerType {
    fn from(t: TerminalKind) -> Self {
        match t {
            TerminalKind::Answered => LedgerType::Answered,
            TerminalKind::Expired => LedgerType::Expired,
            TerminalKind::Cancelled => LedgerType::Cancelled,
            TerminalKind::Superseded => LedgerType::Superseded,
        }
    }
}

impl LedgerType {
    pub fn as_str(self) -> &'static str {
        match self {
            LedgerType::Answered => "answered",
            LedgerType::Expired => "expired",
            LedgerType::Cancelled => "cancelled",
            LedgerType::Superseded => "superseded",
            LedgerType::ApprovalConsumed => "approval_consumed",
            LedgerType::Execution => "execution",
            LedgerType::Delivery => "delivery",
        }
    }
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// 一行 ledger：`{ts, type, question:<全量快照>, answer?, execution?}`。
/// internal 題頂層標 internal（P1 定案 10，統計排除用）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LedgerRow {
    pub ts: Ts,
    #[serde(rename = "type")]
    pub ledger_type: LedgerType,
    #[serde(default, skip_serializing_if = "is_false")]
    pub internal: bool,
    /// 每行唯一的 uuid（舊行沒有）。逾時的寫入可能之後才寫成、又被補寫一次；重建時同一個 `row_id` 只認第一次。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub row_id: Option<String>,
    pub question: Question,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer: Option<Answer>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution: Option<serde_json::Value>,
}

/// OPEN.json 結構：pending（非終態）題全量快照。days_pending 不落盤，讀取端算。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenSnapshot {
    pub generated_at: Ts,
    pub pending: Vec<Question>,
}

#[derive(Clone)]
pub struct Ledger {
    vault_root: PathBuf,
    /// 同一時間最多一個在途的 vault 讀寫：上一個還卡著（多半是沒有完全磁碟權限）就不再開執行緒，直接回錯誤讓上層排隊。
    in_flight: std::sync::Arc<InFlight>,
}

/// 在途的 vault 讀寫：是否有一筆還沒回來、從什麼時候開始（epoch 毫秒；0＝沒有）。
#[derive(Default)]
struct InFlight {
    busy: std::sync::atomic::AtomicBool,
    since_ms: std::sync::atomic::AtomicI64,
}

/// vault IO 種類：逾時與「上一筆還卡著」的訊息要分清楚讀和寫（寫才會排進待補寫佇列）。
#[derive(Clone, Copy)]
enum IoKind {
    Read,
    Write,
}

/// 把可能阻塞的檔案 IO 丟到另一條執行緒並設逾時。macOS 受保護目錄（Desktop）未授權時
/// 檔案呼叫會卡在核心層，沒有逾時就會凍住持有 write_gate 的整個 server。
fn with_timeout<T, F>(secs: u64, kind: IoKind, in_flight: &std::sync::Arc<InFlight>, f: F) -> Result<T, ServerError>
where
    T: Send + 'static,
    F: FnOnce() -> std::io::Result<T> + Send + 'static,
{
    use std::sync::atomic::Ordering;
    if in_flight.busy.swap(true, Ordering::AcqRel) {
        let msg = match kind {
            IoKind::Write => "上一筆 vault 讀寫還卡著（多半是 macOS 未授予完全磁碟權限）：這一筆先排進待補寫佇列",
            IoKind::Read => "上一筆 vault 讀寫還卡著（多半是 macOS 未授予完全磁碟權限），現在讀不到 vault",
        };
        return Err(ServerError::Io(std::io::Error::new(std::io::ErrorKind::WouldBlock, msg)));
    }
    in_flight.since_ms.store(chrono::Utc::now().timestamp_millis(), Ordering::Release);
    let flag = in_flight.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result = f();
        flag.since_ms.store(0, Ordering::Release);
        flag.busy.store(false, Ordering::Release);
        let _ = tx.send(result);
    });
    match rx.recv_timeout(std::time::Duration::from_secs(secs)) {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err(e.into()),
        Err(_) => Err(ServerError::Io(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            match kind {
                IoKind::Write => {
                    "vault 檔案寫入逾時——多半是 macOS 未授予 Kunsu 完全磁碟/桌面存取權限；\
                     答案仍在 app 資料庫，權限恢復後 Kunsu 會自動依序補寫帳本（補寫完成前不要 rebuild-db）"
                }
                IoKind::Read => "vault 檔案讀取逾時——多半是 macOS 未授予 Kunsu 完全磁碟/桌面存取權限",
            },
        ))),
    }
}

impl Ledger {
    pub fn new(vault_root: impl Into<PathBuf>) -> Self {
        Ledger { vault_root: vault_root.into(), in_flight: Default::default() }
    }

    /// 目前在途（還沒回來）的 vault 讀寫從什麼時候開始；沒有就 None。卡太久代表核心層卡住，要請使用者重啟。
    pub fn stuck_since(&self) -> Option<Ts> {
        let ms = self.in_flight.since_ms.load(std::sync::atomic::Ordering::Acquire);
        (ms > 0).then(|| chrono::DateTime::from_timestamp_millis(ms)).flatten()
    }

    pub fn vault_root(&self) -> &Path {
        &self.vault_root
    }

    pub fn asks_dir(&self) -> PathBuf {
        self.vault_root.join("logs").join("asks")
    }

    /// 檔名以台北本地日期分日（vault 日檔慣例）。
    pub fn day_file(&self, now: Ts) -> PathBuf {
        self.asks_dir().join(format!("{}.jsonl", taipei_date(now)))
    }

    pub fn open_json_path(&self) -> PathBuf {
        self.asks_dir().join("OPEN.json")
    }

    /// append-only 寫入。絕不 truncate、絕不 seek。
    ///
    /// ⚠️ vault 在 Desktop（macOS 受保護目錄）：授權未給時檔案系統呼叫會**無限期阻塞**，
    /// 而本函式在 write_gate 內被呼叫 → 整個 server 凍住（2026-08-01 事故）。所以實際 IO
    /// 丟到另一條執行緒並設 5s 逾時：逾時就回錯誤讓上層記錄「帳本沒寫成」，
    /// **絕不拖垮 server**（答案仍在 SQLite；服務層把這一行排進待補寫佇列，權限恢復後依序補寫）。
    pub fn append(&self, row: &LedgerRow) -> Result<PathBuf, ServerError> {
        let dir = self.asks_dir();
        let path = self.day_file(row.ts);
        let mut line = serde_json::to_string(row)?;
        line.push('\n');
        let p = path.clone();
        with_timeout(5, IoKind::Write, &self.in_flight, move || {
            fs::create_dir_all(&dir)?;
            let mut f = OpenOptions::new().create(true).append(true).open(&p)?;
            f.write_all(line.as_bytes())?;
            f.flush()?;
            Ok(())
        })?;
        Ok(path)
    }

    /// OPEN.json 整檔重寫（temp + rename 原子換，唯一允許重寫的檔）。
    pub fn rewrite_open(&self, pending: &[Question], now: Ts) -> Result<(), ServerError> {
        let dir = self.asks_dir();
        let snapshot =
            OpenSnapshot { generated_at: now, pending: pending.to_vec() };
        let tmp = dir.join(".OPEN.json.tmp");
        let body = serde_json::to_string_pretty(&snapshot)?;
        let dst = self.open_json_path();
        with_timeout(5, IoKind::Write, &self.in_flight, move || {
            // 建目錄也在逾時保護內（受保護目錄未授權時 create_dir_all 一樣會卡在核心）。
            fs::create_dir_all(tmp.parent().unwrap_or(Path::new(".")))?;
            fs::write(&tmp, body)?;
            fs::rename(&tmp, &dst)?;
            Ok(())
        })
    }

    /// 讀 OPEN.json。檔案不存在才回 None；讀不到（權限、逾時）一律回錯誤——不能當成「沒有檔案」（INV-13）。
    pub fn read_open(&self) -> Result<Option<OpenSnapshot>, ServerError> {
        let path = self.open_json_path();
        let raw = with_timeout(10, IoKind::Read, &self.in_flight, move || match fs::read_to_string(&path) {
            Ok(raw) => Ok(Some(raw)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        })?;
        match raw {
            Some(raw) => Ok(Some(serde_json::from_str(&raw)?)),
            None => Ok(None),
        }
    }

    /// 讀全部 jsonl（檔名排序＝時間排序；同檔內行序即寫入序）。rebuild 用。
    /// 目錄不存在才回空；讀不到（權限、逾時）回錯誤。壞行跳過但回報；同一個 `row_id` 只保留第一次出現。
    pub fn read_all_rows(&self) -> Result<(Vec<LedgerRow>, Vec<String>), ServerError> {
        let dir = self.asks_dir();
        let files = with_timeout(30, IoKind::Read, &self.in_flight, move || {
            let entries = match fs::read_dir(&dir) {
                Ok(entries) => entries,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
                Err(e) => return Err(e),
            };
            let mut paths: Vec<PathBuf> = Vec::new();
            for entry in entries {
                let p = entry?.path();
                if p.extension().is_some_and(|ext| ext == "jsonl") {
                    paths.push(p);
                }
            }
            paths.sort();
            let mut out = Vec::new();
            for p in paths {
                let raw = fs::read_to_string(&p)?;
                out.push((p, raw));
            }
            Ok(out)
        })?;
        let mut rows = Vec::new();
        let mut bad = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for (file, raw) in files {
            for (i, line) in raw.lines().enumerate() {
                if line.trim().is_empty() {
                    continue;
                }
                match serde_json::from_str::<LedgerRow>(line) {
                    Ok(row) => {
                        if let Some(id) = row.row_id.as_deref()
                            && !seen.insert(id.to_string())
                        {
                            continue;
                        }
                        rows.push(row)
                    }
                    Err(e) => bad.push(format!("{}:{} — {e}", file.display(), i + 1)),
                }
            }
        }
        Ok((rows, bad))
    }
}
