//! kunsu-serverd — 獨立可執行（開發用）。
//!
//! 載 policy → 起 service + timer + HTTP（127.0.0.1:4777）+ reactor + inbox。
//! 之後 Tauri shell 用 lib 介面起同一套（本 bin 不進 launchd——CONTRACT §0：
//! launchd 只指 /Applications/Kunsu.app）。

use std::path::PathBuf;
use std::sync::Arc;

use kunsu_core::policy::Policy;
use kunsu_server::{
    default_key_provider, ensure_token, resolve_webhook, AppState, DiscordSender, InboxIngester,
    KunsuConfig, LoggingDiscord, Reactor, ReactorConfig, ServerHandle, StubGuardProbe,
    WebhookDiscord,
};

/// CONTRACT §0 常數。開發冒煙可用環境變數覆寫（KUNSU_BIND / KUNSU_HOME_DIR /
/// KUNSU_VAULT_ROOT / KUNSU_DB_PATH）——prod（launchd/Tauri）不帶即用預設。
const BIND_ADDR: &str = "127.0.0.1:4777";

fn env_or(key: &str, default: String) -> String {
    std::env::var(key).ok().filter(|v| !v.trim().is_empty()).unwrap_or(default)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt().with_env_filter(
        tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
    )
    .init();

    let home = std::env::var("HOME")?;
    let kunsu_dir = PathBuf::from(env_or(
        "KUNSU_HOME_DIR",
        PathBuf::from(&home).join(".kunsu").to_string_lossy().to_string(),
    ));
    std::fs::create_dir_all(&kunsu_dir)?;

    // policy：~/.kunsu/policy.toml（build-install 建 symlink → repo config/policy.toml）。
    // 缺檔或壞檔 → 預設值起服務並大聲 log（不因 policy 打錯字整台起不來）。
    let policy_path = kunsu_dir.join("policy.toml");
    let policy = match std::fs::read_to_string(&policy_path) {
        Ok(raw) => match Policy::from_toml_str(&raw) {
            Ok(p) => p,
            Err(e) => {
                tracing::error!("policy.toml 解析失敗（改用預設值）：{e}");
                Policy::default()
            }
        },
        Err(_) => {
            tracing::warn!("讀不到 {}（改用預設 policy）", policy_path.display());
            Policy::default()
        }
    };

    let token = ensure_token(&kunsu_dir.join("token"))?;
    // admin token（Codex 覆檢 P1）：/cli/answer 與 /admin/* 專用，agent 永不引用。
    let admin_token = ensure_token(&kunsu_dir.join("admin-token"))?;

    let discord: Arc<dyn DiscordSender> = match resolve_webhook(
        &policy.discord.webhook,
        &kunsu_core::paths::meta_ads_config(&kunsu_server::ledger::vault_root_for_this_mac()),
    ) {
        Some(url) => {
            tracing::info!("Discord webhook 已解析（policy 或 meta-ads config fallback）");
            WebhookDiscord::spawn(url)
        }
        None => {
            tracing::warn!("找不到任何 Discord webhook——訊息只落 log");
            Arc::new(LoggingDiscord)
        }
    };

    let db_dir = PathBuf::from(&home).join("Library/Application Support/me.ray.kunsu");
    let db_path = PathBuf::from(env_or(
        "KUNSU_DB_PATH",
        db_dir.join("kunsu.db").to_string_lossy().to_string(),
    ));
    std::fs::create_dir_all(db_path.parent().unwrap_or(&db_dir))?;
    let vault_root = kunsu_server::ledger::vault_root_for_this_mac();

    let handle = ServerHandle::new(KunsuConfig {
        db_path,
        vault_root: vault_root.clone(),
        policy,
        discord: discord.clone(),
        guards: Arc::new(StubGuardProbe::new()), // 真偵測在 Tauri shell 層接上
    })
    .await?;

    let key = default_key_provider(&kunsu_dir);

    let _timer = kunsu_server::spawn_timer(handle.clone());
    let reactor = Reactor::new(handle.clone(), ReactorConfig::new(vault_root), key.clone());
    let _reactor_task = reactor.spawn();
    let inbox = InboxIngester::new(handle.clone(), kunsu_dir.join("inbox"));
    let _inbox_task = inbox.spawn();

    let bind = env_or("KUNSU_BIND", BIND_ADDR.to_string());
    tracing::info!("kunsu-serverd 起動：HTTP {bind}、MCP /mcp、token {}/token", kunsu_dir.display());
    kunsu_server::serve(AppState { handle, token, admin_token, key }, &bind).await?;
    Ok(())
}
