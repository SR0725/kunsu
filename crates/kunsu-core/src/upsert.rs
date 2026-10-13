//! `ask_create` 同 id upsert 的純合併函數（CONTRACT §4 / inv_upsert_no_reset）。
//!
//! 合約：只 touch `last_seen` 與證據。**不**重置倒數（`next_transition_at`
//! / `created_at`）、**不**動 `first_proposed`、**不**動 state / snooze_count /
//! forced_today_used / answer。不發任何通知——本函數回傳值不含 Effect，
//! server 的 upsert 路徑拿到合併結果只准寫 DB，不准觸發通知或重排 timer。

use crate::time::Ts;
use crate::types::{CreateRequest, DraftRef, Question};

/// 合併重複建題到既有題目。回傳合併後的新 Question（不改輸入）。
pub fn apply_upsert(existing: &Question, incoming: &CreateRequest, now: Ts) -> Question {
    let mut merged = existing.clone();
    merged.last_seen = now;
    // 證據允許刷新（agent 可能帶來更新的數據快照）；空的就保留舊證據。
    if !incoming.evidence.is_empty() {
        merged.evidence = incoming.evidence.clone();
    }
    merged
}

/// 草稿重新註冊（PRD §24 / AC-EC-04）：換成新版本的內容，週期與排程不動。
///
/// 換：題目文字、why、內文、kind、選項、題組項目（重算每項雜湊）、證據、錢參數與參照、預設動作（新草稿已以新選項驗證過）。
/// 不動：狀態、週期時間戳與 cycle_number、first_proposed、created_at、tier、到期、來源、喚醒方式、答案。
pub fn apply_draft_update(existing: &Question, incoming: &CreateRequest, draft: DraftRef, now: Ts) -> Question {
    let mut q = existing.clone();
    q.title = incoming.title.clone();
    q.question_line = incoming.question_line.clone();
    q.body_md = incoming.body_md.clone();
    q.why_md = incoming.why_md.clone();
    q.visual_symbol = incoming.visual_symbol.clone();
    q.kind = incoming.kind;
    q.product = incoming.product.clone();
    q.gate = incoming.gate;
    q.evidence = incoming.evidence.clone();
    q.params_line = incoming.params_line.clone();
    q.options = incoming.options.clone();
    q.items = crate::batch::seal_items(incoming.items.clone());
    q.allow_free_text = incoming.allow_free_text;
    q.money = incoming.money.clone();
    q.decision_ref = incoming.decision_ref.clone();
    q.prediction_ref = incoming.prediction_ref.clone();
    q.kongko_ref = incoming.kongko_ref.clone();
    q.deep_link = incoming.deep_link.clone();
    q.irreversible = incoming.irreversible;
    q.standing_policy_ref = incoming.standing_policy_ref.clone();
    q.default_action = incoming.default_action.clone();
    q.last_seen = now;
    q.draft = Some(draft);
    q
}
