//! CONTRACT §3 轉移表——15 分鐘強制詢問週期的表驅動測試。

mod common;

use chrono::Duration;
use common::*;
use kunsu_core::*;

fn warned_question(policy: &Policy) -> Question {
    let mut q = pending_question(base_req(), now(), policy);
    let (state, effects) = transition(
        &q,
        Event::Tick,
        q.warn_at.expect("qualified question has warn_at"),
        policy,
        &GuardState::default(),
    );
    apply(&mut q, state, &effects);
    assert_eq!(q.state, State::Warned);
    q
}

fn forced_question(policy: &Policy) -> Question {
    let mut q = warned_question(policy);
    let (state, effects) = transition(
        &q,
        Event::Tick,
        q.force_at.expect("qualified question has force_at"),
        policy,
        &GuardState::default(),
    );
    apply(&mut q, state, &effects);
    assert_eq!(q.state, State::Forced);
    q
}

#[test]
fn created_enters_pending_and_keeps_the_persisted_cycle_schedule() {
    let policy = Policy::default();
    let mut q = Question::from_create(base_req(), now());
    q.start_force_cycle(now(), &policy, false);

    let (state, effects) =
        transition(&q, Event::Tick, now(), &policy, &GuardState::default());

    assert_eq!(state, State::Pending);
    assert!(has_notify(&effects, NotifyKind::NewQuestion));
    assert!(effects.contains(&Effect::ScheduleNext { at: now() + Duration::minutes(12) }));
}

#[test]
fn every_non_internal_tier_warns_at_twelve_minutes_and_forces_at_fifteen() {
    let policy = Policy::default();
    for tier in [Tier::T0, Tier::T1, Tier::T2, Tier::T3] {
        let mut req = base_req();
        req.tier = tier;
        let q = pending_question(req, now(), &policy);

        let (state, early_fx) = transition(
            &q,
            Event::Tick,
            now() + Duration::minutes(11),
            &policy,
            &GuardState::default(),
        );
        assert_eq!(state, State::Pending, "{tier:?} must not warn early");
        assert!(!has_notify(&early_fx, NotifyKind::WarnCountdown));

        let (state, warn_fx) = transition(
            &q,
            Event::Tick,
            now() + Duration::minutes(12),
            &policy,
            &GuardState::default(),
        );
        assert_eq!(state, State::Warned, "{tier:?} must warn");
        assert!(has_notify(&warn_fx, NotifyKind::WarnCountdown));

        let mut warned = q;
        apply(&mut warned, state, &warn_fx);
        let (state, force_fx) = transition(
            &warned,
            Event::Tick,
            now() + Duration::minutes(15),
            &policy,
            &GuardState::default(),
        );
        assert_eq!(state, State::Forced, "{tier:?} must force");
        assert!(has_notify(&force_fx, NotifyKind::ForcedOverlay));
    }
}

#[test]
fn internal_doctor_questions_never_enter_a_force_cycle() {
    let policy = Policy::default();
    let mut req = base_req();
    req.internal = true;
    let mut q = Question::from_create(req, now());
    q.start_force_cycle(now(), &policy, false);
    assert_eq!(q.cycle_started_at, None);
    assert_eq!(q.warn_at, None);
    assert_eq!(q.force_at, None);
    assert_eq!(q.cycle_number, 0);

    q.state = State::Pending;
    let (state, effects) = transition(
        &q,
        Event::Tick,
        now() + Duration::days(1),
        &policy,
        &GuardState::default(),
    );
    assert_eq!(state, State::Pending);
    assert!(!has_notify(&effects, NotifyKind::WarnCountdown));
    assert!(!has_notify(&effects, NotifyKind::ForcedOverlay));
}

#[test]
fn guards_defer_only_the_forced_overlay_and_do_not_reset_the_cycle() {
    let policy = Policy::default();
    let mut q = warned_question(&policy);
    let cycle = (q.cycle_started_at, q.warn_at, q.force_at, q.cycle_number);
    let guards = GuardState { screen_shared: true, ..GuardState::default() };

    let (state, blocked_fx) = transition(
        &q,
        Event::Tick,
        q.force_at.unwrap(),
        &policy,
        &guards,
    );
    assert_eq!(state, State::Warned);
    assert!(!has_notify(&blocked_fx, NotifyKind::ForcedOverlay));
    apply(&mut q, state, &blocked_fx);
    assert_eq!((q.cycle_started_at, q.warn_at, q.force_at, q.cycle_number), cycle);

    let (state, resumed_fx) = transition(
        &q,
        Event::Tick,
        q.force_at.unwrap() + Duration::seconds(policy.force.deferred_retry_s as i64),
        &policy,
        &GuardState::default(),
    );
    assert_eq!(state, State::Forced);
    assert!(has_notify(&resumed_fx, NotifyKind::ForcedOverlay));
}

#[test]
fn a_guard_appearing_while_forced_hides_overlay_but_keeps_overdue_cycle() {
    let policy = Policy::default();
    let q = forced_question(&policy);
    let guards = GuardState { camera_on: true, ..GuardState::default() };
    let (state, effects) = transition(
        &q,
        Event::Tick,
        q.force_at.unwrap() + Duration::seconds(1),
        &policy,
        &guards,
    );
    assert_eq!(state, State::Warned);
    assert!(effects.contains(&Effect::HideOverlay));
    assert!(!effects.iter().any(|effect| matches!(effect, Effect::RestartForceCycle { .. })));
}

#[test]
fn dismissing_an_unanswered_overlay_starts_a_fresh_full_cycle_without_a_cap() {
    let policy = Policy::default();
    let mut q = forced_question(&policy);
    for cycle in 2..=12 {
        let dismissed_at = now() + Duration::minutes(15 * cycle as i64);
        let (state, effects) = transition(
            &q,
            Event::Dismiss,
            dismissed_at,
            &policy,
            &GuardState::default(),
        );
        assert_eq!(state, State::Pending);
        assert!(effects.contains(&Effect::IncrementCycle));
        assert!(effects.contains(&Effect::RestartForceCycle {
            cycle_started_at: dismissed_at,
            warn_at: dismissed_at + Duration::minutes(12),
            force_at: dismissed_at + Duration::minutes(15),
        }));
        assert!(!effects.iter().any(|effect| matches!(effect, Effect::DowngradeTier { .. })));
        apply(&mut q, state, &effects);
        assert_eq!(q.cycle_number, cycle);
        assert_eq!(q.tier, Tier::T0);
    }
}

#[test]
fn legacy_snooze_event_is_a_compatibility_alias_for_dismiss() {
    let policy = Policy::default();
    let q = forced_question(&policy);
    let at = now() + Duration::minutes(16);
    let dismiss = transition(
        &q,
        Event::Dismiss,
        at,
        &policy,
        &GuardState::default(),
    );
    let snooze = transition(
        &q,
        Event::Snooze,
        at,
        &policy,
        &GuardState::default(),
    );
    assert_eq!(snooze, dismiss);
}

#[test]
fn answer_cancel_supersede_and_delivery_remain_terminal_and_audited() {
    let policy = Policy::default();
    let q = pending_question(base_req(), now(), &policy);
    let guards = GuardState::default();

    let (state, effects) = transition(
        &q,
        Event::Answered { channel: Channel::Ui },
        now(),
        &policy,
        &guards,
    );
    assert_eq!(state, State::Answered);
    assert!(has_ledger(&effects, TerminalKind::Answered));

    let (state, effects) = transition(&q, Event::Cancel, now(), &policy, &guards);
    assert_eq!(state, State::Cancelled);
    assert!(has_ledger(&effects, TerminalKind::Cancelled));

    let (state, effects) = transition(&q, Event::Supersede, now(), &policy, &guards);
    assert_eq!(state, State::Superseded);
    assert!(has_ledger(&effects, TerminalKind::Superseded));

    let mut answered = q;
    answered.state = State::Answered;
    assert_eq!(
        transition(&answered, Event::DeliveryComplete, now(), &policy, &guards),
        (State::Routed, vec![])
    );
}

#[test]
fn explicit_deadline_and_max_age_still_end_the_cycle() {
    let policy = Policy::default();
    let mut req = base_req();
    req.deadline_at = Some(now() + Duration::hours(2));
    let q = pending_question(req, now(), &policy);
    let (state, effects) = transition(
        &q,
        Event::Tick,
        now() + Duration::hours(2),
        &policy,
        &GuardState::default(),
    );
    assert_eq!(state, State::Expired);
    assert!(has_ledger(&effects, TerminalKind::Expired));

    let mut req = base_req();
    req.tier = Tier::T2;
    let q = pending_question(req, now(), &policy);
    let (state, effects) = transition(
        &q,
        Event::Tick,
        now() + Duration::days(policy.expiry.t2_supersede_after_d as i64),
        &policy,
        &GuardState::default(),
    );
    assert_eq!(state, State::Superseded);
    assert!(has_ledger(&effects, TerminalKind::Superseded));
}

#[test]
fn full_lifecycle_walkthrough() {
    let policy = Policy::default();
    let guards = GuardState::default();
    let mut q = Question::from_create(base_req(), now());
    q.start_force_cycle(now(), &policy, false);

    for (at, expected) in [
        (now(), State::Pending),
        (now() + Duration::minutes(12), State::Warned),
        (now() + Duration::minutes(15), State::Forced),
    ] {
        let (state, effects) = transition(&q, Event::Tick, at, &policy, &guards);
        apply(&mut q, state, &effects);
        assert_eq!(q.state, expected);
    }

    let dismissed_at = now() + Duration::minutes(16);
    let (state, effects) = transition(&q, Event::Dismiss, dismissed_at, &policy, &guards);
    apply(&mut q, state, &effects);
    assert_eq!(q.state, State::Pending);
    assert_eq!(q.cycle_number, 2);

    let (state, effects) = transition(
        &q,
        Event::Answered { channel: Channel::Ui },
        dismissed_at + Duration::minutes(1),
        &policy,
        &guards,
    );
    apply(&mut q, state, &effects);
    assert_eq!(q.state, State::Answered);
    assert!(has_ledger(&effects, TerminalKind::Answered));
}

/// AC-QL-11／AC-RC-09：收割只把已回答的題標成 routed；過期、取消、被取代保持原終態。
#[test]
fn delivery_complete_only_routes_answered_questions() {
    let policy = Policy::default();
    let guards = GuardState::default();
    let q = pending_question(base_req(), now(), &policy);
    for (state, expected) in [
        (State::Answered, State::Routed),
        (State::Routed, State::Routed),
        (State::Expired, State::Expired),
        (State::Cancelled, State::Cancelled),
        (State::Superseded, State::Superseded),
    ] {
        let mut t = q.clone();
        t.state = state;
        assert_eq!(transition(&t, Event::DeliveryComplete, now(), &policy, &guards), (expected, vec![]), "{state:?}");
    }
}

/// CONTRACT §7（QA continuity 第 9 輪）：錢題不論 wake 設定都進 reactor——使用者批准的錢鏈一定會跑；
/// 非錢題 wake=none 照舊不進 reactor。
#[test]
fn answered_money_spawns_the_reactor_even_without_wake() {
    let policy = Policy::default();
    let mut money = pending_question(money_req(), now(), &policy);
    money.wake_on_answer.mode = WakeMode::None;
    let (_, fx) = transition(&money, Event::Answered { channel: Channel::Ui }, now(), &policy, &GuardState::default());
    assert!(fx.contains(&Effect::SpawnReactor), "錢題 wake=none 也要進 reactor：{fx:?}");

    let mut plain = pending_question(base_req(), now(), &policy);
    plain.wake_on_answer.mode = WakeMode::None;
    let (_, fx) = transition(&plain, Event::Answered { channel: Channel::Ui }, now(), &policy, &GuardState::default());
    assert!(!fx.contains(&Effect::SpawnReactor), "非錢題 wake=none 不進 reactor：{fx:?}");
}

/// INV-26（2026-10-09）：到期參數在建題時固定在題目上（`expiry_after_h`／`max_pending_d`）。之後 policy 調低，舊題照自己的快照到期；
/// 沒有快照的舊資料才用目前 policy。快照用 JSON 寫入：舊資料就是這樣從 SQLite／帳本讀回來的。
#[test]
fn expiry_pinned_at_creation_wins_over_the_current_policy() {
    let created = Policy::default();
    let mut lowered = Policy::default();
    lowered.expiry.t1_default_after_h = 1;
    lowered.expiry.t2_supersede_after_d = 1;
    let pinned = |tier: Tier| -> Question {
        let mut req = base_req();
        req.tier = tier;
        let q = pending_question(req, now(), &created);
        let mut v = serde_json::to_value(&q).unwrap();
        v["expiry_after_h"] = serde_json::json!(48);
        v["max_pending_d"] = serde_json::json!(14);
        serde_json::from_value(v).unwrap()
    };
    let guards = GuardState { dnd: true, ..GuardState::default() };

    // 一般題：照快照 48 小時，不照目前的 1 小時。
    let q = pinned(Tier::T1);
    let (s, fx) = transition(&q, Event::Tick, now() + Duration::hours(2), &lowered, &guards);
    assert!(!s.is_terminal(), "快照 48 小時，第 2 小時不能到期（got {s:?}）");
    assert!(!fx.iter().any(|e| matches!(e, Effect::RunDefault { .. })), "不能跑預設動作");
    let (s, _) = transition(&q, Event::Tick, now() + Duration::hours(48), &lowered, &guards);
    assert_eq!(s, State::Expired);
    // 排程也照快照：下一個時限是建題時的 48 小時，不是目前 policy 的 1 小時（只看到期時限：週期時刻另計）。
    let mut quiet = q.clone();
    quiet.internal = true;
    assert_eq!(next_due(&quiet, now() + Duration::minutes(30), &lowered), now() + Duration::hours(48));

    // 策略題：照快照 14 天，不照目前的 1 天。
    let q = pinned(Tier::T2);
    let (s, _) = transition(&q, Event::Tick, now() + Duration::days(2), &lowered, &guards);
    assert!(!s.is_terminal(), "快照 14 天，第 2 天不能被取代（got {s:?}）");
    let (s, _) = transition(&q, Event::Tick, now() + Duration::days(14), &lowered, &guards);
    assert_eq!(s, State::Superseded);

    // 沒有快照的舊資料：沿用目前 policy。
    let mut req = base_req();
    req.tier = Tier::T1;
    let legacy = pending_question(req, now(), &created);
    let (s, _) = transition(&legacy, Event::Tick, now() + Duration::hours(2), &lowered, &guards);
    assert_eq!(s, State::Expired, "舊題沒有快照時照目前 policy");
}
