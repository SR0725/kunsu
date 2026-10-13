//! Discord 發送器：trait + 記錄型 stub（測試）+ 真 webhook 實作。
//! policy `[discord].webhook` 空字串時沿用 meta-ads config.json 的
//! `discord_webhooks.mkt_daily`（CONTRACT §10）。發送失敗只 log 不 panic。

use std::path::Path;
use std::sync::{Arc, Mutex};

/// 一則要發往 Discord 的訊息（人話文字由服務層組好）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscordMessage {
    pub question_id: Option<String>,
    /// 機器可讀的原因碼（snooze_cap_downgrade / money_expired / ...）。
    pub reason: String,
    pub text: String,
}

pub trait DiscordSender: Send + Sync {
    fn send(&self, msg: DiscordMessage);
}

/// 記錄型 stub：測試與本階段預設。訊息全部存記憶體供斷言。
#[derive(Default)]
pub struct RecordingDiscord {
    sent: Mutex<Vec<DiscordMessage>>,
}

impl RecordingDiscord {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn messages(&self) -> Vec<DiscordMessage> {
        self.sent.lock().expect("discord stub lock").clone()
    }
}

impl DiscordSender for RecordingDiscord {
    fn send(&self, msg: DiscordMessage) {
        self.sent.lock().expect("discord stub lock").push(msg);
    }
}

/// 只寫 log 的後備發送器（找不到任何 webhook 時 serverd 用，確保不靜默丟棄語義以 log 呈現）。
pub struct LoggingDiscord;

impl DiscordSender for LoggingDiscord {
    fn send(&self, msg: DiscordMessage) {
        tracing::warn!(
            "Discord 未設定 webhook，訊息只落 log：[{}] {}",
            msg.reason,
            msg.text
        );
    }
}


/// webhook 解析：policy 值非空 → 用它；否則讀 meta-ads config.json 的
/// `discord_webhooks.mkt_daily`。兩邊都沒有 → None（呼叫端落 LoggingDiscord）。
/// 逾時讀檔：受保護目錄（Desktop 等）未授權時 `open()` 會無限期阻塞，啟動路徑上一律用它。
pub fn read_to_string_timeout(path: &Path, secs: u64) -> Option<String> {
    let (tx, rx) = std::sync::mpsc::channel();
    let p = path.to_path_buf();
    std::thread::spawn(move || {
        let _ = tx.send(std::fs::read_to_string(&p));
    });
    match rx.recv_timeout(std::time::Duration::from_secs(secs)) {
        Ok(r) => r.ok(),
        Err(_) => {
            tracing::error!(
                "讀取 {} 逾時 {secs}s——多半是 macOS 檔案存取授權未給（Kunsu 需要「完全取得磁碟權限」或桌面資料夾存取），先跳過繼續啟動",
                path.display()
            );
            None
        }
    }
}

pub fn resolve_webhook(policy_webhook: &str, meta_config_path: &Path) -> Option<String> {
    let trimmed = policy_webhook.trim();
    if !trimmed.is_empty() {
        return Some(trimmed.to_string());
    }
    // ⚠️ 這個路徑在 Desktop（受 macOS 隱私保護）：授權未給時 open() 會在核心層無限期阻塞，
    // 而本函式位於啟動路徑上 → 整支 app 靜默假死（2026-08-01 事故）。一律走逾時讀。
    let raw = read_to_string_timeout(meta_config_path, 2)?;
    let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
    v.get("discord_webhooks")?
        .get("mkt_daily")?
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.trim().to_string())
}

/// 真 webhook 發送器：`send` 進 channel，背景 task 逐則 POST
/// `{content, allowed_mentions: {parse: []}}`。失敗只 log、不重試、不 panic。
pub struct WebhookDiscord {
    tx: tokio::sync::mpsc::UnboundedSender<DiscordMessage>,
}

impl WebhookDiscord {
    /// 必須在 tokio runtime 內呼叫（背景 task 由此 spawn）。
    pub fn spawn(webhook_url: String) -> Arc<Self> {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<DiscordMessage>();
        tokio::spawn(async move {
            let client = reqwest::Client::new();
            while let Some(msg) = rx.recv().await {
                let body = serde_json::json!({
                    "content": msg.text,
                    "allowed_mentions": {"parse": []},
                });
                match client.post(&webhook_url).json(&body).send().await {
                    Ok(resp) if resp.status().is_success() => {}
                    Ok(resp) => tracing::warn!(
                        "Discord webhook 回 {}（reason={}，只 log 不重試）",
                        resp.status(),
                        msg.reason
                    ),
                    Err(e) => tracing::warn!(
                        "Discord webhook 發送失敗（reason={}，只 log 不重試）：{e}",
                        msg.reason
                    ),
                }
            }
        });
        Arc::new(WebhookDiscord { tx })
    }
}

impl DiscordSender for WebhookDiscord {
    fn send(&self, msg: DiscordMessage) {
        // channel 關閉（runtime 收尾）時靜默丟棄即可——app 正在退出。
        let _ = self.tx.send(msg);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// AC-MU-02：沒有 Meta 廣告設定（新使用者）時告警安靜地關閉。
    #[test]
    fn missing_meta_config_disables_discord_quietly() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(resolve_webhook("", &dir.path().join("projects/meta-ads/config.json")), None);
    }

    #[test]
    fn resolve_webhook_prefers_policy_then_config() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path().join("config.json");
        std::fs::write(
            &cfg,
            r#"{"discord_webhooks": {"mkt_daily": "https://discord.com/api/webhooks/x/y"}}"#,
        )
        .unwrap();
        assert_eq!(
            resolve_webhook("  https://p.example/hook  ", &cfg).as_deref(),
            Some("https://p.example/hook")
        );
        assert_eq!(
            resolve_webhook("", &cfg).as_deref(),
            Some("https://discord.com/api/webhooks/x/y")
        );
        assert_eq!(resolve_webhook("", &dir.path().join("missing.json")), None);
    }
}

