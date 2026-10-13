//! 批准簽章鏈（CONTRACT §6；BDD money-approval）。
//!
//! - `KeyProvider`：HMAC key 來源。prod 走 macOS Keychain
//!   （service `me.ray.kunsu.approval-hmac`，首次自動生成）；測試注入固定 key。
//! - `normalized_param_hash`：executor.py 同名函數的逐字元一致移植——
//!   使用者在卡片上批的就是這組欄位（所見即所簽）。
//! - `run_approval_chain`：點批准當下 spawn prepare → 解析 token → 讀 pending
//!   spec → 算 param_hash → 組 record → HMAC 簽章 → 落檔
//!   `projects/meta-ads/data/executor_pending/approvals/<question_id>.json`。
//! - `verify_and_consume`：`POST /approvals/verify` 的內核——驗簽 → TTL（900s，
//!   answered_at 起算）→ 單次消耗（DB + ledger）。任一失敗回明確 reason。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use kunsu_core::time::Ts;
use kunsu_core::types::Question;
use serde::Serialize;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use crate::service::{ApprovalOutcome, ServerHandle};

type HmacSha256 = Hmac<Sha256>;

/// Keychain service 名（CONTRACT §0）。
pub const KEYCHAIN_SERVICE: &str = "me.ray.kunsu.approval-hmac";
pub const KEYCHAIN_ACCOUNT: &str = "kunsu";
/// approval record 有效期：使用者點擊（answered_at）起 15 分鐘（executor.py 同值）。
pub const APPROVAL_TTL_SECONDS: i64 = 900;

/// HMAC key 來源。錯誤用人話字串（上層直接進 reason / log）。
pub trait KeyProvider: Send + Sync {
    fn hmac_key(&self) -> Result<Vec<u8>, String>;
}

/// 測試/開發注入用固定 key。
pub struct StaticKeyProvider(pub Vec<u8>);

impl KeyProvider for StaticKeyProvider {
    fn hmac_key(&self) -> Result<Vec<u8>, String> {
        Ok(self.0.clone())
    }
}

/// macOS Keychain 實作：generic password，首次讀不到即生成 32 bytes 並寫入。
/// key ACL 由 Keychain 管——只有本 app（同簽章身分）能讀。
#[cfg(target_os = "macos")]
pub struct KeychainKeyProvider;

#[cfg(target_os = "macos")]
impl KeyProvider for KeychainKeyProvider {
    fn hmac_key(&self) -> Result<Vec<u8>, String> {
        use security_framework::passwords::{get_generic_password, set_generic_password};
        match get_generic_password(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT) {
            Ok(bytes) => Ok(bytes),
            Err(_) => {
                let key: Vec<u8> = uuid::Uuid::new_v4()
                    .into_bytes()
                    .into_iter()
                    .chain(uuid::Uuid::new_v4().into_bytes())
                    .collect();
                set_generic_password(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT, &key)
                    .map_err(|e| format!("Keychain 寫入失敗：{e}"))?;
                Ok(key)
            }
        }
    }
}

/// 預設 key provider：macOS 走 Keychain；其他平台落檔（開發/CI 便利）。
pub fn default_key_provider(fallback_dir: &Path) -> std::sync::Arc<dyn KeyProvider> {
    #[cfg(target_os = "macos")]
    {
        let _ = fallback_dir;
        std::sync::Arc::new(KeychainKeyProvider)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let path = fallback_dir.join("approval-hmac.key");
        let key = match std::fs::read(&path) {
            Ok(k) if !k.is_empty() => k,
            _ => {
                let key: Vec<u8> = uuid::Uuid::new_v4()
                    .into_bytes()
                    .into_iter()
                    .chain(uuid::Uuid::new_v4().into_bytes())
                    .collect();
                let _ = std::fs::create_dir_all(fallback_dir);
                let _ = std::fs::write(&path, &key);
                key
            }
        };
        std::sync::Arc::new(StaticKeyProvider(key))
    }
}

// ---------------------------------------------------------------------------
// param_hash（executor.py normalized_param_hash 逐字元一致移植）
// ---------------------------------------------------------------------------

/// executor.py `normalized_param_hash` 的 Rust 移植。演算法必須逐字元一致：
/// 取 spec 中定義這筆變更的欄位子集 → canonical JSON（sort_keys、separators
/// 無空白、ensure_ascii=False，即 serde_json 預設 compact + BTreeMap 排序）
/// → sha256 hexdigest。缺鍵以 null 填（Python `spec.get(k)`）。
pub fn normalized_param_hash(spec: &Value) -> String {
    let action = spec.get("action").and_then(Value::as_str).unwrap_or("");
    let keys: Vec<String> = match action {
        "update_budget" => ["action", "entity", "product", "new_budget", "current_budget"]
            .iter()
            .map(|s| s.to_string())
            .collect(),
        "update_field" => ["action", "entity", "product", "field", "new_value", "current"]
            .iter()
            .map(|s| s.to_string())
            .collect(),
        _ => {
            let mut ks: Vec<String> = spec
                .as_object()
                .map(|o| {
                    o.keys()
                        .filter(|k| !k.starts_with('_') && k.as_str() != "prepared_at")
                        .cloned()
                        .collect()
                })
                .unwrap_or_default();
            ks.sort();
            ks
        }
    };
    let mut obj: BTreeMap<String, Value> = BTreeMap::new();
    for k in keys {
        let v = spec.get(&k).cloned().unwrap_or(Value::Null);
        obj.insert(k, v);
    }
    let raw = serde_json::to_string(&obj).expect("BTreeMap<String, Value> 恆可序列化");
    hex::encode(Sha256::digest(raw.as_bytes()))
}

// ---------------------------------------------------------------------------
// HMAC domain separation（Codex 覆檢 P0）
// ---------------------------------------------------------------------------
//
// Keychain 只存一把 master key，但**任何 MAC 都不用 master key 直接算**：
// - approval record 用 `derive_subkey(master, DOMAIN_APPROVAL_RECORD)`
// - UI 稽核答案簽章用 `derive_subkey(master, DOMAIN_UI_ANSWER)`
// 兩把子鍵由 HMAC(master, domain) 推導（單向），且 UI 簽章的訊息採
// 「domain 前綴 + 每欄位長度編碼」——拼接偽造（把 canonical approval JSON
// 切三段塞進 question_id|choice_id|params）在兩層都被杜絕。
// approval 子鍵算出的 MAC 只出現在 approval record，永不對外簽任意資料。

/// approval record 簽章 domain（版本化；改演算法必須換字串）。
pub const DOMAIN_APPROVAL_RECORD: &str = "kunsu/v1/approval-record";
/// UI 稽核答案簽章 domain。
pub const DOMAIN_UI_ANSWER: &str = "kunsu/v1/ui-answer";

/// HKDF-style 子鍵推導：HMAC-SHA256(master, domain)。
pub fn derive_subkey(master: &[u8], domain: &str) -> Result<Vec<u8>, String> {
    let mut mac =
        HmacSha256::new_from_slice(master).map_err(|e| format!("HMAC key 非法：{e}"))?;
    mac.update(domain.as_bytes());
    Ok(mac.finalize().into_bytes().to_vec())
}

/// UI 稽核答案簽章（Tauri IPC 用；CONTRACT §2 Answer.signature）。
/// 訊息編碼：`DOMAIN_UI_ANSWER \0` + 每欄位 `(len as u64 BE || bytes)`——
/// 長度編碼杜絕欄位邊界挪移的拼接攻擊。
pub fn ui_answer_signature(
    master: &[u8],
    question_id: &str,
    choice_id: &str,
    params: &str,
) -> Result<String, String> {
    let subkey = derive_subkey(master, DOMAIN_UI_ANSWER)?;
    let mut mac =
        HmacSha256::new_from_slice(&subkey).map_err(|e| format!("HMAC key 非法：{e}"))?;
    mac.update(DOMAIN_UI_ANSWER.as_bytes());
    mac.update(&[0u8]);
    for field in [question_id, choice_id, params] {
        mac.update(&(field.len() as u64).to_be_bytes());
        mac.update(field.as_bytes());
    }
    Ok(hex::encode(mac.finalize().into_bytes()))
}

// ---------------------------------------------------------------------------
// record 簽章 / 驗章
// ---------------------------------------------------------------------------

/// 簽章覆蓋的五欄位（canonical：BTreeMap 排序 + compact）。
/// verify 端以「收到什麼就簽什麼」重算——任一欄位（含型別）被改動即驗章失敗。
const SIGNED_FIELDS: [&str; 5] =
    ["question_id", "executor_token", "param_hash", "answered_at", "nonce"];

fn canonical_record(record: &Map<String, Value>) -> String {
    let mut obj: BTreeMap<&str, Value> = BTreeMap::new();
    for k in SIGNED_FIELDS {
        obj.insert(k, record.get(k).cloned().unwrap_or(Value::Null));
    }
    serde_json::to_string(&obj).expect("canonical record 恆可序列化")
}

/// 對 record 補上 `hmac` 欄位。MAC = HMAC-SHA256（approval 子鍵）over
/// `DOMAIN_APPROVAL_RECORD \0 canonical`（hex 小寫）。`key` 是 master key，
/// 子鍵在函數內推導——呼叫端永遠拿不到 approval 子鍵。
pub fn sign_record(record: &mut Map<String, Value>, key: &[u8]) -> Result<(), String> {
    let subkey = derive_subkey(key, DOMAIN_APPROVAL_RECORD)?;
    let mut mac =
        HmacSha256::new_from_slice(&subkey).map_err(|e| format!("HMAC key 非法：{e}"))?;
    mac.update(DOMAIN_APPROVAL_RECORD.as_bytes());
    mac.update(&[0u8]);
    mac.update(canonical_record(record).as_bytes());
    let sig = hex::encode(mac.finalize().into_bytes());
    record.insert("hmac".to_string(), Value::String(sig));
    Ok(())
}

fn verify_signature(record: &Map<String, Value>, key: &[u8]) -> Result<(), String> {
    let provided = record
        .get("hmac")
        .and_then(Value::as_str)
        .ok_or_else(|| "record 缺 hmac 欄位".to_string())?;
    let bytes = hex::decode(provided).map_err(|_| "hmac 非合法 hex".to_string())?;
    let subkey = derive_subkey(key, DOMAIN_APPROVAL_RECORD)?;
    let mut mac =
        HmacSha256::new_from_slice(&subkey).map_err(|e| format!("HMAC key 非法：{e}"))?;
    mac.update(DOMAIN_APPROVAL_RECORD.as_bytes());
    mac.update(&[0u8]);
    mac.update(canonical_record(record).as_bytes());
    mac.verify_slice(&bytes)
        .map_err(|_| "HMAC 驗章失敗：record 遭竄改或非本機 Keychain key 簽出".to_string())
}

fn parse_answered_at(v: &Value) -> Option<f64> {
    if let Some(n) = v.as_f64() {
        return Some(n);
    }
    let s = v.as_str()?;
    if let Ok(n) = s.parse::<f64>() {
        return Some(n);
    }
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.with_timezone(&Utc).timestamp() as f64)
}

/// `/approvals/verify` 回應體。
#[derive(Debug, Clone, Serialize)]
pub struct VerifyOutcome {
    pub valid: bool,
    pub consumed_now: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl VerifyOutcome {
    fn reject(reason: impl Into<String>) -> Self {
        VerifyOutcome { valid: false, consumed_now: false, reason: Some(reason.into()) }
    }
}

/// 容許的時鐘偏移（answered_at 在未來超過此值必拒）。
pub const APPROVAL_CLOCK_SKEW_SECONDS: i64 = 60;

/// 驗簽 + TTL + 簽發登記比對 + 題目狀態 + 單次消耗（INV-9 / INV-10 + verify
/// 強化）。驗證順序：欄位齊全 → HMAC → TTL（含未來時間戳，±60s skew）→
/// approvals_issued 逐欄比對（只有 server 簽發過的 record 能消耗）→
/// 題目確為 yellow 錢題且由 ui 通道作答 → 單次消耗（`consume_approval`，同一段寫入鎖內：已消耗、這次授權已有結果、
/// 被這題更新的簽發取代都拒絕；消耗同時落 DB + ledger）。任一失敗回明確 reason、fail closed。
pub async fn verify_and_consume(
    handle: &ServerHandle,
    key: &dyn KeyProvider,
    record: &Value,
    now: Ts,
) -> VerifyOutcome {
    let Some(rec) = record.as_object() else {
        return VerifyOutcome::reject("approval_record 必須是 JSON object");
    };
    for k in ["question_id", "executor_token", "param_hash", "answered_at", "nonce", "hmac"] {
        if rec.get(k).is_none_or(Value::is_null) {
            return VerifyOutcome::reject(format!("approval record 缺欄位 {k}"));
        }
    }
    let hmac_key = match key.hmac_key() {
        Ok(k) => k,
        Err(e) => return VerifyOutcome::reject(format!("HMAC key 取得失敗（fail closed）：{e}")),
    };
    if let Err(e) = verify_signature(rec, &hmac_key) {
        return VerifyOutcome::reject(e);
    }
    let Some(answered_at) = parse_answered_at(&rec["answered_at"]) else {
        return VerifyOutcome::reject(format!("answered_at 無法解析：{:?}", rec["answered_at"]));
    };
    let age = now.timestamp() as f64 - answered_at;
    if age > APPROVAL_TTL_SECONDS as f64 {
        return VerifyOutcome::reject(format!(
            "TTL 過期：使用者點擊（answered_at）起已 {age:.0}s > {APPROVAL_TTL_SECONDS}s，請在 Kunsu 重新批一次"
        ));
    }
    if age < -(APPROVAL_CLOCK_SKEW_SECONDS as f64) {
        return VerifyOutcome::reject(format!(
            "answered_at 在未來 {:.0}s（容許時鐘偏移 ±{APPROVAL_CLOCK_SKEW_SECONDS}s）——拒絕未來時間戳",
            -age
        ));
    }
    let question_id = rec["question_id"].as_str().unwrap_or_default().to_string();
    let nonce = rec["nonce"].as_str().unwrap_or_default().to_string();
    if question_id.is_empty() || nonce.is_empty() {
        return VerifyOutcome::reject("question_id / nonce 必須是非空字串");
    }

    // 簽發登記比對：只有 server 自己簽發過（approvals_issued 有登記且逐欄
    // 一致）的 record 才可能被消耗——就算 key 外洩，離線偽造也進不來。
    let issued = match handle.approval_issued(&nonce).await {
        Ok(Some(i)) => i,
        Ok(None) => {
            return VerifyOutcome::reject(
                "此 record 不在簽發登記簿（approvals_issued）——非本 server 簽發，拒絕",
            );
        }
        Err(e) => return VerifyOutcome::reject(format!("簽發登記查詢失敗（fail closed）：{e}")),
    };
    let token = rec["executor_token"].as_str().unwrap_or_default();
    let param_hash = rec["param_hash"].as_str().unwrap_or_default();
    if issued.question_id != question_id
        || issued.executor_token != token
        || issued.param_hash != param_hash
        || (issued.answered_at_epoch as f64 - answered_at).abs() >= 1.0
    {
        return VerifyOutcome::reject(
            "record 欄位與簽發登記不一致（question_id/token/param_hash/answered_at 任一不符），拒絕",
        );
    }

    // 題目狀態閘：確為 yellow 錢題、且答案來自 ui 通道（物理點擊）。
    match handle.get_question(&question_id).await {
        Ok(Some(q)) => {
            if !q.is_money_op() {
                return VerifyOutcome::reject(format!("題目 {question_id} 非錢題，不走 approval 通道"));
            }
            if q.gate == Some(kunsu_core::types::Gate::Red) {
                return VerifyOutcome::reject(format!("題目 {question_id} 是紅燈題，永不進一鍵通道"));
            }
            match &q.answer {
                Some(a) if a.channel.is_ray_device() => {}
                Some(a) => {
                    return VerifyOutcome::reject(format!(
                        "題目 {question_id} 的答案來自 channel={:?}，不是使用者在 Kunsu app 或已配對手機上的點擊，拒絕",
                        a.channel
                    ));
                }
                None => {
                    return VerifyOutcome::reject(format!("題目 {question_id} 尚未作答"));
                }
            }
        }
        Ok(None) => {
            return VerifyOutcome::reject(format!("題目 {question_id} 不存在於 app 活體 DB"));
        }
        Err(e) => return VerifyOutcome::reject(format!("題目查詢失敗（fail closed）：{e}")),
    }

    match handle.consume_approval(&question_id, &nonce, record, now).await {
        Ok(ApprovalOutcome::Consumed) => {
            VerifyOutcome { valid: true, consumed_now: true, reason: None }
        }
        Ok(ApprovalOutcome::AlreadyConsumed) => {
            VerifyOutcome::reject("單次消耗：此 approval record 已被使用過，重放無效")
        }
        Ok(ApprovalOutcome::QuestionMissing) => {
            VerifyOutcome::reject(format!("題目 {question_id} 不存在於 app 活體 DB"))
        }
        // INV-9（2026-10-09）：Kunsu 已判定結束或被取代的授權永遠花不掉。
        Ok(ApprovalOutcome::AlreadySettled) => VerifyOutcome::reject(
            "這次授權已有結果（Kunsu 已判定結束，例如被擋下），不能再使用——請在 Kunsu 回執重新授權",
        ),
        Ok(ApprovalOutcome::Superseded) => {
            VerifyOutcome::reject("這次授權已被更新的授權取代（使用者已重新授權），舊的授權不能再使用")
        }
        Err(e) => VerifyOutcome::reject(format!("消耗記錄寫入失敗（fail closed）：{e}")),
    }
}

// ---------------------------------------------------------------------------
// 簽章鏈：prepare → token → spec → record → 落檔
// ---------------------------------------------------------------------------

/// 極簡 shell-words 切分（單雙引號 + 反斜線跳脫）。prepare 命令不經 shell，
/// 避免 zsh alias / profile 汙染（與 reactor spawn 同原則）。
pub fn split_command(s: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut chars = s.chars().peekable();
    let mut in_single = false;
    let mut in_double = false;
    let mut has_token = false;
    while let Some(c) = chars.next() {
        match c {
            '\'' if !in_double => {
                in_single = !in_single;
                has_token = true;
            }
            '"' if !in_single => {
                in_double = !in_double;
                has_token = true;
            }
            '\\' if !in_single => {
                let Some(n) = chars.next() else {
                    return Err("命令字串以未完成的反斜線結尾".to_string());
                };
                cur.push(n);
                has_token = true;
            }
            c if c.is_whitespace() && !in_single && !in_double => {
                if has_token {
                    out.push(std::mem::take(&mut cur));
                    has_token = false;
                }
            }
            c => {
                cur.push(c);
                has_token = true;
            }
        }
    }
    if in_single || in_double {
        return Err("命令字串引號未閉合".to_string());
    }
    if has_token {
        out.push(cur);
    }
    if out.is_empty() {
        return Err("命令字串為空".to_string());
    }
    Ok(out)
}

/// executor prepare stdout 的 token 行解析：找含 `commit --token <tok>` 的行。
/// 回 (整行 trim 後, token)。
pub fn parse_commit_line(stdout: &str) -> Option<(String, String)> {
    for line in stdout.lines() {
        if let Some(idx) = line.find("commit --token") {
            let rest = &line[idx..];
            let mut words = rest.split_whitespace();
            let _commit = words.next()?; // "commit"
            let _flag = words.next()?; // "--token"
            let tok = words.next()?.to_string();
            if !tok.is_empty() {
                return Some((line.trim().to_string(), tok));
            }
        }
    }
    None
}

/// 簽章鏈產出：approval 檔路徑 + server 自組的 commit argv + 簽發登記欄位。
#[derive(Debug, Clone)]
pub struct ChainOutcome {
    pub executor_token: String,
    pub param_hash: String,
    pub approval_path: PathBuf,
    /// argv 形式（不經 shell）；[0]=program。**全部由 server 寫死組裝**，
    /// prepare stdout 只提供 token——絕不讓子程序輸出改寫要執行的程式。
    pub commit_argv: Vec<String>,
    /// record 的 nonce（簽發登記用）。
    pub nonce: String,
    /// record 的 answered_at（epoch 秒；簽發登記用）。
    pub answered_at_epoch: i64,
}

/// prepare / commit 的直譯器與 script 路徑（server 寫死；INVARIANTS 第 17 條）。
pub const PYTHON_BIN: &str = "python3";
pub const EXECUTOR_SCRIPT_REL: &str = "projects/meta-ads/scripts/executor.py";

fn pending_dir(vault_root: &Path) -> PathBuf {
    vault_root.join("projects/meta-ads/data/executor_pending")
}

/// executor token 白名單：ASCII `[A-Za-z0-9_-]`、長度 1..=128（token 會成為
/// pending spec 檔名，杜絕路徑逃逸）。
fn valid_token(t: &str) -> bool {
    !t.is_empty()
        && t.len() <= 128
        && t.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn spec_field_str(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn spec_field_num(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    }
}

/// INVARIANTS 第 18 條 `inv_what_you_see_is_what_you_sign`：簽章前逐欄比對
/// prepare 產出的 spec 與卡片結構化欄位。任一不符 → Err（呼叫端
/// execution_blocked + Discord，絕不簽章）。
pub fn verify_spec_matches_card(spec: &Value, q: &Question) -> Result<(), String> {
    let fail = |why: String| {
        Err(format!("inv_what_you_see_is_what_you_sign：{why}——spec 與卡片不一致，不簽章"))
    };
    let money = match q.money.as_ref() {
        Some(m) => m,
        None => return fail("題目缺 money 結構".into()),
    };
    // entity
    let Some(card_entity) = money.entity_id.as_deref() else {
        return fail("卡片缺 money.entity_id，無從比對 entity".into());
    };
    let Some(spec_entity) = spec.get("entity").and_then(spec_field_str) else {
        return fail("spec 缺 entity".into());
    };
    if spec_entity != card_entity {
        return fail(format!("spec.entity={spec_entity:?} ≠ 卡片 entity_id={card_entity:?}"));
    }
    // 目標值：spec 的 new_budget / new_value vs（編輯後）params_line 解析出的目標值
    let target_key = match spec.get("action").and_then(Value::as_str) {
        Some("update_budget") => "new_budget",
        Some("update_field") => "new_value",
        other => return fail(format!("spec.action 不認得：{other:?}")),
    };
    let Some(spec_target) = spec.get(target_key).and_then(spec_field_num) else {
        return fail(format!("spec 缺可比對的 {target_key}"));
    };
    let Some(params_line) = q.params_line.as_deref() else {
        return fail("卡片缺 params_line".into());
    };
    let Some(card_target) = kunsu_core::redline::parse_money_change(params_line).map(|c| c.to)
    else {
        return fail(format!("params_line 解析不出目標值：{params_line:?}"));
    };
    if (spec_target - card_target).abs() > 1e-9 {
        return fail(format!(
            "spec.{target_key}={spec_target} ≠ 卡片 params_line 目標值 {card_target}"
        ));
    }
    // product
    let Some(card_product) = q.product.as_deref() else {
        return fail("卡片缺 product，無從比對".into());
    };
    let Some(spec_product) = spec.get("product").and_then(spec_field_str) else {
        return fail("spec 缺 product".into());
    };
    if spec_product != card_product {
        return fail(format!("spec.product={spec_product:?} ≠ 卡片 product={card_product:?}"));
    }
    Ok(())
}

/// 這題的 approval 檔固定路徑（`<vault>/projects/meta-ads/data/executor_pending/approvals/<question_id>.json`）。
pub fn approval_file_path(vault_root: &Path, question_id: &str) -> PathBuf {
    pending_dir(vault_root).join("approvals").join(format!("{question_id}.json"))
}

/// 被擋的授權刪掉 approval 檔（CONTRACT §7，best effort）：檔案不在就算了，刪不掉只記 log——真正的保證是 verify 拒絕已有結果的授權。
pub fn remove_approval_file(path: &Path) {
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => tracing::warn!("被擋的 approval 檔刪不掉（{}）：{e}；verify 仍會拒絕這次授權", path.display()),
    }
}

/// approval record 原子落檔：暫存檔 0600 建立 → rename（同目錄原子替換）。
fn write_approval_file(
    approvals_dir: &Path,
    question_id: &str,
    record: &Map<String, Value>,
) -> Result<PathBuf, String> {
    std::fs::create_dir_all(approvals_dir)
        .map_err(|e| format!("approvals 目錄建立失敗：{e}"))?;
    let approval_path = approvals_dir.join(format!("{question_id}.json"));
    let tmp = approvals_dir.join(format!(".{question_id}.{}.tmp", uuid::Uuid::new_v4()));
    {
        use std::io::Write;
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts
            .open(&tmp)
            .map_err(|e| format!("approval 暫存檔建立失敗（{}）：{e}", tmp.display()))?;
        f.write_all(
            serde_json::to_string_pretty(&Value::Object(record.clone()))
                .expect("record 恆可序列化")
                .as_bytes(),
        )
        .map_err(|e| format!("approval record 寫入失敗：{e}"))?;
        f.sync_all().map_err(|e| format!("approval record fsync 失敗：{e}"))?;
    }
    std::fs::rename(&tmp, &approval_path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("approval record rename 失敗：{e}")
    })?;
    Ok(approval_path)
}

/// 錢黃燈題被批准當下的完整簽章鏈（BDD money-approval 場景 2）。
/// 任一步失敗回人話 Err——呼叫端（reactor）負責 execution_blocked + 事件，絕不靜默。
pub async fn run_approval_chain(
    vault_root: &Path,
    q: &Question,
    key: &dyn KeyProvider,
) -> Result<ChainOutcome, String> {
    let prepare_cmd = q
        .money
        .as_ref()
        .and_then(|m| m.executor_prepare_cmd.as_deref())
        .ok_or_else(|| "題目缺 money.executor_prepare_cmd".to_string())?;
    let answer = q.answer.as_ref().ok_or_else(|| "題目尚未作答".to_string())?;
    if !kunsu_core::validate::valid_id(&q.id) {
        return Err(format!("question_id {:?} 不符路徑安全白名單，拒絕進簽章鏈", q.id));
    }

    // 1) INVARIANTS 第 17 條 inv_prepare_cmd_whitelist：spawn 前強制解析白名單。
    //    命令字串只是「結構化操作的宣告」——argv 由 server 用固定 program 與
    //    固定 script 路徑組裝，字串裡的路徑一律忽略，且以 argv 陣列 spawn 絕不過 shell。
    let op = kunsu_core::prepare_cmd::parse_prepare_cmd(prepare_cmd)
        .map_err(|e| format!("inv_prepare_cmd_whitelist：prepare 命令不在白名單（{e}）"))?;
    let script = vault_root.join(EXECUTOR_SCRIPT_REL);
    let mut prepare_args: Vec<String> = vec![script.to_string_lossy().to_string()];
    prepare_args.extend(op.argv_tail());

    // 2) 點批准當下才 prepare（token 新鮮期從「週」壓到「秒」）。cwd = vault 根。
    // 沒有輸入的 spawn 一律 stdin=/dev/null：不繼承 Kunsu 自己的 stdin（`kunsu serve` 從終端機啟動時不把終端機交給 prepare）。
    let output = tokio::process::Command::new(PYTHON_BIN)
        .args(&prepare_args)
        .current_dir(vault_root)
        .stdin(std::process::Stdio::null())
        .output()
        .await
        .map_err(|e| format!("prepare spawn 失敗（{PYTHON_BIN}）：{e}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    if !output.status.success() {
        return Err(format!(
            "prepare 非零退出（{}）：{}",
            output.status,
            tail(&scrub_token_args(&format!("{stdout}\n{stderr}")), 400)
        ));
    }

    // 3) 解析 token（stdout 只取 token，其餘一概不信）。guard 升紅 / BLOCK 時
    //    executor 不印 token → 這裡擋下。
    let Some((_commit_line, token)) = parse_commit_line(&stdout) else {
        return Err(format!(
            "prepare 輸出解析不到 commit token（guard 升紅或 BLOCK？）輸出尾：{}",
            tail(&scrub_token_args(&stdout), 400)
        ));
    };
    if !valid_token(&token) {
        return Err("executor token 不符白名單（ASCII [A-Za-z0-9_-]≤128）".to_string());
    }
    // 之後任何錯誤訊息都會進 execution_result、Discord 與 /events：先把 token 遮掉（AC-CC-14）。
    sign_with_token(vault_root, q, answer, key, &script, token.clone()).map_err(|e| e.replace(token.as_str(), TOKEN_MASK))
}

/// 錯誤訊息裡代替 executor token 的字樣。
const TOKEN_MASK: &str = "‹executor token›";
/// 錯誤訊息裡代替 approval 檔路徑的字樣。
const APPROVAL_PATH_MASK: &str = "‹approval 檔›";

/// 錢鏈錯誤訊息去敏：遮掉 executor token、approval 檔路徑（完整路徑與檔名）與 `--token <值>`。
pub fn mask_chain_secrets(text: &str, token: &str, approval_path: &Path) -> String {
    let mut out = text.to_string();
    let full = approval_path.to_string_lossy();
    if !full.is_empty() {
        out = out.replace(full.as_ref(), APPROVAL_PATH_MASK);
    }
    if let Some(name) = approval_path.file_name().map(|n| n.to_string_lossy().to_string())
        && !name.is_empty()
    {
        out = out.replace(&name, APPROVAL_PATH_MASK);
    }
    if !token.is_empty() {
        out = out.replace(token, TOKEN_MASK);
    }
    scrub_token_args(&out)
}

/// 把文字裡 `--token <值>` 的值遮掉（prepare 輸出尾會進錯誤訊息）。
fn scrub_token_args(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find("--token") {
        let (head, tail_part) = rest.split_at(i + "--token".len());
        out.push_str(head);
        let trimmed = tail_part.trim_start_matches([' ', '=', '\t']);
        out.push_str(&tail_part[..tail_part.len() - trimmed.len()]);
        let end = trimmed.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-')).unwrap_or(trimmed.len());
        if end > 0 {
            out.push_str(TOKEN_MASK);
        }
        rest = &trimmed[end..];
    }
    out.push_str(rest);
    out
}

/// prepare 拿到 token 之後的步驟：讀 pending spec、所見即所簽、簽章、落 approval 檔、組 commit argv。
fn sign_with_token(
    vault_root: &Path,
    q: &Question,
    answer: &kunsu_core::types::Answer,
    key: &dyn KeyProvider,
    script: &Path,
    token: String,
) -> Result<ChainOutcome, String> {

    // 4) 讀 pending spec 檔（canonicalize 圍欄：必須落在固定 pending 目錄內）。
    let pending = pending_dir(vault_root);
    let spec_path = pending.join(format!("{token}.json"));
    let canon_spec = spec_path
        .canonicalize()
        .map_err(|e| format!("pending spec 路徑解析失敗（{}）：{e}", spec_path.display()))?;
    let canon_pending = pending
        .canonicalize()
        .map_err(|e| format!("pending 目錄解析失敗（{}）：{e}", pending.display()))?;
    if !canon_spec.starts_with(&canon_pending) {
        return Err(format!(
            "pending spec 路徑逃出固定目錄（{}）——拒絕",
            canon_spec.display()
        ));
    }
    let spec_raw = std::fs::read_to_string(&canon_spec)
        .map_err(|e| format!("pending spec 讀取失敗（{}）：{e}", canon_spec.display()))?;
    let spec: Value = serde_json::from_str(&spec_raw)
        .map_err(|e| format!("pending spec 非法 JSON：{e}"))?;
    if let Some(tier) = spec.get("tier").and_then(Value::as_str)
        && tier != "yellow"
    {
        return Err(format!("pending spec tier={tier}：非 yellow 不走一鍵通道"));
    }

    // 5) INVARIANTS 第 18 條：所見即所簽逐欄比對（entity / 目標值 / product）。
    verify_spec_matches_card(&spec, q)?;
    let param_hash = normalized_param_hash(&spec);

    // 6) 組 record + HMAC 簽章（approval 子鍵）。answered_at 用「短效授權時間」：
    //    重新授權過的題以 reauthorized_at 為準（決策不變、授權更新），
    //    否則用 answered_at。epoch 秒（executor 兩種格式皆收）。
    let authorized_at = answer.reauthorized_at.unwrap_or(answer.answered_at);
    let answered_at_epoch = authorized_at.timestamp();
    let nonce = uuid::Uuid::new_v4().to_string();
    let hmac_key = key.hmac_key().map_err(|e| format!("HMAC key 取得失敗：{e}"))?;
    let mut record = Map::new();
    record.insert("question_id".into(), json!(q.id));
    record.insert("executor_token".into(), json!(token));
    record.insert("param_hash".into(), json!(param_hash));
    record.insert("answered_at".into(), json!(answered_at_epoch));
    record.insert("nonce".into(), json!(nonce));
    sign_record(&mut record, &hmac_key)?;

    // 7) 原子落檔 approvals/<question_id>.json（0600）。
    let approval_path = write_approval_file(&pending.join("approvals"), &q.id, &record)?;

    // 8) commit argv 由 server 寫死組裝（program/script/子命令固定；
    //    stdout 印什麼一概不信，只用第 3 步驗過白名單的 token）。
    let commit_argv = vec![
        PYTHON_BIN.to_string(),
        script.to_string_lossy().to_string(),
        "commit".to_string(),
        "--token".to_string(),
        token.clone(),
        "--approval-file".to_string(),
        approval_path.to_string_lossy().to_string(),
    ];

    Ok(ChainOutcome {
        executor_token: token,
        param_hash,
        approval_path,
        commit_argv,
        nonce,
        answered_at_epoch,
    })
}

fn tail(s: &str, n: usize) -> String {
    let t = s.trim();
    let chars: Vec<char> = t.chars().collect();
    if chars.len() <= n {
        t.to_string()
    } else {
        chars[chars.len() - n..].iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 三組 golden hash 由 executor.py normalized_param_hash（Python 原文：json.dumps sort_keys、無空白、ensure_ascii=False → sha256）算出。
    /// 演算法必須逐字元一致——這條測試是跨語言契約。
    #[test]
    fn param_hash_matches_python_reference() {
        let spec1 = json!({
            "action": "update_budget", "entity": "120000000000005", "product": "course-a",
            "new_budget": 600, "current_budget": "500", "tier": "yellow", "guards": [],
            "_token": "x", "prepared_at": 1
        });
        assert_eq!(
            normalized_param_hash(&spec1),
            "131fac962b10b320a2c3acfcf1b1ce58399702083c105a9c06ca931785744a95"
        );
        let spec2 = json!({
            "action": "update_field", "entity": "e-9", "field": "bid_amount",
            "new_value": "990", "product": "nova", "current": null,
            "tier": "yellow", "guards": ["中文備註"]
        });
        assert_eq!(
            normalized_param_hash(&spec2),
            "21b4d522c491a74cb4ac3759c8f51ffb461603dce27b65ee711cfa65dcd6e333"
        );
        let spec3 = json!({
            "action": "pause", "entity": "e-1", "note": "測試",
            "_token": "z", "prepared_at": 5, "flag": true
        });
        assert_eq!(
            normalized_param_hash(&spec3),
            "52568f4d2ae784dd05588e43eaf6ece0f698a0ddb8cb09fb0d70ab5293b779c9"
        );
    }

    #[test]
    fn split_command_handles_quotes() {
        assert_eq!(
            split_command(r#"python3 scripts/executor.py prepare-budget --entity "a b" --note 'c d' e\ f"#)
                .unwrap(),
            vec!["python3", "scripts/executor.py", "prepare-budget", "--entity", "a b", "--note", "c d", "e f"]
        );
        assert!(split_command("unterminated 'quote").is_err());
        assert!(split_command("   ").is_err());
    }

    #[test]
    fn parse_commit_line_finds_token() {
        let out = "===\n  ✅ 要核准請使用者在自己的 Terminal 貼上：\n     python executor.py commit --token abc123def456\n===";
        let (line, tok) = parse_commit_line(out).unwrap();
        assert_eq!(tok, "abc123def456");
        assert_eq!(line, "python executor.py commit --token abc123def456");
        assert!(parse_commit_line("🔴 RED：需完整人工複核").is_none());
    }

    /// P0：HMAC domain separation——UI 答案簽章與 approval record 簽章用
    /// 不同子鍵 + 長度編碼，拼接偽造在兩層都被杜絕。
    #[test]
    fn hmac_domain_separation_blocks_concat_forgery() {
        let master = b"test-key-32-bytes-test-key-32-by".to_vec();

        // 1) 子鍵彼此不同、也不等於 master。
        let k_approval = derive_subkey(&master, DOMAIN_APPROVAL_RECORD).unwrap();
        let k_answer = derive_subkey(&master, DOMAIN_UI_ANSWER).unwrap();
        assert_ne!(k_approval, k_answer);
        assert_ne!(k_approval, master);
        assert_ne!(k_answer, master);

        // 2) 拼接攻擊：舊制 `qid|cid|params` 用 master 直接 MAC——攻擊者可把
        //    canonical approval JSON 切三段丟進答案簽章取得合法 record MAC。
        //    新制下：用 master 對 canonical 直接算的 MAC 不再等於 record.hmac。
        let mut rec = Map::new();
        rec.insert("question_id".into(), json!("pa-1"));
        rec.insert("executor_token".into(), json!("tok"));
        rec.insert("param_hash".into(), json!("aa"));
        rec.insert("answered_at".into(), json!(1_753_900_000i64));
        rec.insert("nonce".into(), json!("n-1"));
        sign_record(&mut rec, &master).unwrap();
        let canonical = canonical_record(&rec);
        let mut legacy = HmacSha256::new_from_slice(&master).unwrap();
        legacy.update(canonical.as_bytes());
        let legacy_mac = hex::encode(legacy.finalize().into_bytes());
        assert_ne!(
            rec["hmac"].as_str().unwrap(),
            legacy_mac,
            "record MAC 不得等於 master key 對 canonical 的裸 MAC（否則答案面可偽造 record）"
        );
        // 把裸 MAC 塞進 record → 驗章必敗。
        let mut forged = rec.clone();
        forged.insert("hmac".into(), json!(legacy_mac));
        assert!(verify_signature(&forged, &master).is_err());
        // UI 答案簽章（就算把整段 canonical 塞進 params）也不會等於 record MAC。
        let ui = ui_answer_signature(&master, "", "", &canonical).unwrap();
        let mut forged2 = rec.clone();
        forged2.insert("hmac".into(), json!(ui));
        assert!(verify_signature(&forged2, &master).is_err());

        // 3) 長度編碼：欄位邊界挪移必產生不同簽章（無拼接歧義）。
        assert_ne!(
            ui_answer_signature(&master, "ab", "c", "d").unwrap(),
            ui_answer_signature(&master, "a", "bc", "d").unwrap()
        );
        assert_ne!(
            ui_answer_signature(&master, "a", "b", "cd").unwrap(),
            ui_answer_signature(&master, "a", "bc", "d").unwrap()
        );
    }

    /// INV-18 純函數面：spec 與卡片逐欄比對。
    #[test]
    fn spec_card_comparison_catches_every_field() {
        use kunsu_core::types::*;
        let mut req = kunsu_core::types::CreateRequest {
            id: "pa-wysiwys".into(),
            title: "t".into(),
            question_line: "q".into(),
            body_md: String::new(),
            why_md: None,
            visual_symbol: None,
            kind: Kind::Choice,
            tier: Tier::T1,
            product: Some("nova".into()),
            gate: Some(Gate::Yellow),
            evidence: vec![],
            params_line: Some("120 日預算 500 → 600".into()),
            options: vec![
                Opt { id: "a".into(), label: "go".into(), detail: None, recommended: Some(true) },
                Opt { id: "b".into(), label: "no".into(), detail: None, recommended: None },
            ],
            allow_free_text: true,
            money: Some(Money {
                is_money_op: true,
                amount_twd: Some(600),
                entity_id: Some("120".into()),
                executor_prepare_cmd: None,
            }),
            decision_ref: None,
            prediction_ref: Some("p.md".into()),
            kongko_ref: None,
            deep_link: None,
            irreversible: false,
            standing_policy_ref: None,
            internal: false,
            default_action: DefaultAction { on_expire: OnExpire::NoopLog, option_id: None },
            deadline_at: None,
            wake_on_answer: Wake::default(),
            source: Source {
                agent: AgentKind::Claude,
                session_id: None,
                cwd: "/x".into(),
                routine_id: None,
                ..Default::default()
            },
            first_proposed: None,
            items: vec![],
        };
        req.params_line = Some("120 日預算 500 → 600".into());
        let q = Question::from_create(req, chrono::Utc::now());

        let good = json!({
            "action": "update_budget", "entity": "120", "product": "nova",
            "new_budget": 600, "current_budget": 500, "tier": "yellow"
        });
        assert!(verify_spec_matches_card(&good, &q).is_ok());
        // entity 不符
        let mut bad = good.clone();
        bad["entity"] = json!("999");
        assert!(verify_spec_matches_card(&bad, &q).unwrap_err().contains("entity"));
        // 目標值不符（顯示批 600、實簽 900）
        let mut bad = good.clone();
        bad["new_budget"] = json!(900);
        assert!(verify_spec_matches_card(&bad, &q).is_err());
        // product 不符
        let mut bad = good.clone();
        bad["product"] = json!("course-a");
        assert!(verify_spec_matches_card(&bad, &q).unwrap_err().contains("product"));
        // spec 數字以字串出現也要能比對（executor spec 常見）
        let mut str_num = good.clone();
        str_num["new_budget"] = json!("600");
        assert!(verify_spec_matches_card(&str_num, &q).is_ok());
    }

    #[test]
    fn sign_then_tamper_fails_verification() {
        let key = b"test-key-32-bytes-test-key-32-by".to_vec();
        let mut rec = Map::new();
        rec.insert("question_id".into(), json!("pa-1"));
        rec.insert("executor_token".into(), json!("tok"));
        rec.insert("param_hash".into(), json!("aa"));
        rec.insert("answered_at".into(), json!(1_753_900_000i64));
        rec.insert("nonce".into(), json!("n-1"));
        sign_record(&mut rec, &key).unwrap();
        assert!(verify_signature(&rec, &key).is_ok());
        let mut tampered = rec.clone();
        tampered.insert("param_hash".into(), json!("bb"));
        assert!(verify_signature(&tampered, &key).is_err());
        // 型別竄改（數字 → 字串）也必敗
        let mut retyped = rec.clone();
        retyped.insert("answered_at".into(), json!("1753900000"));
        assert!(verify_signature(&retyped, &key).is_err());
    }
}
