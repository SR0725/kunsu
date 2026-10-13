//! 測試共用建構器。時間錨點刻意選台北白天（14:00，quiet hours 之外），
//! 讓 guards 全綠時 policy quiet hours 不會誤擋。
#![allow(dead_code)] // 每個測試 binary 只用到子集，helper 齊備即可

use chrono::{DateTime, Utc};
use kunsu_core::*;

/// 2026-07-31T06:00:00Z = 台北 14:00（週五白天）。
pub const NOW_STR: &str = "2026-07-31T06:00:00Z";

pub fn ts(s: &str) -> Ts {
    DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
}

pub fn now() -> Ts {
    ts(NOW_STR)
}

pub fn base_req() -> CreateRequest {
    CreateRequest {
        id: "pa-test-0001".into(),
        title: "測試題".into(),
        question_line: "要不要加碼？".into(),
        body_md: "內文".into(),
        why_md: None,
        visual_symbol: None,
        kind: Kind::Choice,
        tier: Tier::T0,
        product: Some("nova".into()),
        gate: Some(Gate::Yellow),
        evidence: vec![EvidenceLine { text: "ROAS 3.2 連三日".into(), source: "meta-api".into() }],
        params_line: None,
        options: vec![
            Opt { id: "a".into(), label: "加碼 20%".into(), detail: None, recommended: Some(true) },
            Opt { id: "b".into(), label: "維持".into(), detail: None, recommended: None },
        ],
        allow_free_text: true,
        money: None,
        decision_ref: None,
        prediction_ref: None,
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
            cwd: "/Users/you/notes".into(),
            routine_id: None,
            ..Default::default()
        },
        first_proposed: None,
        items: vec![],
    }
}

/// 錢類題請求（通過 validate 的合法版本）。
pub fn money_req() -> CreateRequest {
    let mut req = base_req();
    req.id = "pa-money-0001".into();
    req.money = Some(Money {
        is_money_op: true,
        amount_twd: Some(600),
        entity_id: Some("120000000000005".into()),
        executor_prepare_cmd: Some(
            "python3 projects/meta-ads/scripts/executor.py prepare-budget --entity 120000000000005 --new-budget 600 --product nova"
                .into(),
        ),
    });
    req.prediction_ref = Some("projects/meta-ads/predictions/2026-07-31-test.md".into());
    req.params_line = Some("adset 120000000000005 日預算 500 → 600 (+20%)".into());
    req
}

/// 建一題並推進到 pending（走正規 created→pending 轉移）。
pub fn pending_question(req: CreateRequest, created_at: Ts, policy: &Policy) -> Question {
    let mut q = Question::from_create(req, created_at);
    q.start_force_cycle(created_at, policy, false);
    let guards = GuardState::default();
    let (s, fx) = transition(&q, Event::Tick, created_at, policy, &guards);
    assert_eq!(s, State::Pending, "created 第一個 tick 必進 pending");
    apply(&mut q, s, &fx);
    q
}

/// 把 transition 的回傳套回 Question（模擬 server 落 DB），供多步驟劇本測試。
pub fn apply(q: &mut Question, new_state: State, effects: &[Effect]) {
    q.state = new_state;
    for e in effects {
        match e {
            Effect::IncrementSnooze => q.snooze_count += 1,
            Effect::IncrementCycle => q.cycle_number += 1,
            Effect::RestartForceCycle { cycle_started_at, warn_at, force_at } => {
                q.cycle_started_at = Some(*cycle_started_at);
                q.warn_at = Some(*warn_at);
                q.force_at = Some(*force_at);
            }
            Effect::DowngradeTier { to } => q.tier = *to,
            Effect::MarkForcedBudgetUsed => q.forced_today_used = true,
            Effect::ScheduleNext { at } => q.next_transition_at = *at,
            _ => {}
        }
    }
}

pub fn has_notify(fx: &[Effect], kind: NotifyKind) -> bool {
    fx.iter().any(|e| matches!(e, Effect::Notify { kind: k } if *k == kind))
}

pub fn has_ledger(fx: &[Effect], terminal: TerminalKind) -> bool {
    fx.iter().any(|e| matches!(e, Effect::WriteLedger { terminal: t } if *t == terminal))
}

pub fn has_discord(fx: &[Effect], reason: DiscordReason) -> bool {
    fx.iter().any(|e| matches!(e, Effect::Discord { reason: r } if *r == reason))
}
