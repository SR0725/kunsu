//! 紅線重檢（CONTRACT §9 P2 / BDD answer-flow 場景 3）。
//!
//! `ui/src/redline.ts` 的 Rust 鏡像——**行為必須與 UI 版逐案例一致**：
//! - 與 executor.py 護欄同值：變更額 > NT$2,000 或偏離 > 20% → 升 red 鎖一鍵。
//! - 邊界值（恰 2,000 / 恰 20%）不算超線，**嚴格大於**才升紅（P2 定案）。
//! - 純函數零 I/O；server 的 `edit_option` 與 Tauri IPC 共用。
//!
//! 真正的最後防線永遠在 executor.py（NT$2,000 紅線、≤20% 偏離、獨立重檢）；
//! 本模組只負責「所見即所簽」的視覺升紅與一鍵封鎖語義。

use serde::Serialize;

/// 與 executor.py 同值的紅線常數。
pub const REDLINE_CHANGE_TWD: f64 = 2000.0;
pub const REDLINE_DEVIATION: f64 = 0.2;

/// 從一行文字解析出的「X → Y」金額變更（含 match 起訖，重寫用）。
#[derive(Debug, Clone, PartialEq)]
pub struct MoneyChange {
    pub from: f64,
    pub to: f64,
    /// match 起點（byte offset，含可選 NT$ 前綴）。
    pub start: usize,
    /// match 終點（byte offset，to 數字之後）。
    pub end: usize,
}

/// 紅線重檢結果。serde 欄位名對齊 `ui/src/redline.ts` 的 `RedlineResult`
/// （camelCase——UI 直接吃這個 shape）。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RedlineResult {
    /// true = 超紅線：視覺升 red、鎖一鍵（只能走使用者終端通道）。
    pub breached: bool,
    pub change_abs: f64,
    /// from=0 時無法算偏離，回 None（UI 收 null）。
    pub deviation_pct: Option<f64>,
    pub reasons: Vec<String>,
}

fn is_num_char(c: char) -> bool {
    c.is_ascii_digit() || c == ','
}

fn parse_num(s: &str) -> Option<f64> {
    let cleaned: String = s.chars().filter(|c| *c != ',').collect();
    if cleaned.is_empty() {
        return None;
    }
    cleaned.parse::<f64>().ok()
}

/// 千分位格式（`Number.toLocaleString('en-US')` 的整數行為鏡像）。
pub fn fmt_thousands(n: f64) -> String {
    let rounded = n.round() as i64;
    let neg = rounded < 0;
    let digits = rounded.abs().to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    if neg {
        format!("-{out}")
    } else {
        out
    }
}

/// 掃描 `(?:NT$\s*)?([\d,]+)\s*(?:→|->)\s*(?:NT$\s*)?([\d,]+)`（ts 版同款）。
/// 解析不到（如 ACTIVE → PAUSED）回 None。
pub fn parse_money_change(line: &str) -> Option<MoneyChange> {
    // 逐個 arrow 嘗試（第一個能湊出兩側數字的 arrow 勝出，等價 regex 最左匹配）。
    let mut search_from = 0usize;
    loop {
        let rest = &line[search_from..];
        let arrow = ["→", "->"]
            .iter()
            .filter_map(|a| rest.find(a).map(|i| (search_from + i, a.len())))
            .min_by_key(|(i, _)| *i)?;
        let (arrow_at, arrow_len) = arrow;

        if let Some(mc) = try_match_at(line, arrow_at, arrow_len) {
            return Some(mc);
        }
        search_from = arrow_at + arrow_len;
        if search_from >= line.len() {
            return None;
        }
    }
}

fn try_match_at(line: &str, arrow_at: usize, arrow_len: usize) -> Option<MoneyChange> {
    // 往回：可選空白 → 數字串（至少一個 digit）→ 可選（NT$ + 空白）前綴。
    let before = &line[..arrow_at];
    let trimmed = before.trim_end();
    let ws_trail = before.len() - trimmed.len();
    let num_start_rel = trimmed
        .char_indices()
        .rev()
        .take_while(|(_, c)| is_num_char(*c))
        .last()
        .map(|(i, _)| i)?;
    let from_str = &trimmed[num_start_rel..];
    if !from_str.chars().any(|c| c.is_ascii_digit()) {
        return None;
    }
    let from = parse_num(from_str)?;
    let _ = ws_trail;

    // NT$ 前綴（可帶空白）併入 match 起點。
    let mut start = num_start_rel;
    let prefix = trimmed[..num_start_rel].trim_end();
    if prefix.ends_with("NT$") {
        start = prefix.len() - 3;
    }

    // 往前：空白 → 可選 NT$ → 空白 → 數字串。
    let after = &line[arrow_at + arrow_len..];
    let mut idx = after.len() - after.trim_start().len();
    let rest = &after[idx..];
    if let Some(stripped) = rest.strip_prefix("NT$") {
        idx += 3 + (stripped.len() - stripped.trim_start().len());
    }
    let to_str: String = after[idx..].chars().take_while(|c| is_num_char(*c)).collect();
    if !to_str.chars().any(|c| c.is_ascii_digit()) {
        return None;
    }
    let to = parse_num(&to_str)?;
    let end = arrow_at + arrow_len + idx + to_str.len();
    Some(MoneyChange { from, to, start, end })
}

/// 鉛筆編輯後重算整行（所見即所簽）：替換「→ 後的目標值」，
/// 並同步重算行內既有的（±N%）標註。ts `recalcLine` 鏡像。
pub fn recalc_line(line: &str, new_to: f64) -> String {
    let Some(c) = parse_money_change(line) else {
        return line.to_string();
    };
    let matched = &line[c.start..c.end];
    // 把 matched 內「arrow 之後的數字串」換成 fmt(new_to)。
    let rewritten = rewrite_target(matched, new_to);
    let mut out = format!("{}{}{}", &line[..c.start], rewritten, &line[c.end..]);

    // 重算既有百分比標註（（+20%）/ (-20%) 兩種括號都認）。
    if c.from > 0.0
        && let Some((ps, pe)) = find_pct_annotation(&out)
    {
        let pct = (new_to - c.from) / c.from * 100.0;
        let sign = if pct > 0.0 {
            "+"
        } else if pct < 0.0 {
            "-"
        } else {
            ""
        };
        let rounded = (pct.abs() * 10.0).round() / 10.0;
        let num = if rounded.fract() == 0.0 {
            format!("{}", rounded as i64)
        } else {
            format!("{rounded}")
        };
        out = format!("{}（{sign}{num}%）{}", &out[..ps], &out[pe..]);
    }
    out
}

fn rewrite_target(matched: &str, new_to: f64) -> String {
    let arrow = ["→", "->"]
        .iter()
        .filter_map(|a| matched.find(a).map(|i| (i, a.len())))
        .min_by_key(|(i, _)| *i);
    let Some((ai, alen)) = arrow else {
        return matched.to_string();
    };
    let after = &matched[ai + alen..];
    let mut idx = after.len() - after.trim_start().len();
    let rest = &after[idx..];
    if let Some(stripped) = rest.strip_prefix("NT$") {
        idx += 3 + (stripped.len() - stripped.trim_start().len());
    }
    let tail_start = idx + after[idx..].chars().take_while(|c| is_num_char(*c)).count();
    format!(
        "{}{}{}{}",
        &matched[..ai + alen],
        &after[..idx],
        fmt_thousands(new_to),
        &after[tail_start..]
    )
}

/// `[（(]\s*[+\-−]?\d+(\.\d+)?\s*%\s*[)）]` 的手工掃描；回 byte 起訖。
fn find_pct_annotation(s: &str) -> Option<(usize, usize)> {
    let chars: Vec<(usize, char)> = s.char_indices().collect();
    for (vi, &(bi, c)) in chars.iter().enumerate() {
        if c != '（' && c != '(' {
            continue;
        }
        let mut j = vi + 1;
        let skip_ws = |j: &mut usize| {
            while *j < chars.len() && chars[*j].1.is_whitespace() {
                *j += 1;
            }
        };
        skip_ws(&mut j);
        if j < chars.len() && matches!(chars[j].1, '+' | '-' | '−') {
            j += 1;
        }
        let digits_start = j;
        while j < chars.len() && chars[j].1.is_ascii_digit() {
            j += 1;
        }
        if j == digits_start {
            continue;
        }
        if j < chars.len() && chars[j].1 == '.' {
            let k = j + 1;
            let mut m = k;
            while m < chars.len() && chars[m].1.is_ascii_digit() {
                m += 1;
            }
            if m > k {
                j = m;
            }
        }
        skip_ws(&mut j);
        if j >= chars.len() || chars[j].1 != '%' {
            continue;
        }
        j += 1;
        skip_ws(&mut j);
        if j < chars.len() && (chars[j].1 == ')' || chars[j].1 == '）') {
            let end = chars[j].0 + chars[j].1.len_utf8();
            return Some((bi, end));
        }
    }
    None
}

/// 紅線重檢：邊界值（恰 2,000 / 恰 20%）不算超線，嚴格大於才升紅。
pub fn check_redline(from: f64, to: f64) -> RedlineResult {
    let change_abs = (to - from).abs();
    let deviation_pct = if from > 0.0 { Some(change_abs / from) } else { None };
    let mut reasons = Vec::new();
    if change_abs > REDLINE_CHANGE_TWD {
        reasons.push(format!(
            "變更額 NT${} 超過紅線 NT${}",
            fmt_thousands(change_abs),
            fmt_thousands(REDLINE_CHANGE_TWD)
        ));
    }
    if let Some(d) = deviation_pct
        && d > REDLINE_DEVIATION
    {
        let pct = (d * 1000.0).round() / 10.0;
        let num = if pct.fract() == 0.0 {
            format!("{}", pct as i64)
        } else {
            format!("{pct}")
        };
        reasons.push(format!("偏離 {num}% 超過 20% 上限"));
    }
    RedlineResult { breached: !reasons.is_empty(), change_abs, deviation_pct, reasons }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_and_ntd_prefixed_changes() {
        let c = parse_money_change("日預算 300 → 240（-20%）").unwrap();
        assert_eq!((c.from, c.to), (300.0, 240.0));
        let c = parse_money_change("NT$1,900 -> NT$500").unwrap();
        assert_eq!((c.from, c.to), (1900.0, 500.0));
        assert!(parse_money_change("ACTIVE → PAUSED").is_none());
        assert!(parse_money_change("沒有箭頭").is_none());
    }

    #[test]
    fn recalc_rewrites_target_and_pct() {
        assert_eq!(recalc_line("日預算 300 → 240（-20%）", 360.0), "日預算 300 → 360（+20%）");
        // ts 版 replace 只換括號本體、保留前導空白——鏡像同行為
        assert_eq!(recalc_line("300 -> 240 (-20%)", 300.0), "300 -> 300 （0%）");
        assert_eq!(recalc_line("NT$1,900 → NT$500", 1200.0), "NT$1,900 → NT$1,200");
        // 解析不到 → 原樣回傳
        assert_eq!(recalc_line("ACTIVE → PAUSED", 100.0), "ACTIVE → PAUSED");
    }

    #[test]
    fn redline_strictly_greater_boundary() {
        // 恰 2,000 / 恰 20% 不升紅（P2：嚴格大於才升）
        assert!(!check_redline(10000.0, 12000.0).breached);
        assert!(check_redline(10000.0, 12001.0).breached);
        assert!(!check_redline(300.0, 360.0).breached); // 恰 +20%
        assert!(check_redline(300.0, 361.0).breached); // > 20%
        // from=0 → 無偏離值，只看變更額
        let r = check_redline(0.0, 1500.0);
        assert!(!r.breached);
        assert_eq!(r.deviation_pct, None);
    }

    #[test]
    fn fmt_thousands_groups() {
        assert_eq!(fmt_thousands(1234567.0), "1,234,567");
        assert_eq!(fmt_thousands(500.0), "500");
        assert_eq!(fmt_thousands(2000.0), "2,000");
    }
}
