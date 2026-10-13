//! 題組（PRD §25；CONTRACT §14）的純函數：每項內容雜湊、第幾輪、逐項答案驗證與總數。
//!
//! 所見即所送：使用者確認的文字就是 agent 要送出的文字。每項雜湊綁定使用者看到的那一版內容；
//! 答案逐項記錄，依題目項目順序保存。

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::types::{BatchItem, ItemAction, ItemDecision, ItemDefault};

/// 一題題組的項目數上下限。
pub const MIN_ITEMS: usize = 1;
pub const MAX_ITEMS: usize = 20;
/// 可編輯草稿（建題的 proposal 與作答的最終文字）上限，以字元計。
pub const MAX_DRAFT_CHARS: usize = 5000;
/// 請 AI 改的指示與整體指示上限。
pub const MAX_NOTE_CHARS: usize = 2000;
/// 項目標題上限（同題目 title）。
pub const MAX_TITLE_CHARS: usize = 80;
/// 項目副標上限（字元）。
pub const MAX_SUBTITLE_CHARS: usize = 200;
/// 題組 id（不含 `-rN` 輪次後綴）上限：題目 id 上限 120 字，留 4 字給 `-r2`～`-r99`（AC-BR-06）。
pub const MAX_BATCH_BASE_ID_LEN: usize = crate::validate::MAX_ID_LEN - 4;

/// 雜湊涵蓋的欄位（固定順序；不含 content_hash 本身）。
#[derive(Serialize)]
struct HashView<'a> {
    id: &'a str,
    title: &'a str,
    subtitle: Option<&'a str>,
    context_md: &'a str,
    proposal: &'a str,
    editable: bool,
    default: ItemDefault,
    default_reason: Option<&'a str>,
}

/// 一項的內容雜湊：`sha256:<hex>`，任何使用者看得到的欄位改了都會變。
pub fn item_hash(item: &BatchItem) -> String {
    let view = HashView {
        id: &item.id,
        title: &item.title,
        subtitle: item.subtitle.as_deref(),
        context_md: &item.context_md,
        proposal: &item.proposal,
        editable: item.editable,
        default: item.default,
        default_reason: item.default_reason.as_deref(),
    };
    let bytes = serde_json::to_vec(&view).unwrap_or_default();
    format!("sha256:{}", hex::encode(Sha256::digest(&bytes)))
}

/// 重算每項雜湊（建題與草稿升版時；agent 帶的值一律覆寫）。
pub fn seal_items(items: Vec<BatchItem>) -> Vec<BatchItem> {
    items
        .into_iter()
        .map(|mut item| {
            item.content_hash = Some(item_hash(&item));
            item
        })
        .collect()
}

/// 題目 id 的第幾輪：`x` → (`x`, 1)；`x-r2` → (`x`, 2)。`-r1`、`-r0` 不算輪次。
pub fn round_of(id: &str) -> (&str, u32) {
    if let Some((base, n)) = id.rsplit_once("-r")
        && !base.is_empty()
        && !n.is_empty()
        && n.chars().all(|c| c.is_ascii_digit())
        && let Ok(round) = n.parse::<u32>()
        && round >= 2
    {
        return (base, round);
    }
    (id, 1)
}

/// 下一輪題組的 id：`x` → `x-r2`、`x-r2` → `x-r3`。
pub fn next_round_id(id: &str) -> String {
    let (base, round) = round_of(id);
    format!("{base}-r{}", round + 1)
}

/// 逐項決定的總數（採用含「使用者改過」）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BatchCounts {
    pub approve: usize,
    pub edited: usize,
    pub skip: usize,
    pub revise: usize,
}

/// 採用的可編輯項目，最終文字與 AI 的草稿不同＝使用者改過。
pub fn is_edited(item: &BatchItem, decision: &ItemDecision) -> bool {
    decision.action == ItemAction::Approve
        && item.editable
        && decision.text.as_deref().is_some_and(|t| t != item.proposal)
}

pub fn counts(items: &[BatchItem], decisions: &[ItemDecision]) -> BatchCounts {
    let mut c = BatchCounts::default();
    for d in decisions {
        match d.action {
            ItemAction::Approve => {
                c.approve += 1;
                if items.iter().find(|i| i.id == d.item_id).is_some_and(|i| is_edited(i, d)) {
                    c.edited += 1;
                }
            }
            ItemAction::Skip => c.skip += 1,
            ItemAction::Revise => c.revise += 1,
        }
    }
    c
}

/// 逐項答案驗證（CONTRACT §14）：每項恰好一次、文字與指示規則、雜湊綁定。
/// 通過時回傳依題目項目順序排好、每項帶當下雜湊的決定；不通過回傳給使用者／agent 看的原因。
/// `require_hash`＝面板作答（channel=ui）：每項都要帶面板顯示時的雜湊。
pub fn check_answer(items: &[BatchItem], decisions: Vec<ItemDecision>, require_hash: bool) -> Result<Vec<ItemDecision>, String> {
    let mut slots: Vec<Option<ItemDecision>> = vec![None; items.len()];
    for d in decisions {
        let Some(index) = items.iter().position(|i| i.id == d.item_id) else {
            let known: Vec<&str> = items.iter().map(|i| i.id.as_str()).collect();
            return Err(format!("item_id {:?} 不在這一題（這題的項目：{}）", d.item_id, known.join("、")));
        };
        if slots[index].is_some() {
            return Err(format!("item_id {:?}（第 {} 項）出現兩次：每個項目只能決定一次", d.item_id, index + 1));
        }
        slots[index] = Some(d);
    }
    let mut out = Vec::with_capacity(items.len());
    for (index, (item, slot)) in items.iter().zip(slots).enumerate() {
        let n = index + 1;
        let Some(mut d) = slot else {
            return Err(format!("第 {n} 項（{}）沒有決定：每個項目都要選採用、不處理或請 AI 改", item.id));
        };
        let current = item.content_hash.clone().unwrap_or_else(|| item_hash(item));
        match d.content_hash.as_deref() {
            None if require_hash => {
                return Err(format!("第 {n} 項沒有帶面板顯示的內容雜湊：請重新開啟面板再送出"));
            }
            Some(h) if h != current => {
                return Err(format!("第 {n} 項的內容已更新（你看的是舊版）：請重新開啟面板看過再送出"));
            }
            _ => {}
        }
        d.content_hash = Some(current);
        d.note = d.note.map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
        match d.action {
            ItemAction::Approve if item.editable => {
                // 可編輯項目採用時最終文字不能是空的（2026-10-09 作者；和面板同一句話）。
                let Some(text) = d.text.as_deref().filter(|t| !t.trim().is_empty()) else {
                    return Err(format!("第 {n} 項沒有內容，先寫好再採用"));
                };
                let len = text.chars().count();
                if len > MAX_DRAFT_CHARS {
                    return Err(format!("第 {n} 項的文字超過 5,000 字（{len} 字）"));
                }
            }
            ItemAction::Approve => {
                if d.text.is_some() {
                    return Err(format!("第 {n} 項（{}）只能照建議或不處理，不能帶文字", item.id));
                }
            }
            ItemAction::Skip => {
                if d.text.is_some() {
                    return Err(format!("第 {n} 項選了不處理，不能帶文字"));
                }
            }
            ItemAction::Revise => {
                let Some(note) = d.note.as_deref() else {
                    return Err(format!("第 {n} 項選了請 AI 改，要寫一句怎麼改"));
                };
                let len = note.chars().count();
                if len > MAX_NOTE_CHARS {
                    return Err(format!("第 {n} 項的指示（請 AI 改）超過 2,000 字（{len} 字）"));
                }
                if d.text.is_some() {
                    return Err(format!("第 {n} 項選了請 AI 改，不能帶文字（要送出的文字請改成採用）"));
                }
            }
        }
        out.push(d);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounds_follow_the_r_suffix() {
        assert_eq!(round_of("mail-1009"), ("mail-1009", 1));
        assert_eq!(round_of("mail-1009-r2"), ("mail-1009", 2));
        assert_eq!(round_of("mail-r10"), ("mail", 10));
        assert_eq!(round_of("mail-r1"), ("mail-r1", 1), "-r1 不是輪次");
        assert_eq!(round_of("-r2"), ("-r2", 1), "沒有原 id 就不是輪次");
        assert_eq!(round_of("mail-rx"), ("mail-rx", 1));
        assert_eq!(next_round_id("mail-1009"), "mail-1009-r2");
        assert_eq!(next_round_id("mail-1009-r2"), "mail-1009-r3");
    }
}
