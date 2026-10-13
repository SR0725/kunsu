//! `money.executor_prepare_cmd` 白名單解析（INVARIANTS 第 17 條
//! `inv_prepare_cmd_whitelist`）與數值引數改寫（CONTRACT §6 規則 7 /
//! INVARIANTS 第 16 條 `inv_edited_params_rewrite_prepare_cmd`）。
//!
//! 編輯（edit_option / edited_params）改動金額時，server 必須同步改寫
//! prepare 命令的目標數字，讓「點擊當下 prepare 產的 spec 即編輯後值」，
//! param_hash 自然綁到使用者看到的數字（所見即所簽全鏈成立）。
//!
//! 只認兩型（白名單，認不得＝呼叫端升 red 鎖一鍵）：
//! - `prepare-budget` 的 `--new-budget N`
//! - `prepare-bid` 的 `--new-value N`
//!
//! N 必須是純整數 token（前置分隔符為空白或 `=`）；新值必須是非負整數。
//! 純函數零 I/O；任何不確定一律 Err——錢的方向永遠 fail closed。

// ---------------------------------------------------------------------------
// 白名單解析（INVARIANTS 第 17 條 inv_prepare_cmd_whitelist）
// ---------------------------------------------------------------------------

/// 白名單允許的兩種 prepare 操作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrepareKind {
    Budget,
    Bid,
}

/// `executor_prepare_cmd` 白名單解析後的結構化操作。
/// server spawn 時**只用這個結構**組 argv（script 路徑由 server 寫死、
/// 字串裡的路徑一律忽略）——命令字串本身永不直接進 spawn。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrepareOp {
    pub kind: PrepareKind,
    /// `--entity`：純數字（Meta entity id）。
    pub entity: String,
    /// `--new-budget`（budget）/ `--new-value`（bid）：非負整數。
    pub target: i64,
    /// `prepare-bid` 可選 `--field`（`[a-z_]`）。
    pub field: Option<String>,
    /// 可選 `--product`（`[A-Za-z0-9_-]`）。
    pub product: Option<String>,
}

impl PrepareOp {
    pub fn subcommand(&self) -> &'static str {
        match self.kind {
            PrepareKind::Budget => "prepare-budget",
            PrepareKind::Bid => "prepare-bid",
        }
    }

    /// server 組 argv 的尾段（subcommand 起）；program 與 script 路徑由 server 提供。
    pub fn argv_tail(&self) -> Vec<String> {
        let mut out = vec![
            self.subcommand().to_string(),
            "--entity".to_string(),
            self.entity.clone(),
        ];
        match self.kind {
            PrepareKind::Budget => {
                out.push("--new-budget".to_string());
                out.push(self.target.to_string());
            }
            PrepareKind::Bid => {
                out.push("--new-value".to_string());
                out.push(self.target.to_string());
            }
        }
        if let Some(f) = &self.field {
            out.push("--field".to_string());
            out.push(f.clone());
        }
        if let Some(p) = &self.product {
            out.push("--product".to_string());
            out.push(p.clone());
        }
        out
    }
}

const MAX_CMD_LEN: usize = 512;
const MAX_ENTITY_LEN: usize = 32;
const MAX_NAME_LEN: usize = 40;

fn parse_digits_i64(s: &str, what: &str) -> Result<i64, String> {
    if s.is_empty() || s.len() > 15 || !s.chars().all(|c| c.is_ascii_digit()) {
        return Err(format!("{what} 必須是純數字整數（收到 {s:?}）"));
    }
    s.parse::<i64>().map_err(|e| format!("{what} 整數解析失敗：{e}"))
}

/// `executor_prepare_cmd` 白名單解析（inv_prepare_cmd_whitelist）。
///
/// 只認：`python3 <…>/executor.py prepare-budget|prepare-bid` + 白名單旗標
/// （`--entity`／`--new-budget`／`--new-value`／`--field`／`--product`）。
/// - 任何 shell 中繼字元（`;|&$\`(){}<>"'\\` 換行…）直接拒絕；
/// - 引數逐一驗格式（entity 純數字、金額非負整數、product/field 白名單字元）；
/// - 重複旗標、未知旗標、缺必要旗標一律拒絕。
///
/// 純函數零 I/O；認不得＝Err（錢的方向永遠 fail closed）。
pub fn parse_prepare_cmd(cmd: &str) -> Result<PrepareOp, String> {
    if cmd.len() > MAX_CMD_LEN {
        return Err(format!("prepare cmd 超過長度上限 {MAX_CMD_LEN}"));
    }
    // 字元白名單：先擋掉一切 shell 中繼字元／控制字元。
    if let Some(bad) = cmd
        .chars()
        .find(|c| !(c.is_ascii_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.' | '/' | '=')))
    {
        return Err(format!("prepare cmd 含非法字元 {bad:?}（禁止任何 shell 中繼字元）"));
    }

    // token 切分（白名單字元集內無引號語義，直接以空白切）＋ --flag=value 展開。
    let mut tokens: Vec<String> = Vec::new();
    for raw in cmd.split_whitespace() {
        if raw.starts_with("--") && raw.contains('=') {
            let (f, v) = raw.split_once('=').expect("contains('=') 已檢查");
            tokens.push(f.to_string());
            tokens.push(v.to_string());
        } else {
            tokens.push(raw.to_string());
        }
    }
    if tokens.len() < 3 {
        return Err("prepare cmd 不完整（需要 python3 + executor.py + 子命令）".to_string());
    }
    if tokens[0] != "python3" && !tokens[0].ends_with("/python3") {
        return Err(format!("命令必須以 python3 開頭（收到 {:?}）", tokens[0]));
    }
    let script_name = tokens[1].rsplit('/').next().unwrap_or("");
    if script_name != "executor.py" {
        return Err(format!("第二段必須是 executor.py（收到 {:?}；實際路徑由 server 寫死）", tokens[1]));
    }
    let kind = match tokens[2].as_str() {
        "prepare-budget" => PrepareKind::Budget,
        "prepare-bid" => PrepareKind::Bid,
        other => return Err(format!("子命令只認 prepare-budget / prepare-bid（收到 {other:?}）")),
    };

    let mut entity: Option<String> = None;
    let mut target: Option<i64> = None;
    let mut field: Option<String> = None;
    let mut product: Option<String> = None;
    let mut i = 3usize;
    while i < tokens.len() {
        let flag = tokens[i].as_str();
        let Some(value) = tokens.get(i + 1) else {
            return Err(format!("旗標 {flag} 缺值"));
        };
        let dup = |name: &str| format!("旗標 {name} 重複出現");
        match (flag, kind) {
            ("--entity", _) => {
                if entity.is_some() {
                    return Err(dup("--entity"));
                }
                if value.is_empty()
                    || value.len() > MAX_ENTITY_LEN
                    || !value.chars().all(|c| c.is_ascii_digit())
                {
                    return Err(format!("--entity 必須是純數字（≤{MAX_ENTITY_LEN} 位；收到 {value:?}）"));
                }
                entity = Some(value.clone());
            }
            ("--new-budget", PrepareKind::Budget) => {
                if target.is_some() {
                    return Err(dup("--new-budget"));
                }
                target = Some(parse_digits_i64(value, "--new-budget")?);
            }
            ("--new-value", PrepareKind::Bid) => {
                if target.is_some() {
                    return Err(dup("--new-value"));
                }
                target = Some(parse_digits_i64(value, "--new-value")?);
            }
            ("--field", PrepareKind::Bid) => {
                if field.is_some() {
                    return Err(dup("--field"));
                }
                if value.is_empty()
                    || value.len() > MAX_NAME_LEN
                    || !value.chars().all(|c| c.is_ascii_lowercase() || c == '_')
                {
                    return Err(format!("--field 只認 [a-z_]（收到 {value:?}）"));
                }
                field = Some(value.clone());
            }
            ("--product", _) => {
                if product.is_some() {
                    return Err(dup("--product"));
                }
                if value.is_empty()
                    || value.len() > MAX_NAME_LEN
                    || !value.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
                {
                    return Err(format!("--product 只認 [A-Za-z0-9_-]（收到 {value:?}）"));
                }
                product = Some(value.clone());
            }
            (other, _) => return Err(format!("旗標 {other:?} 不在白名單")),
        }
        i += 2;
    }

    let entity = entity.ok_or("缺必要旗標 --entity")?;
    let target = target.ok_or_else(|| match kind {
        PrepareKind::Budget => "缺必要旗標 --new-budget".to_string(),
        PrepareKind::Bid => "缺必要旗標 --new-value".to_string(),
    })?;
    Ok(PrepareOp { kind, entity, target, field, product })
}

/// 改寫 prepare cmd 的目標數值引數。成功回改寫後的完整命令字串；
/// 失敗回可讀原因（呼叫端據此升 red 鎖一鍵）。
pub fn rewrite_prepare_cmd(cmd: &str, new_value: f64) -> Result<String, String> {
    if !new_value.is_finite() || new_value < 0.0 || new_value.fract() != 0.0 || new_value >= 9.0e15
    {
        return Err(format!("新值 {new_value} 不是合法非負整數，拒絕改寫"));
    }
    let n = new_value as i64;

    let flag = if cmd.contains("prepare-budget") {
        "--new-budget"
    } else if cmd.contains("prepare-bid") {
        "--new-value"
    } else {
        return Err("prepare cmd 不是已知型式（只認 prepare-budget / prepare-bid）".to_string());
    };

    let hits: Vec<usize> = cmd.match_indices(flag).map(|(i, _)| i).collect();
    match hits.len() {
        0 => return Err(format!("prepare cmd 缺 {flag} 引數，無法改寫")),
        1 => {}
        k => return Err(format!("prepare cmd 內 {flag} 出現 {k} 次，無法唯一定位")),
    }

    let after_flag = hits[0] + flag.len();
    let rest = &cmd[after_flag..];

    // 分隔符：單一 `=` 或至少一個空白。
    let sep_len = if rest.starts_with('=') {
        1
    } else {
        let ws = rest.len() - rest.trim_start().len();
        if ws == 0 {
            return Err(format!("{flag} 後缺分隔符（空白或 =）"));
        }
        ws
    };
    let num_part = &rest[sep_len..];
    let digits_len = num_part.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits_len == 0 {
        return Err(format!("{flag} 後不是整數引數"));
    }
    // token 完整性：數字後必須是空白或字串結尾（150.5 / 150x 都拒絕）。
    let tail = &num_part[digits_len..];
    if let Some(c) = tail.chars().next()
        && !c.is_whitespace()
    {
        return Err(format!("{flag} 的引數不是純整數（數字後緊接 {c:?}）"));
    }

    Ok(format!("{}{}{}{}", &cmd[..after_flag], &rest[..sep_len], n, tail))
}

#[cfg(test)]
mod tests {
    use super::*;

    const BUDGET_CMD: &str =
        "python3 projects/meta-ads/scripts/executor.py prepare-budget --entity 120215 --new-budget 300 --product nova";
    const BID_CMD: &str =
        "python3 projects/meta-ads/scripts/executor.py prepare-bid --entity 120215 --new-value 45";

    #[test]
    fn rewrites_budget_and_bid_targets() {
        let out = rewrite_prepare_cmd(BUDGET_CMD, 150.0).unwrap();
        assert!(out.contains("--new-budget 150"), "{out}");
        assert!(out.ends_with("--product nova"), "數字後引數保留：{out}");

        let out = rewrite_prepare_cmd(BID_CMD, 60.0).unwrap();
        assert!(out.ends_with("--new-value 60"), "{out}");
    }

    #[test]
    fn rewrites_equals_separator() {
        let cmd = "executor prepare-budget --new-budget=300";
        assert_eq!(rewrite_prepare_cmd(cmd, 150.0).unwrap(), "executor prepare-budget --new-budget=150");
    }

    #[test]
    fn rejects_unknown_command_shape() {
        let err = rewrite_prepare_cmd("python3 weird.py --amount 300", 150.0).unwrap_err();
        assert!(err.contains("已知型式"), "{err}");
        // prepare-budget 存在但缺 flag → 也拒
        let err = rewrite_prepare_cmd("executor prepare-budget --amount 300", 150.0).unwrap_err();
        assert!(err.contains("--new-budget"), "{err}");
    }

    #[test]
    fn rejects_non_integer_values_and_tokens() {
        assert!(rewrite_prepare_cmd(BUDGET_CMD, 150.5).is_err(), "新值必須整數");
        assert!(rewrite_prepare_cmd(BUDGET_CMD, -1.0).is_err(), "新值必須非負");
        assert!(rewrite_prepare_cmd(BUDGET_CMD, f64::NAN).is_err());
        // 既有引數不是純整數 token → 拒
        let cmd = "executor prepare-budget --new-budget 300.5";
        assert!(rewrite_prepare_cmd(cmd, 150.0).is_err());
        let cmd = "executor prepare-budget --new-budget abc";
        assert!(rewrite_prepare_cmd(cmd, 150.0).is_err());
    }

    #[test]
    fn rejects_ambiguous_multiple_flags() {
        let cmd = "executor prepare-budget --new-budget 300 --new-budget 500";
        let err = rewrite_prepare_cmd(cmd, 150.0).unwrap_err();
        assert!(err.contains("出現 2 次"), "{err}");
    }

    // -----------------------------------------------------------------
    // inv_prepare_cmd_whitelist（INVARIANTS 第 17 條）純函數面
    // -----------------------------------------------------------------

    #[test]
    fn inv_prepare_cmd_whitelist_accepts_canonical_forms() {
        let op = parse_prepare_cmd(
            "python3 projects/meta-ads/scripts/executor.py prepare-budget --entity 120000000000005 --new-budget 600 --product nova",
        )
        .unwrap();
        assert_eq!(op.kind, PrepareKind::Budget);
        assert_eq!(op.entity, "120000000000005");
        assert_eq!(op.target, 600);
        assert_eq!(op.product.as_deref(), Some("nova"));
        assert_eq!(
            op.argv_tail(),
            vec![
                "prepare-budget",
                "--entity",
                "120000000000005",
                "--new-budget",
                "600",
                "--product",
                "nova"
            ]
        );

        // = 分隔、bid 型帶 --field 也接受
        let op = parse_prepare_cmd(
            "python3 executor.py prepare-bid --entity=99 --new-value=45 --field bid_amount",
        )
        .unwrap();
        assert_eq!(op.kind, PrepareKind::Bid);
        assert_eq!(op.target, 45);
        assert_eq!(op.field.as_deref(), Some("bid_amount"));
    }

    #[test]
    fn inv_prepare_cmd_whitelist_rejects_shell_metacharacters() {
        for evil in [
            "python3 executor.py prepare-budget --entity 1 --new-budget 2; rm -rf /",
            "python3 executor.py prepare-budget --entity 1 --new-budget 2 && rm -rf ~",
            "python3 executor.py prepare-budget --entity 1 --new-budget $(cat /etc/passwd)",
            "python3 executor.py prepare-budget --entity `id` --new-budget 2",
            "python3 executor.py prepare-budget --entity 1 --new-budget 2 | tee /tmp/x",
            "python3 executor.py prepare-budget --entity 1 --new-budget 2 > /tmp/x",
            "python3 executor.py prepare-budget --entity '1' --new-budget 2",
            "python3 executor.py prepare-budget --entity 1 --new-budget 2\nrm -rf /",
        ] {
            let err = parse_prepare_cmd(evil).unwrap_err();
            assert!(err.contains("非法字元"), "{evil:?} → {err}");
        }
    }

    #[test]
    fn inv_prepare_cmd_whitelist_rejects_wrong_program_script_and_flags() {
        // 換掉程式 / script
        assert!(parse_prepare_cmd("rm -rf /").is_err());
        assert!(parse_prepare_cmd("bash executor.py prepare-budget --entity 1 --new-budget 2").is_err());
        assert!(parse_prepare_cmd("python3 evil.py prepare-budget --entity 1 --new-budget 2").is_err());
        // 換子命令
        assert!(parse_prepare_cmd("python3 executor.py commit --token x").is_err());
        // 未知旗標 / 重複旗標 / 缺必要旗標
        assert!(
            parse_prepare_cmd("python3 executor.py prepare-budget --entity 1 --new-budget 2 --extra x")
                .is_err()
        );
        assert!(parse_prepare_cmd(
            "python3 executor.py prepare-budget --entity 1 --entity 2 --new-budget 3"
        )
        .is_err());
        assert!(parse_prepare_cmd("python3 executor.py prepare-budget --entity 1").is_err());
        assert!(parse_prepare_cmd("python3 executor.py prepare-budget --new-budget 2").is_err());
        // budget 型不認 --new-value；bid 型不認 --new-budget
        assert!(
            parse_prepare_cmd("python3 executor.py prepare-budget --entity 1 --new-value 2").is_err()
        );
        assert!(
            parse_prepare_cmd("python3 executor.py prepare-bid --entity 1 --new-budget 2").is_err()
        );
    }

    #[test]
    fn inv_prepare_cmd_whitelist_validates_argument_formats() {
        // entity 非純數字（含路徑逃逸企圖）
        for bad in ["abc", "12a", "../../etc", "1/2"] {
            let cmd =
                format!("python3 executor.py prepare-budget --entity {bad} --new-budget 2");
            assert!(parse_prepare_cmd(&cmd).is_err(), "entity {bad:?} 必拒");
        }
        // 金額非整數
        for bad in ["2.5", "-3", "abc", ""] {
            let cmd =
                format!("python3 executor.py prepare-budget --entity 1 --new-budget {bad}");
            assert!(parse_prepare_cmd(&cmd).is_err(), "budget {bad:?} 必拒");
        }
        // product 字元白名單
        assert!(parse_prepare_cmd(
            "python3 executor.py prepare-budget --entity 1 --new-budget 2 --product a/b"
        )
        .is_err());
        // field 只認小寫與底線
        assert!(parse_prepare_cmd(
            "python3 executor.py prepare-bid --entity 1 --new-value 2 --field Bid"
        )
        .is_err());
    }
}
