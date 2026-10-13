//! 檔案 inbox ingester（CONTRACT §0 / P1 定案 6；harness-roundtrip 場景 5）。
//!
//! app 死掉時 agent 落檔 `~/.kunsu/inbox/<timestamp>-<id>.json`（與 ask_create
//! arguments 完全同格式、一檔一題）。app 啟動時 + 每 60s 掃描 ingest：
//! - 合法檔走與 ask_create 完全相同的驗證與 upsert 語義 → 移 `processed/`
//! - 壞檔（非法 JSON / schema 違規 / 配額超額）隔離移 `quarantine/` +
//!   Discord 告警，不阻塞其他檔。
//!
//! 路徑可注入（測試 tempdir）。

use std::path::PathBuf;
use std::sync::Arc;

use chrono::Utc;
use kunsu_core::types::CreateRequest;
use serde::Serialize;

use crate::discord::DiscordMessage;
use crate::mcp::normalize_create_args;
use crate::service::ServerHandle;

pub const INGEST_INTERVAL_S: u64 = 60;

#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct IngestReport {
    pub ingested: u32,
    pub quarantined: u32,
}

pub struct InboxIngester {
    handle: ServerHandle,
    dir: PathBuf,
}

impl InboxIngester {
    pub fn new(handle: ServerHandle, dir: PathBuf) -> Arc<Self> {
        Arc::new(InboxIngester { handle, dir })
    }

    /// 背景 task：啟動先掃一輪，之後每 60s 一輪。
    pub fn spawn(self: &Arc<Self>) -> tokio::task::JoinHandle<()> {
        let this = self.clone();
        tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(std::time::Duration::from_secs(INGEST_INTERVAL_S));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                interval.tick().await; // 第一次 tick 立即返回 → 啟動即掃
                let report = this.ingest_once().await;
                if report.ingested > 0 || report.quarantined > 0 {
                    tracing::info!(
                        "inbox ingest：{} 題入列、{} 檔隔離",
                        report.ingested,
                        report.quarantined
                    );
                }
            }
        })
    }

    /// 掃一輪 inbox。壞檔不阻塞其他檔（逐檔獨立處理）。
    pub async fn ingest_once(&self) -> IngestReport {
        let mut report = IngestReport::default();
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return report; // inbox 目錄不存在 = 沒東西可收，不是錯誤
        };
        let mut files: Vec<PathBuf> = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.is_file() && p.extension().is_some_and(|ext| ext == "json"))
            .collect();
        files.sort(); // 檔名 <timestamp>-<id>.json → 排序即時間序

        for file in files {
            match self.ingest_file(&file).await {
                Ok(()) => {
                    self.move_to(&file, "processed");
                    report.ingested += 1;
                }
                Err(reason) => {
                    self.move_to(&file, "quarantine");
                    report.quarantined += 1;
                    let name = file
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();
                    self.handle.discord().send(DiscordMessage {
                        question_id: None,
                        reason: "inbox_quarantine".to_string(),
                        text: format!("🗑️ inbox 壞檔已隔離 quarantine/{name}：{reason}"),
                    });
                }
            }
        }
        report
    }

    async fn ingest_file(&self, path: &PathBuf) -> Result<(), String> {
        let raw = std::fs::read_to_string(path).map_err(|e| format!("讀檔失敗：{e}"))?;
        let value: serde_json::Value =
            serde_json::from_str(&raw).map_err(|e| format!("非法 JSON：{e}"))?;
        let req: CreateRequest = serde_json::from_value(normalize_create_args(value))
            .map_err(|e| format!("schema 違規（malformed）：{e}"))?;
        self.handle
            .create_question(req, Utc::now())
            .await
            .map(|_| ())
            .map_err(|e| format!("建題被拒：{e}"))
    }

    fn move_to(&self, file: &PathBuf, sub: &str) {
        let dest_dir = self.dir.join(sub);
        if let Err(e) = std::fs::create_dir_all(&dest_dir) {
            tracing::error!("inbox {sub}/ 目錄建立失敗：{e}");
            return;
        }
        let Some(name) = file.file_name() else { return };
        if let Err(e) = std::fs::rename(file, dest_dir.join(name)) {
            tracing::error!("inbox 檔案移動失敗（{}）：{e}", file.display());
        }
    }
}
