//! 主視窗設定與狀態（PRD §27、CONTRACT §15、BDD settings、INV-26）。
//!
//! - 設定頁讀寫 `~/.kunsu/policy.local.toml`（個人值），逐鍵疊在安裝時複製的 `~/.kunsu/policy.toml` 上
//!   （[`Policy::layered`]，純函數在 core）。寫入是 tmp＋rename、只寫可設定鍵、檔頭標明由設定頁管理；先算疊層，不合法
//!   整筆拒絕、檔案不變；成功立即 `set_policy`。
//! - INV-26：這裡永不碰題目、帳本或事件——只有 local 檔與生效 policy 會變。
//! - `app_status`：版本、運作時間、位址、路徑、帳本健康，以及四項 Agent 連線的真實狀態（只看檔案存在與鍵，
//!   不讀、不回傳任何 token 或設定內容）。

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use kunsu_core::policy::{is_settable, LayerError, Policy, SettingError, SettingsView, SETTABLE_KEYS};
use kunsu_core::time::Ts;
use serde::Serialize;
use serde_json::{Map, Value};

use crate::error::ServerError;
use crate::service::ServerHandle;

/// 檔案層錯誤（local 檔語法壞掉、讀寫失敗、路徑沒設定）在 `errors` 裡用的 key。
pub const LOCAL_FILE_KEY: &str = "policy.local.toml";

/// local 檔的檔頭（每次寫入都保留）。
pub const LOCAL_HEADER: &str = "# 由 Kunsu 設定頁管理（主視窗「設定」）。這裡是使用者的個人值，逐鍵疊在 ~/.kunsu/policy.toml 上；\n\
# 重新安裝不會碰這個檔。只有設定頁開放的鍵有效（CONTRACT §15）；手動改壞時 Kunsu 只用預設值，設定頁會顯示錯誤。\n";

/// MCP 端點（與 `kunsu doctor` 印的指令相同）。
const MCP_URL: &str = "http://127.0.0.1:4777/mcp";

/// 設定檔路徑（shell 依 `KUNSU_HOME_DIR` 注入；測試指 tempdir）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsPaths {
    pub base: PathBuf,
    pub local: PathBuf,
}

impl SettingsPaths {
    /// `<kunsu_dir>/policy.toml` 與 `<kunsu_dir>/policy.local.toml`。
    pub fn in_dir(kunsu_dir: &Path) -> SettingsPaths {
        SettingsPaths { base: kunsu_dir.join("policy.toml"), local: kunsu_dir.join("policy.local.toml") }
    }
}

/// `settings_get`／`settings_set`／`settings_reset` 的回應（CONTRACT §15）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SettingsSnapshot {
    pub effective: SettingsView,
    pub defaults: SettingsView,
    pub overridden: Vec<String>,
    pub local_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_error: Option<String>,
}

/// 設定被拒（IPC 以 Promise reject 回傳，形狀 `{errors: [{key, message}]}`）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SettingsRejection {
    pub errors: Vec<SettingError>,
}

impl SettingsRejection {
    fn file(message: impl Into<String>) -> SettingsRejection {
        SettingsRejection { errors: vec![SettingError::new(LOCAL_FILE_KEY, message)] }
    }

    fn keys(mut errors: Vec<SettingError>) -> SettingsRejection {
        errors.sort_by_key(|e| SETTABLE_KEYS.iter().position(|k| *k == e.key).unwrap_or(SETTABLE_KEYS.len()));
        SettingsRejection { errors }
    }

    fn not_configured() -> SettingsRejection {
        SettingsRejection::file("設定檔位置還沒設定（app 啟動時注入），無法讀寫個人設定")
    }
}

impl std::fmt::Display for SettingsRejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let parts: Vec<String> = self.errors.iter().map(|e| format!("{}：{}", e.key, e.message)).collect();
        f.write_str(&parts.join("；"))
    }
}

impl From<LayerError> for SettingsRejection {
    fn from(e: LayerError) -> SettingsRejection {
        match e {
            LayerError::Invalid(errors) => SettingsRejection::keys(errors),
            LayerError::Syntax(_) => SettingsRejection::file(format!("{e}；先按「全部回復預設」")),
        }
    }
}

/// 兩檔原文 → 生效 policy（啟動、熱載、CLI 共用）。local 不合法時只用 base，錯誤放在 `local_error`。
#[derive(Debug, Clone, PartialEq)]
pub struct LoadedPolicy {
    pub policy: Policy,
    /// 只有 base（設定頁的「預設值」）。
    pub base: Policy,
    pub overridden: Vec<String>,
    pub ignored: Vec<String>,
    pub local_error: Option<String>,
}

/// `base` 為 None（讀不到）或不合法時用 `Policy::default()`；`local` 為 None 表示沒有個人設定。
pub fn effective_policy(base: Option<&str>, local: Option<&str>) -> LoadedPolicy {
    let base_raw = base.unwrap_or("");
    let base_policy = Policy::from_toml_str(base_raw).unwrap_or_default();
    match Policy::layered(base_raw, local) {
        Ok(l) => LoadedPolicy {
            policy: l.policy,
            base: base_policy,
            overridden: l.overridden,
            ignored: l.ignored,
            local_error: None,
        },
        Err(e) => LoadedPolicy {
            policy: base_policy.clone(),
            base: base_policy,
            overridden: Vec::new(),
            ignored: Vec::new(),
            local_error: Some(e.to_string()),
        },
    }
}

/// `kunsu doctor` 的一行（不算失敗）：有沒有個人覆寫；local 檔壞掉時說清楚 app 只用預設值。
pub fn doctor_line(base: Option<&str>, local: Option<&str>, local_path: &Path) -> String {
    let loaded = effective_policy(base, local);
    let path = local_path.display();
    if let Some(err) = &loaded.local_error {
        return format!("個人設定檔壞掉，app 只用預設值：{err}（{path}；到設定頁按「全部回復預設」或修正檔案）");
    }
    let mut line = if loaded.overridden.is_empty() {
        format!("沒有個人覆寫，全部用預設值（{path}）")
    } else {
        format!("個人覆寫 {} 項：{}（{path}）", loaded.overridden.len(), loaded.overridden.join("、"))
    };
    if !loaded.ignored.is_empty() {
        line.push_str(&format!("；忽略不開放的鍵：{}", loaded.ignored.join("、")));
    }
    line
}

/// 讀檔；不存在回 None。
fn read_optional(path: &Path) -> std::io::Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(s) => Ok(Some(s)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// local 檔目前的可設定值（不開放的鍵不保留：設定頁只寫可設定鍵）。
enum LocalValues {
    Values(BTreeMap<&'static str, toml::Value>),
    /// 語法壞掉（訊息）。
    Broken(String),
}

fn parse_local(raw: Option<&str>) -> LocalValues {
    let Some(raw) = raw else { return LocalValues::Values(BTreeMap::new()) };
    let table: toml::Table = match raw.parse() {
        Ok(t) => t,
        // 一行人話（行號＋原因，不貼原檔），與 `Policy::layered` 的 Syntax 訊息相同。
        Err(_) => {
            let msg = Policy::layered("", Some(raw)).err().map(|e| e.to_string()).unwrap_or_default();
            return LocalValues::Broken(msg);
        }
    };
    let mut values = BTreeMap::new();
    for key in SETTABLE_KEYS {
        let (section, name) = key.split_once('.').expect("鍵是 section.key");
        if let Some(v) = table.get(section).and_then(|s| s.get(name)) {
            values.insert(key, v.clone());
        }
    }
    LocalValues::Values(values)
}

/// 依 CONTRACT §15 表的順序寫出 local 檔（檔頭＋只有可設定鍵）。
fn render_local(values: &BTreeMap<&'static str, toml::Value>) -> String {
    let mut table = toml::Table::new();
    for key in SETTABLE_KEYS {
        let Some(v) = values.get(key) else { continue };
        let (section, name) = key.split_once('.').expect("鍵是 section.key");
        if let toml::Value::Table(t) =
            table.entry(section.to_string()).or_insert(toml::Value::Table(toml::Table::new()))
        {
            t.insert(name.to_string(), v.clone());
        }
    }
    let body = toml::to_string(&table).unwrap_or_default();
    if body.trim().is_empty() { LOCAL_HEADER.to_string() } else { format!("{LOCAL_HEADER}\n{body}") }
}

/// tmp＋fsync＋rename 原子替換（同目錄，讀到的永遠是完整的舊檔或新檔）。
fn write_atomic(path: &Path, contents: &str) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(dir)?;
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| LOCAL_FILE_KEY.to_string());
    let tmp = dir.join(format!(".{name}.tmp-{}-{nanos}", std::process::id()));
    let result = (|| {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(contents.as_bytes())?;
        f.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// 設定頁送來的 JSON 值 → TOML 值（型別與範圍交給 `Policy::layered` 檢查）。
fn json_to_toml(key: &str, v: &Value) -> Result<toml::Value, SettingError> {
    let scalar = |v: &Value| -> Option<toml::Value> {
        match v {
            Value::Bool(b) => Some(toml::Value::Boolean(*b)),
            Value::Number(n) => n.as_i64().map(toml::Value::Integer).or_else(|| n.as_f64().map(toml::Value::Float)),
            Value::String(s) => Some(toml::Value::String(s.clone())),
            _ => None,
        }
    };
    match v {
        Value::Null => Err(SettingError::new(key, "值不能是空的；要回到預設請按「回復預設」")),
        Value::Array(items) => items
            .iter()
            .map(scalar)
            .collect::<Option<Vec<_>>>()
            .map(toml::Value::Array)
            .ok_or_else(|| SettingError::new(key, "格式不對")),
        other => scalar(other).ok_or_else(|| SettingError::new(key, "格式不對")),
    }
}

fn settable_static(key: &str) -> Option<&'static str> {
    SETTABLE_KEYS.iter().copied().find(|k| *k == key)
}

fn snapshot(paths: &SettingsPaths, base: Option<&str>, local: Option<&str>) -> SettingsSnapshot {
    let loaded = effective_policy(base, local);
    SettingsSnapshot {
        effective: SettingsView::from(&loaded.policy),
        defaults: SettingsView::from(&loaded.base),
        overridden: loaded.overridden,
        local_path: paths.local.to_string_lossy().into_owned(),
        local_error: loaded.local_error,
    }
}

fn io_rejection(action: &str, path: &Path, e: &std::io::Error) -> SettingsRejection {
    SettingsRejection::file(format!("{action} {} 失敗：{e}", path.display()))
}

impl ServerHandle {
    /// 讀兩檔，回生效值、預設值、被覆寫的鍵與 local 檔錯誤（不改任何東西）。
    pub fn settings_get(&self) -> Result<SettingsSnapshot, SettingsRejection> {
        let paths = self.settings_paths().ok_or_else(SettingsRejection::not_configured)?;
        let base = read_optional(&paths.base).map_err(|e| io_rejection("讀取", &paths.base, &e))?;
        let local = read_optional(&paths.local).map_err(|e| io_rejection("讀取", &paths.local, &e))?;
        Ok(snapshot(&paths, base.as_deref(), local.as_deref()))
    }

    /// 存設定：只接受 CONTRACT §15 表上的鍵；與 local 檔現有值合併後算疊層，不合法整筆拒絕、檔案不變；成功原子寫入並立即生效。
    pub fn settings_set(&self, changes: &Map<String, Value>) -> Result<SettingsSnapshot, SettingsRejection> {
        let paths = self.settings_paths().ok_or_else(SettingsRejection::not_configured)?;
        let _gate = self.settings_gate();
        let mut errors = Vec::new();
        let mut converted: Vec<(&'static str, toml::Value)> = Vec::new();
        for (key, value) in changes {
            match settable_static(key) {
                None => errors.push(SettingError::new(key, "這個設定不能從設定頁修改")),
                Some(k) => match json_to_toml(k, value) {
                    Ok(v) => converted.push((k, v)),
                    Err(e) => errors.push(e),
                },
            }
        }
        let base = read_optional(&paths.base).map_err(|e| io_rejection("讀取", &paths.base, &e))?;
        let local = read_optional(&paths.local).map_err(|e| io_rejection("讀取", &paths.local, &e))?;
        if errors.is_empty() && converted.is_empty() {
            return Ok(snapshot(&paths, base.as_deref(), local.as_deref()));
        }
        let mut values = match parse_local(local.as_deref()) {
            LocalValues::Values(v) => v,
            LocalValues::Broken(msg) => {
                return Err(SettingsRejection::file(format!(
                    "{msg}（{}）；先按「全部回復預設」",
                    paths.local.display()
                )));
            }
        };
        values.extend(converted);
        if !errors.is_empty() {
            // 整筆拒絕，但其他鍵的範圍錯誤也一起列出，設定頁一次標完。
            if let Err(LayerError::Invalid(more)) = Policy::layered(base.as_deref().unwrap_or(""), Some(&render_local(&values))) {
                let fresh: Vec<SettingError> = more.into_iter().filter(|m| !errors.iter().any(|e| e.key == m.key)).collect();
                errors.extend(fresh);
            }
            return Err(SettingsRejection::keys(errors));
        }
        self.commit_local(&paths, base.as_deref(), &values)
    }

    /// 回復預設：從 local 檔移除這些鍵（`[]`＝全部）；local 檔語法壞掉時清空整個檔；剩下的組合不合法整筆拒絕。
    pub fn settings_reset(&self, keys: &[String]) -> Result<SettingsSnapshot, SettingsRejection> {
        let paths = self.settings_paths().ok_or_else(SettingsRejection::not_configured)?;
        let _gate = self.settings_gate();
        let unknown: Vec<SettingError> = keys
            .iter()
            .filter(|k| !is_settable(k))
            .map(|k| SettingError::new(k, "這個設定不能從設定頁修改"))
            .collect();
        if !unknown.is_empty() {
            return Err(SettingsRejection::keys(unknown));
        }
        let base = read_optional(&paths.base).map_err(|e| io_rejection("讀取", &paths.base, &e))?;
        let local = read_optional(&paths.local).map_err(|e| io_rejection("讀取", &paths.local, &e))?;
        let mut values = match parse_local(local.as_deref()) {
            LocalValues::Values(v) => v,
            LocalValues::Broken(_) => BTreeMap::new(),
        };
        if keys.is_empty() {
            values.clear();
        } else {
            values.retain(|k, _| !keys.iter().any(|r| r == k));
        }
        self.commit_local(&paths, base.as_deref(), &values)
    }

    /// 先算疊層（不合法就拒絕、不寫檔），再原子寫入並立即 `set_policy`。
    fn commit_local(
        &self,
        paths: &SettingsPaths,
        base: Option<&str>,
        values: &BTreeMap<&'static str, toml::Value>,
    ) -> Result<SettingsSnapshot, SettingsRejection> {
        let rendered = render_local(values);
        let layered = Policy::layered(base.unwrap_or(""), Some(&rendered)).map_err(SettingsRejection::from)?;
        write_atomic(&paths.local, &rendered).map_err(|e| io_rejection("寫入", &paths.local, &e))?;
        self.set_policy(layered.policy);
        tracing::info!("個人設定已更新：{}", if layered.overridden.is_empty() { "全部預設".to_string() } else { layered.overridden.join("、") });
        Ok(snapshot(paths, base, Some(&rendered)))
    }

    /// 熱載（每 5 秒看兩檔 mtime，有變就呼叫）：base 讀不到或不合法 → 沿用上一次的生效值（回 Err）；
    /// local 不合法 → 只用 base（記 log、回 Ok）。
    pub fn reload_policy_files(&self) -> Result<(), String> {
        let paths = self.settings_paths().ok_or("設定檔位置還沒設定")?;
        // 與設定頁儲存序列化：熱載讀到舊 local 之後才 set_policy，不能蓋掉剛存好的新值。
        let _gate = self.settings_gate();
        let base = std::fs::read_to_string(&paths.base).map_err(|e| format!("讀不到 {}（沿用上一次的生效值）：{e}", paths.base.display()))?;
        Policy::from_toml_str(&base).map_err(|e| format!("{} 解析失敗（沿用上一次的生效值）：{e}", paths.base.display()))?;
        let local = read_optional(&paths.local).unwrap_or_else(|e| {
            tracing::error!("讀不到 {}（只用預設值）：{e}", paths.local.display());
            None
        });
        let loaded = effective_policy(Some(&base), local.as_deref());
        if let Some(err) = &loaded.local_error {
            tracing::error!("{} 不合法，只用 policy.toml：{err}", paths.local.display());
        }
        if !loaded.ignored.is_empty() {
            tracing::warn!("{} 有不開放的鍵（已忽略）：{}", paths.local.display(), loaded.ignored.join("、"));
        }
        self.set_policy(loaded.policy);
        Ok(())
    }

    /// 主視窗「Agent 連線」「資料與帳本」「關於」的資料（CONTRACT §15 `app_status`）。
    pub async fn app_status(&self, env: &StatusEnv, now: Ts) -> Result<AppStatus, ServerError> {
        let health = self.health(now).await?;
        let (home, kunsu_dir) = (env.home.clone(), env.kunsu_dir.clone());
        let integrations = tokio::task::spawn_blocking(move || check_integrations(&home, &kunsu_dir))
            .await
            .unwrap_or_default();
        Ok(AppStatus {
            version: health.version,
            uptime_s: health.uptime_s,
            server_addr: env.server_addr.clone(),
            vault_root: self.ledger().vault_root().to_string_lossy().into_owned(),
            db_path: self.db_path().to_string_lossy().into_owned(),
            ledger: LedgerStatus {
                backlog: health.ledger_backlog,
                open_json_dirty: health.open_json_dirty,
                stuck_s: health.ledger_stuck_s,
            },
            integrations,
        })
    }
}

// ---------------------------------------------------------------------------
// app_status
// ---------------------------------------------------------------------------

/// `app_status` 要的環境（shell 注入；測試用 tempdir 當 HOME）。
#[derive(Debug, Clone)]
pub struct StatusEnv {
    pub server_addr: String,
    pub home: PathBuf,
    pub kunsu_dir: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
pub struct AppStatus {
    pub version: String,
    pub uptime_s: u64,
    pub server_addr: String,
    pub vault_root: String,
    pub db_path: String,
    pub ledger: LedgerStatus,
    pub integrations: Vec<Integration>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LedgerStatus {
    pub backlog: u64,
    pub open_json_dirty: bool,
    pub stuck_s: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IntegrationStatus {
    Ok,
    Missing,
    Unknown,
}

#[derive(Debug, Clone, Serialize)]
pub struct Integration {
    pub id: String,
    pub status: IntegrationStatus,
    pub detail: String,
    /// 未設定時給使用者複製到終端機的指令（app 不執行）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix: Option<String>,
}

fn integration(id: &str, status: IntegrationStatus, detail: impl Into<String>, fix: Option<String>) -> Integration {
    let fix = if status == IntegrationStatus::Missing { fix } else { None };
    Integration { id: id.to_string(), status, detail: detail.into(), fix }
}

/// 路徑在 HOME 底下時用 `~/…` 顯示。
fn tilde(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

/// `kunsu doctor` 印的 Claude MCP 註冊指令（同源，drift 由測試檢查）。
pub fn claude_mcp_fix(kunsu_dir: &Path) -> String {
    format!(
        "claude mcp add --scope user --transport http kunsu {MCP_URL} --header \"Authorization: Bearer $(cat {}/token)\"",
        kunsu_dir.display()
    )
}

/// Codex 接上 Kunsu 的修復指令：安裝本機外掛（CONTRACT §16；`kunsu doctor` 印同一行）。貼上就能執行——舊的
/// `[mcp_servers.kunsu]` 設定段還要自己補 token 設定，不能直接用（2026-10-09 主視窗 QA 第 1 輪）。
pub const CODEX_FIX: &str = "python3 ~/.kunsu/plugins/kunsu/scripts/manage.py install";

/// 重新安裝 kunsu CLI：App 每次打開都會把同版的 CLI 放到 `~/.kunsu/bin/kunsu`（AC-MU-06）。
pub const CLI_FIX: &str = "open -a Kunsu";

/// Codex 設定裡 kunsu 有沒有接上（AC-LP-05 的 doctor 規則）：啟用的直接連線 `[mcp_servers.kunsu]`（沒寫 enabled 視為啟用），
/// 或啟用的本機外掛 `[plugins."kunsu@…"]`。
pub fn codex_kunsu_registered(config: &toml::Table) -> bool {
    let enabled = |v: &toml::Value| v.is_table() && v.get("enabled").and_then(toml::Value::as_bool).unwrap_or(true);
    let direct = config.get("mcp_servers").and_then(|m| m.get("kunsu")).is_some_and(enabled);
    let plugin = config
        .get("plugins")
        .and_then(toml::Value::as_table)
        .is_some_and(|plugins| plugins.iter().any(|(name, v)| name.split('@').next() == Some("kunsu") && enabled(v)));
    direct || plugin
}

/// `~/.claude.json` 只看 `mcpServers` 的鍵名；值一律略過不保留（檔案可能很大、裡面有 token）。
#[derive(serde::Deserialize)]
struct ClaudeJson {
    #[serde(rename = "mcpServers", default)]
    mcp_servers: Option<BTreeMap<String, serde::de::IgnoredAny>>,
}

/// 四項 Agent 連線（順序：claude_mcp、codex_mcp、cli、skill）。只看檔案存在與鍵，detail 只有固定文字與路徑。
pub fn check_integrations(home: &Path, kunsu_dir: &Path) -> Vec<Integration> {
    use IntegrationStatus::{Missing, Ok as Good, Unknown};

    let claude_path = home.join(".claude.json");
    let claude_shown = tilde(&claude_path, home);
    let claude = match std::fs::read(&claude_path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            integration("claude_mcp", Missing, format!("Claude Code 還沒註冊 kunsu MCP（找不到 {claude_shown}）"), Some(claude_mcp_fix(kunsu_dir)))
        }
        Err(e) => integration("claude_mcp", Unknown, format!("讀不到 {claude_shown}（{}），無法確認", e.kind()), None),
        Ok(bytes) => match serde_json::from_slice::<ClaudeJson>(&bytes) {
            Err(_) => integration("claude_mcp", Unknown, format!("{claude_shown} 格式無法解析，無法確認"), None),
            Ok(c) if c.mcp_servers.as_ref().is_some_and(|m| m.contains_key("kunsu")) => {
                integration("claude_mcp", Good, format!("Claude Code 已註冊 kunsu MCP（{claude_shown}）"), None)
            }
            Ok(_) => integration(
                "claude_mcp",
                Missing,
                format!("Claude Code 還沒註冊 kunsu MCP（{claude_shown} 沒有 mcpServers.kunsu）"),
                Some(claude_mcp_fix(kunsu_dir)),
            ),
        },
    };

    let codex_path = home.join(".codex").join("config.toml");
    let codex_shown = tilde(&codex_path, home);
    let codex = match std::fs::read_to_string(&codex_path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            integration("codex_mcp", Missing, format!("Codex 還沒接上 kunsu（找不到 {codex_shown}）"), Some(CODEX_FIX.to_string()))
        }
        Err(e) => integration("codex_mcp", Unknown, format!("讀不到 {codex_shown}（{}），無法確認", e.kind()), None),
        Ok(raw) => match raw.parse::<toml::Table>() {
            Err(_) => integration("codex_mcp", Unknown, format!("{codex_shown} 格式無法解析，無法確認"), None),
            Ok(t) if codex_kunsu_registered(&t) => {
                integration("codex_mcp", Good, format!("Codex 已接上 kunsu（{codex_shown}）"), None)
            }
            Ok(_) => integration(
                "codex_mcp",
                Missing,
                format!("Codex 還沒接上 kunsu（{codex_shown} 沒有啟用的 [mcp_servers.kunsu] 或 kunsu 外掛）"),
                Some(CODEX_FIX.to_string()),
            ),
        },
    };

    let cli_path = kunsu_dir.join("bin").join("kunsu");
    let cli_shown = tilde(&cli_path, home);
    let cli = match std::fs::metadata(&cli_path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            integration("cli", Missing, format!("找不到 kunsu CLI（{cli_shown}）"), Some(CLI_FIX.to_string()))
        }
        Err(e) => integration("cli", Unknown, format!("讀不到 {cli_shown}（{}），無法確認", e.kind()), None),
        Ok(meta) if meta.is_file() && is_executable(&meta) => {
            integration("cli", Good, format!("kunsu CLI 已安裝（{cli_shown}）"), None)
        }
        Ok(_) => integration("cli", Missing, format!("{cli_shown} 存在但不能執行"), Some(CLI_FIX.to_string())),
    };

    let skill_path = home.join(".claude").join("skills").join("kunsu").join("SKILL.md");
    let skill_shown = tilde(&skill_path, home);
    let skill = match std::fs::metadata(&skill_path) {
        Ok(meta) if meta.is_file() => integration("skill", Good, format!("kunsu skill 已安裝（{skill_shown}）"), None),
        Ok(_) => integration("skill", Missing, format!("{skill_shown} 不是檔案"), None),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            integration("skill", Missing, format!("Claude Code 的 kunsu skill 沒有安裝（找不到 {skill_shown}）"), None)
        }
        Err(e) => integration("skill", Unknown, format!("讀不到 {skill_shown}（{}），無法確認", e.kind()), None),
    };

    vec![claude, codex, cli, skill]
}

#[cfg(unix)]
fn is_executable(meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(_meta: &std::fs::Metadata) -> bool {
    true
}
