//! 資料位置（PRD §31.3；BDD multi-user AC-MU-01）：不綁任何人的個人路徑。
//!
//! 順序：環境變數 `KUNSU_VAULT_ROOT` → `~/.kunsu/config.json` 的 `data_dir` → 舊資料夾 `~/Desktop/personal-system`
//! （存在時沿用，作者公開前的安裝不必搬資料）→ `~/Kunsu`。純函數：讀檔與檢查存在由呼叫端做。

use std::path::{Path, PathBuf};

/// 公開前的舊資料夾（相對家目錄）。
pub const LEGACY_VAULT: &str = "Desktop/personal-system";
/// 新使用者的預設資料夾（相對家目錄）。
pub const DEFAULT_VAULT: &str = "Kunsu";

fn expand(home: &Path, raw: &str) -> Option<PathBuf> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }
    Some(match s.strip_prefix("~/") {
        Some(rest) => home.join(rest),
        None if s == "~" => home.to_path_buf(),
        None => PathBuf::from(s),
    })
}

pub fn resolve_vault_root(env: Option<&str>, config_json: Option<&str>, home: &Path, legacy_exists: bool) -> PathBuf {
    if let Some(p) = env.and_then(|e| expand(home, e)) {
        return p;
    }
    let from_config = config_json
        .and_then(|c| serde_json::from_str::<serde_json::Value>(c).ok())
        .and_then(|v| v.get("data_dir").and_then(|d| d.as_str()).map(str::to_string))
        .and_then(|d| expand(home, &d));
    if let Some(p) = from_config {
        return p;
    }
    if legacy_exists {
        return home.join(LEGACY_VAULT);
    }
    home.join(DEFAULT_VAULT)
}

/// Meta 廣告整合的設定（選用，只有設定了 Meta 廣告整合的使用者才有）。
pub fn meta_ads_config(vault: &Path) -> PathBuf {
    vault.join("projects/meta-ads/config.json")
}
