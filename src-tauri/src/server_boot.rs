//! 內嵌 kunsu-server 全套啟動（CONTRACT §1：Tauri 內嵌起同一套 lib）。
//!
//! service + HTTP(4777) + timer + reactor + inbox + policy 熱載 watcher（policy.toml＋設定頁的 policy.local.toml，
//! CONTRACT §15）。
//! 埠被占 = 已有實例在跑 → 回 `AlreadyRunning`，前景實例直接退出（避免雙開）。
//! 環境變數覆寫與 kunsu-serverd 同款（KUNSU_BIND / KUNSU_HOME_DIR /
//! KUNSU_VAULT_ROOT / KUNSU_DB_PATH）——prod 不帶即用 CONTRACT §0 常數。

use std::path::PathBuf;
use std::sync::Arc;

use kunsu_core::policy::Policy;
use kunsu_server::settings::{effective_policy, SettingsPaths};
use kunsu_server::{
    default_key_provider, ensure_token, resolve_webhook, AppState, DiscordSender, InboxIngester,
    KeyProvider, KunsuConfig, LoggingDiscord, Reactor, ReactorConfig, ServerHandle,
    WebhookDiscord,
};

use crate::guards::{MacGuardProbe, SharedGuardFlags};

const BIND_ADDR: &str = "127.0.0.1:4777";

pub enum BootError {
    /// 埠被占 = 已有 Kunsu 實例在跑；前景實例應直接退出，不是錯誤。
    AlreadyRunning(String),
    Fatal(String),
}

pub struct Boot {
    pub handle: ServerHandle,
    pub key: Arc<dyn KeyProvider>,
    pub vault_root: PathBuf,
    /// `~/.kunsu`（`KUNSU_HOME_DIR` 覆寫）：設定檔、CLI、token 都在這裡。
    pub kunsu_dir: PathBuf,
    /// 本機服務位址（`KUNSU_BIND` 覆寫）。
    pub bind: String,
    /// 手機橋接（PRD §30.8）；金鑰檔讀寫失敗時為 None，其他功能照常。
    pub mobile: Option<kunsu_server::mobile::MobileBridge>,
}

fn env_or(key: &str, default: String) -> String {
    std::env::var(key).ok().filter(|v| !v.trim().is_empty()).unwrap_or(default)
}

pub fn kunsu_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(env_or(
        "KUNSU_HOME_DIR",
        PathBuf::from(home).join(".kunsu").to_string_lossy().to_string(),
    ))
}

/// 讀檔但**永不無限阻塞**：另起執行緒讀、2 秒拿不到就放棄用預設值。
///
/// 2026-08-01 事故：`~/.kunsu/policy.toml` 曾是指向 `~/Desktop/kunsu/config/` 的 symlink，
/// app 由 LaunchServices 啟動且重簽名後 macOS 桌面隱私授權失效 → `open()` 在核心層無限期
/// 卡住（無錯誤、無 log、port 永不 bind、doctor 全紅）。**啟動路徑上的任何檔案讀取都必須有
/// 逾時**，否則一個權限提示就能讓整個 app 靜默假死。
fn read_to_string_timeout(path: &std::path::Path, secs: u64) -> Option<String> {
    let (tx, rx) = std::sync::mpsc::channel();
    let p = path.to_path_buf();
    std::thread::spawn(move || {
        let _ = tx.send(std::fs::read_to_string(&p));
    });
    match rx.recv_timeout(std::time::Duration::from_secs(secs)) {
        Ok(Ok(s)) => Some(s),
        Ok(Err(_)) => None,
        Err(_) => {
            tracing::error!(
                "讀取 {} 逾時 {secs}s（多半是 macOS 檔案存取授權未給或指向受保護目錄）——改用預設值繼續啟動",
                path.display()
            );
            None
        }
    }
}

/// 啟動時的生效 policy（CONTRACT §15）：policy.toml 逐鍵疊上設定頁的 policy.local.toml。兩檔都經逾時讀取；
/// base 讀不到或壞掉用預設值，local 不合法只用 base（不崩潰、不套用部分值）。
fn load_policy(kunsu_dir: &std::path::Path) -> Policy {
    let paths = SettingsPaths::in_dir(kunsu_dir);
    let base = read_to_string_timeout(&paths.base, 2);
    match &base {
        None => tracing::warn!("讀不到 {}（改用預設 policy）", paths.base.display()),
        Some(raw) => {
            if let Err(e) = Policy::from_toml_str(raw) {
                tracing::error!("policy.toml 解析失敗（改用預設值）：{e}");
            }
        }
    }
    let local = read_to_string_timeout(&paths.local, 2);
    let loaded = effective_policy(base.as_deref(), local.as_deref());
    if let Some(err) = &loaded.local_error {
        tracing::error!("{} 不合法，只用 policy.toml：{err}", paths.local.display());
    }
    if !loaded.ignored.is_empty() {
        tracing::warn!("{} 有不開放的鍵（已忽略）：{}", paths.local.display(), loaded.ignored.join("、"));
    }
    if !loaded.overridden.is_empty() {
        tracing::info!("個人設定覆寫：{}", loaded.overridden.join("、"));
    }
    loaded.policy
}

/// 熱載看的兩個 mtime：(policy.toml, policy.local.toml)。
pub type PolicyMtimes = (Option<std::time::SystemTime>, Option<std::time::SystemTime>);

/// 熱載判斷：第一次只記下；之後任一檔出現、消失或改變就要重讀。
pub fn policy_files_changed(last: &mut Option<PolicyMtimes>, now: PolicyMtimes) -> bool {
    let changed = last.is_some_and(|prev| prev != now);
    *last = Some(now);
    changed
}

/// 啟動全套。必須在 tokio runtime context 內呼叫
/// （`tauri::async_runtime::block_on`）。
pub async fn boot(guard_flags: Arc<SharedGuardFlags>) -> Result<Boot, BootError> {
    let bind = env_or("KUNSU_BIND", BIND_ADDR.to_string());

    // 埠占用預檢：綁得起來→立刻放掉給 axum；綁不起來→已有實例。
    match std::net::TcpListener::bind(&bind) {
        Ok(l) => drop(l),
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
            return Err(BootError::AlreadyRunning(bind));
        }
        Err(e) => return Err(BootError::Fatal(format!("綁定 {bind} 失敗：{e}"))),
    }

    let kunsu_dir = kunsu_dir();
    std::fs::create_dir_all(&kunsu_dir)
        .map_err(|e| BootError::Fatal(format!("建立 {} 失敗：{e}", kunsu_dir.display())))?;

    let policy = load_policy(&kunsu_dir);
    let token = ensure_token(&kunsu_dir.join("token"))
        .map_err(|e| BootError::Fatal(format!("token 初始化失敗：{e}")))?;
    // admin token（Codex 覆檢 P1）：/cli/answer 與 /admin/* 專用，agent 永不引用。
    let admin_token = ensure_token(&kunsu_dir.join("admin-token"))
        .map_err(|e| BootError::Fatal(format!("admin token 初始化失敗：{e}")))?;

    let discord: Arc<dyn DiscordSender> = match resolve_webhook(
        &policy.discord.webhook,
        &kunsu_core::paths::meta_ads_config(&kunsu_server::ledger::vault_root_for_this_mac()),
    ) {
        Some(url) => WebhookDiscord::spawn(url),
        None => {
            tracing::warn!("找不到任何 Discord webhook——訊息只落 log");
            Arc::new(LoggingDiscord)
        }
    };

    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    let db_dir = PathBuf::from(&home).join("Library/Application Support/me.ray.kunsu");
    let db_path = PathBuf::from(env_or(
        "KUNSU_DB_PATH",
        db_dir.join("kunsu.db").to_string_lossy().to_string(),
    ));
    std::fs::create_dir_all(db_path.parent().unwrap_or(&db_dir))
        .map_err(|e| BootError::Fatal(format!("建立 DB 目錄失敗：{e}")))?;
    let vault_root = kunsu_server::ledger::vault_root_for_this_mac();

    let handle = ServerHandle::new(KunsuConfig {
        db_path,
        vault_root: vault_root.clone(),
        policy,
        discord,
        guards: Arc::new(MacGuardProbe::new(guard_flags)),
    })
    .await
    .map_err(|e| BootError::Fatal(format!("service 啟動失敗：{e}")))?;
    // 設定頁讀寫 policy.local.toml 的位置（CONTRACT §15）。
    handle.set_settings_paths(SettingsPaths::in_dir(&kunsu_dir));

    let key = default_key_provider(&kunsu_dir);

    // timer + reactor（真跑，非 dry-run）+ inbox ingester。
    let _timer = kunsu_server::spawn_timer(handle.clone());
    let reactor = Reactor::new(handle.clone(), ReactorConfig::new(vault_root.clone()), key.clone());
    let _reactor_task = reactor.spawn();
    let inbox = InboxIngester::new(handle.clone(), kunsu_dir.join("inbox"));
    let _inbox_task = inbox.spawn();

    // HTTP（axum）背景常駐。
    {
        let state = AppState { handle: handle.clone(), token, admin_token, key: key.clone() };
        let bind = bind.clone();
        tokio::spawn(async move {
            if let Err(e) = kunsu_server::serve(state, &bind).await {
                tracing::error!("HTTP 服務終止：{e}");
            }
        });
    }

    // policy 熱載（BDD forcing-and-guards 場景 12、CONTRACT §15）：每 5 秒看 policy.toml 與 policy.local.toml 的 mtime，
    // 任一改變就重讀兩檔疊層後 set_policy。base 壞掉沿用舊值；local 壞掉只用 base。
    {
        let handle = handle.clone();
        let paths = SettingsPaths::in_dir(&kunsu_dir);
        tokio::spawn(async move {
            let mtime = |p: &std::path::Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
            let mut last: Option<PolicyMtimes> = None;
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                interval.tick().await;
                if policy_files_changed(&mut last, (mtime(&paths.base), mtime(&paths.local))) {
                    match handle.reload_policy_files() {
                        Ok(()) => tracing::info!("policy 熱載完成（policy.toml＋policy.local.toml）"),
                        Err(e) => tracing::error!("policy 熱載失敗：{e}"),
                    }
                }
            }
        });
    }

    // vault 存取健檢（2026-08-01 事故後加）：Desktop 受 macOS 隱私保護，授權沒給時
    // ledger 寫入會失敗或阻塞。啟動後在背景探一次（逾時 3s），失敗就大聲：log + Discord，
    // 讓「權限沒給」變成看得見的訊息，而不是答案默默寫不進帳本。
    {
        let vault = vault_root.clone();
        let discord = handle.discord();
        tokio::task::spawn_blocking(move || {
            let probe = vault.join("logs/asks/.kunsu-access-probe");
            let (tx, rx) = std::sync::mpsc::channel();
            let p2 = probe.clone();
            std::thread::spawn(move || {
                let r = std::fs::write(&p2, b"ok").and_then(|_| std::fs::remove_file(&p2));
                let _ = tx.send(r.is_ok());
            });
            let ok = matches!(rx.recv_timeout(std::time::Duration::from_secs(3)), Ok(true));
            if !ok {
                tracing::error!(
                    "vault 存取健檢失敗（{}）——多半是 macOS 沒給 Kunsu 桌面/完全磁碟存取權限：\
                     系統設定 → 隱私權與安全性 → 完全取得磁碟權限 → 加入 Kunsu.app。\
                     在授權前，答案仍會存在 app 內部資料庫，但寫不進 vault 帳本。",
                    probe.display()
                );
                discord.send(kunsu_server::DiscordMessage {
                    question_id: None,
                    reason: "vault_access_denied".to_string(),
                    text: "⚠️ Kunsu 寫不進 vault 帳本（macOS 檔案存取權限未給）——請到「系統設定 → 隱私權與安全性 → 完全取得磁碟權限」加入 Kunsu.app。在那之前答案只存在 app 內部資料庫。".to_string(),
                });
            }
        });
    }

    tracing::info!("Kunsu 內嵌 server 起動：HTTP {bind}、MCP /mcp");
    // 手機橋接：連 Zeabur 中轉站（`KUNSU_RELAY_URL` 可覆寫預設網址）。
    let relay = env_or("KUNSU_RELAY_URL", kunsu_server::mobile::DEFAULT_RELAY_URL.to_string());
    let mobile = match kunsu_server::mobile::MobileBridge::load(handle.clone(), key.clone(), &kunsu_dir, &relay) {
        Ok(b) => {
            b.spawn();
            Some(b)
        }
        Err(e) => {
            tracing::error!("手機橋接啟動失敗（其他功能不受影響）：{e}");
            None
        }
    };

    Ok(Boot { handle, key, vault_root, kunsu_dir, bind, mobile })
}

#[cfg(test)]
mod tests {
    use super::{policy_files_changed, PolicyMtimes};
    use std::time::{Duration, SystemTime};

    /// CONTRACT §15：熱載同時看 policy.toml 與 policy.local.toml 的 mtime——第一次只記下、之後任一檔出現、消失或改變就重讀。
    #[test]
    fn hot_reload_watches_both_policy_files() {
        let t = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);
        let later = t + Duration::from_secs(5);
        let mut last: Option<PolicyMtimes> = None;
        assert!(!policy_files_changed(&mut last, (Some(t), None)), "第一次只記下");
        assert!(!policy_files_changed(&mut last, (Some(t), None)), "沒變不重讀");
        assert!(policy_files_changed(&mut last, (Some(t), Some(t))), "設定頁第一次寫 local");
        assert!(policy_files_changed(&mut last, (Some(t), Some(later))), "local 改變");
        assert!(policy_files_changed(&mut last, (Some(later), Some(later))), "重裝複製 base");
        assert!(policy_files_changed(&mut last, (Some(later), None)), "local 被刪掉");
        assert!(!policy_files_changed(&mut last, (Some(later), None)));
    }
}
