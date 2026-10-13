//! CONTRACT §10 policy.toml 解析。所有 section / 鍵都有預設值：
//! 空檔案、缺 section、缺鍵都解析成功並落回預設；未知鍵大聲失敗
//! （調參打錯字要立刻被發現，不准無聲用預設）。
//!
//! CONTRACT §15 設定疊層：[`Policy::layered`] 把設定頁寫的 `policy.local.toml` 逐鍵疊在 base 上，只認
//! [`SETTABLE_KEYS`]、逐鍵驗證範圍，不合法整筆拒絕（INV-26：設定不能放寬不變量，例如 INV-14 的 14 天上限）。

use chrono::NaiveTime;
use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error as _};

use crate::time::{self, Ts};

/// `"HH:MM"` 字串 ↔ NaiveTime 的 newtype（policy 時窗鍵專用；台北本地時間解釋）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hhmm(pub NaiveTime);

impl Hhmm {
    pub fn new(h: u32, m: u32) -> Hhmm {
        Hhmm(NaiveTime::from_hms_opt(h, m, 0).expect("合法 HH:MM"))
    }
}

impl Serialize for Hhmm {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.0.format("%H:%M").to_string())
    }
}

impl<'de> Deserialize<'de> for Hhmm {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(d)?;
        NaiveTime::parse_from_str(&raw, "%H:%M")
            .map(Hhmm)
            .map_err(|e| {
                serde::de::Error::custom(format!("時窗必須是 \"HH:MM\"（收到 {raw:?}）：{e}"))
            })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ForcePolicy {
    /// 每一輪從建立／dismiss 到強制詢問的完整分鐘數。
    pub cycle_min: u32,
    /// 強制詢問前幾分鐘進入警告狀態。
    pub warn_before_min: u32,
    /// guard 擋住強制蓋板時的持久化重試間隔。
    pub deferred_retry_s: u32,
}

impl Default for ForcePolicy {
    fn default() -> Self {
        ForcePolicy {
            cycle_min: 15,
            warn_before_min: 3,
            deferred_retry_s: 1,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WindowsPolicy {
    /// T1 批次窗（台北本地時刻）。
    pub t1_batch: Vec<Hhmm>,
    /// [start, end)，start > end 表示跨午夜。
    pub quiet_hours: [Hhmm; 2],
    /// 睡眠時間開關（設定頁）；false 時 quiet hours 不擋強制詢問。
    pub quiet_enabled: bool,
}

impl Default for WindowsPolicy {
    fn default() -> Self {
        WindowsPolicy {
            t1_batch: vec![Hhmm::new(9, 45), Hhmm::new(21, 30)],
            quiet_hours: [Hhmm::new(23, 0), Hhmm::new(9, 0)],
            quiet_enabled: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ExpiryPolicy {
    pub t1_default_after_h: u32,
    pub t2_supersede_after_d: u32,
}

impl Default for ExpiryPolicy {
    fn default() -> Self {
        ExpiryPolicy {
            t1_default_after_h: 48,
            t2_supersede_after_d: 14,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct QuotaPolicy {
    pub per_source_daily_create: u32,
}

impl Default for QuotaPolicy {
    fn default() -> Self {
        QuotaPolicy {
            per_source_daily_create: 12,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FatiguePolicy {
    pub skip_rate_window_d: u32,
    pub skip_rate_downgrade: f64,
}

impl Default for FatiguePolicy {
    fn default() -> Self {
        FatiguePolicy {
            skip_rate_window_d: 7,
            skip_rate_downgrade: 0.5,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct DiscordPolicy {
    /// 空字串 = 沿用 meta-ads config.json 的 mkt_daily webhook（server 層解釋）。
    pub webhook: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct Policy {
    pub force: ForcePolicy,
    pub windows: WindowsPolicy,
    pub expiry: ExpiryPolicy,
    pub quota: QuotaPolicy,
    pub fatigue: FatiguePolicy,
    pub discord: DiscordPolicy,
}

impl Policy {
    /// 解析 policy.toml 內容（檔案由呼叫端讀，core 不碰 I/O）。
    pub fn from_toml_str(raw: &str) -> Result<Policy, toml::de::Error> {
        let policy: Policy = toml::from_str(raw)?;
        if policy.force.cycle_min == 0 {
            return Err(toml::de::Error::custom("force.cycle_min 必須大於 0"));
        }
        if policy.force.warn_before_min >= policy.force.cycle_min {
            return Err(toml::de::Error::custom(
                "force.warn_before_min 必須小於 force.cycle_min",
            ));
        }
        if policy.force.deferred_retry_s == 0 {
            return Err(toml::de::Error::custom("force.deferred_retry_s 必須大於 0"));
        }
        Ok(policy)
    }

    /// `now`（UTC）是否落在台北 quiet hours。睡眠時間關掉（`quiet_enabled = false`）時恆 false。
    pub fn in_quiet_hours(&self, now: Ts) -> bool {
        if !self.windows.quiet_enabled {
            return false;
        }
        let [start, end] = self.windows.quiet_hours;
        time::in_taipei_window(now, start.0, end.0)
    }

    /// `after` 之後最近的一個 T1 批次窗（UTC）。批次窗為空回 None。
    pub fn next_t1_batch(&self, after: Ts) -> Option<Ts> {
        self.windows
            .t1_batch
            .iter()
            .map(|h| time::next_taipei_occurrence(after, h.0))
            .min()
    }
}

// ---------------------------------------------------------------------------
// 設定疊層（CONTRACT §15）
// ---------------------------------------------------------------------------

/// 設定頁能改的鍵，依 CONTRACT §15 表的順序（`overridden` 與錯誤都照這個順序）。
pub const SETTABLE_KEYS: [&str; 9] = [
    "force.cycle_min",
    "force.warn_before_min",
    "windows.quiet_enabled",
    "windows.quiet_hours",
    "expiry.t1_default_after_h",
    "expiry.t2_supersede_after_d",
    "quota.per_source_daily_create",
    "fatigue.skip_rate_window_d",
    "fatigue.skip_rate_downgrade",
];

/// 是不是設定頁能改的鍵。
pub fn is_settable(key: &str) -> bool {
    SETTABLE_KEYS.contains(&key)
}

fn key_rank(key: &str) -> usize {
    SETTABLE_KEYS.iter().position(|k| *k == key).unwrap_or(SETTABLE_KEYS.len())
}

/// 一個鍵的錯誤：人話、帶範圍（設定頁逐欄顯示）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingError {
    pub key: String,
    pub message: String,
}

impl SettingError {
    pub fn new(key: &str, message: impl Into<String>) -> SettingError {
        SettingError { key: key.to_string(), message: message.into() }
    }
}

/// `Policy::layered` 的錯誤：local 不是合法 TOML，或某些鍵不合法（全部列出）。
#[derive(Debug, Clone, PartialEq)]
pub enum LayerError {
    Syntax(String),
    Invalid(Vec<SettingError>),
}

impl std::fmt::Display for LayerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LayerError::Syntax(msg) => write!(f, "個人設定檔不是合法的 TOML：{}", msg.trim()),
            LayerError::Invalid(errors) => {
                let parts: Vec<String> = errors.iter().map(|e| format!("{}：{}", e.key, e.message)).collect();
                write!(f, "個人設定有不合法的值（{}）", parts.join("；"))
            }
        }
    }
}

impl std::error::Error for LayerError {}

/// 疊層結果：生效 policy、被覆寫的鍵（local 裡出現的可設定鍵）、被忽略的鍵（local 裡不開放的鍵）。
#[derive(Debug, Clone, PartialEq)]
pub struct Layered {
    pub policy: Policy,
    pub overridden: Vec<String>,
    pub ignored: Vec<String>,
}

fn int_in(key: &str, v: &toml::Value, lo: i64, hi: i64, message: &str) -> Result<u32, SettingError> {
    match v.as_integer() {
        Some(n) if (lo..=hi).contains(&n) => Ok(n as u32),
        _ => Err(SettingError::new(key, message)),
    }
}

/// TOML 語法錯誤 → 一行人話：「第 N 行第 M 字：原因」（不貼原檔片段）。
fn syntax_summary(e: &toml::de::Error, raw: &str) -> String {
    let reason = e.message().lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join("，");
    let Some(before) = e.span().and_then(|span| raw.get(..span.start)) else { return reason };
    let line = before.matches('\n').count() + 1;
    let column = before.rsplit('\n').next().map(|l| l.chars().count()).unwrap_or(0) + 1;
    format!("第 {line} 行第 {column} 字：{reason}")
}

/// 24 小時制兩位數 `HH:MM`（`9:00`、`23:00:00`、全形冒號都不收）。
fn strict_hhmm(raw: &str) -> Option<Hhmm> {
    let b = raw.as_bytes();
    if b.len() != 5 || b[2] != b':' || ![0, 1, 3, 4].iter().all(|&i| b[i].is_ascii_digit()) {
        return None;
    }
    NaiveTime::parse_from_str(raw, "%H:%M").ok().map(Hhmm)
}

fn quiet_hours_value(key: &str, v: &toml::Value) -> Result<[Hhmm; 2], SettingError> {
    let format = || SettingError::new(key, "睡眠時間要是兩個 24 小時制時間（HH:MM），例如 [\"23:00\", \"09:00\"]");
    let arr = v.as_array().ok_or_else(format)?;
    let [start, end] = arr.as_slice() else { return Err(format()) };
    let start = start.as_str().and_then(strict_hhmm).ok_or_else(format)?;
    let end = end.as_str().and_then(strict_hhmm).ok_or_else(format)?;
    if start == end {
        return Err(SettingError::new(key, "睡眠時間的開始與結束不能相同"));
    }
    Ok([start, end])
}

/// 把一個可設定鍵的值驗證後套到 policy 上（提前警告與週期的交叉檢查在 [`Policy::layered`] 做）。
fn apply_setting(policy: &mut Policy, key: &str, v: &toml::Value) -> Result<(), SettingError> {
    match key {
        "force.cycle_min" => policy.force.cycle_min = int_in(key, v, 5, 120, "週期長度要是 5–120 分鐘之間的整數")?,
        "force.warn_before_min" => {
            policy.force.warn_before_min = match v.as_integer() {
                Some(n) if n >= 1 && n <= i64::from(u32::MAX) => n as u32,
                _ => {
                    return Err(SettingError::new(
                        key,
                        format!(
                            "提前警告要是 1–{} 分鐘之間的整數（要小於週期長度）",
                            policy.force.cycle_min.saturating_sub(1)
                        ),
                    ));
                }
            }
        }
        "windows.quiet_enabled" => {
            policy.windows.quiet_enabled =
                v.as_bool().ok_or_else(|| SettingError::new(key, "睡眠時間開關只能是開（true）或關（false）"))?
        }
        "windows.quiet_hours" => policy.windows.quiet_hours = quiet_hours_value(key, v)?,
        "expiry.t1_default_after_h" => {
            policy.expiry.t1_default_after_h = int_in(key, v, 1, 336, "一般題到期時間要是 1–336 小時之間的整數")?
        }
        "expiry.t2_supersede_after_d" => {
            policy.expiry.t2_supersede_after_d =
                int_in(key, v, 1, 14, "題目最長等待天數要是 1–14 天之間的整數（所有題目，最多 14 天）")?
        }
        "quota.per_source_daily_create" => {
            policy.quota.per_source_daily_create =
                int_in(key, v, 1, 100, "每個來源每天最多建題數要是 1–100 題之間的整數")?
        }
        "fatigue.skip_rate_window_d" => {
            policy.fatigue.skip_rate_window_d = int_in(key, v, 1, 30, "疲勞觀察天數要是 1–30 天之間的整數")?
        }
        "fatigue.skip_rate_downgrade" => {
            let rate = match v {
                toml::Value::Float(f) => Some(*f),
                toml::Value::Integer(n) => Some(*n as f64),
                _ => None,
            };
            policy.fatigue.skip_rate_downgrade = match rate {
                Some(r) if r.is_finite() && (0.1..=1.0).contains(&r) => r,
                _ => return Err(SettingError::new(key, "略過率門檻要在 10%–100% 之間（0.1–1.0）")),
            }
        }
        _ => return Err(SettingError::new(key, "這個設定不能從設定頁修改")),
    }
    Ok(())
}

impl Policy {
    /// CONTRACT §15：把 `local`（設定頁寫的 `policy.local.toml`）逐鍵疊在 `base` 上。純函數。
    ///
    /// - base 本身不合法時照舊退回 `Policy::default()`（與啟動時讀 policy.toml 相同）。
    /// - local 只認 [`SETTABLE_KEYS`]；其他鍵（`deferred_retry_s`、`t1_batch`、`discord.webhook`、未知鍵）忽略並列在 `ignored`。
    /// - 每個鍵依範圍驗證；提前警告一定小於（生效的）週期長度；任一鍵不合法就整筆 `Invalid`，不套用部分值。
    pub fn layered(base: &str, local: Option<&str>) -> Result<Layered, LayerError> {
        let mut policy = Policy::from_toml_str(base).unwrap_or_default();
        let Some(local) = local else {
            return Ok(Layered { policy, overridden: Vec::new(), ignored: Vec::new() });
        };
        let table: toml::Table = local.parse().map_err(|e: toml::de::Error| LayerError::Syntax(syntax_summary(&e, local)))?;

        let mut present: Vec<(&'static str, &toml::Value)> = Vec::new();
        let mut ignored: Vec<String> = Vec::new();
        for (section, value) in &table {
            let Some(inner) = value.as_table() else {
                ignored.push(section.clone());
                continue;
            };
            for (name, v) in inner {
                let path = format!("{section}.{name}");
                match SETTABLE_KEYS.iter().find(|k| **k == path) {
                    Some(key) => present.push((*key, v)),
                    None => ignored.push(path),
                }
            }
        }
        present.sort_by_key(|(key, _)| key_rank(key));

        let mut errors: Vec<SettingError> = Vec::new();
        for (key, v) in &present {
            if let Err(e) = apply_setting(&mut policy, key, v) {
                errors.push(e);
            }
        }
        let force_touched = present.iter().any(|(k, _)| k.starts_with("force."));
        let force_errors = errors.iter().any(|e| e.key.starts_with("force."));
        if force_touched && !force_errors && policy.force.warn_before_min >= policy.force.cycle_min {
            errors.push(SettingError::new(
                "force.warn_before_min",
                format!("提前警告要小於週期長度（1–{} 分鐘）", policy.force.cycle_min - 1),
            ));
        }
        if !errors.is_empty() {
            errors.sort_by_key(|e| key_rank(&e.key));
            return Err(LayerError::Invalid(errors));
        }
        Ok(Layered { policy, overridden: present.iter().map(|(k, _)| k.to_string()).collect(), ignored })
    }
}

/// 設定頁讀取面（`settings_get` 的 `effective`／`defaults`）：只有可設定的值，quiet hours 拆成開始與結束。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SettingsView {
    pub force: ForceView,
    pub windows: WindowsView,
    pub expiry: ExpiryView,
    pub quota: QuotaView,
    pub fatigue: FatigueView,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ForceView {
    pub cycle_min: u32,
    pub warn_before_min: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WindowsView {
    pub quiet_enabled: bool,
    pub quiet_start: Hhmm,
    pub quiet_end: Hhmm,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExpiryView {
    pub t1_default_after_h: u32,
    pub t2_supersede_after_d: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QuotaView {
    pub per_source_daily_create: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FatigueView {
    pub skip_rate_window_d: u32,
    pub skip_rate_downgrade: f64,
}

impl From<&Policy> for SettingsView {
    fn from(p: &Policy) -> SettingsView {
        let [quiet_start, quiet_end] = p.windows.quiet_hours;
        SettingsView {
            force: ForceView { cycle_min: p.force.cycle_min, warn_before_min: p.force.warn_before_min },
            windows: WindowsView { quiet_enabled: p.windows.quiet_enabled, quiet_start, quiet_end },
            expiry: ExpiryView {
                t1_default_after_h: p.expiry.t1_default_after_h,
                t2_supersede_after_d: p.expiry.t2_supersede_after_d,
            },
            quota: QuotaView { per_source_daily_create: p.quota.per_source_daily_create },
            fatigue: FatigueView {
                skip_rate_window_d: p.fatigue.skip_rate_window_d,
                skip_rate_downgrade: p.fatigue.skip_rate_downgrade,
            },
        }
    }
}
