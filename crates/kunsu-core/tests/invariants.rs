//! INVARIANTS.md 中屬於 core 層的不變量，測試名照文件括號裡的。
//! （ledger append-only / MCP 無作答面 / approval 單次+TTL / rebuild 等
//! server 層不變量由 kunsu-server 的 integration test 覆蓋。）

mod common;

use chrono::Duration;
use common::*;
use kunsu_core::*;

/// INVARIANT 1：錢類題缺 prediction_ref 必被 reject。
#[test]
fn inv_money_requires_prediction() {
    let mut req = money_req();
    req.prediction_ref = None;
    assert_eq!(validate_create(&req), Err(RejectReason::MoneyRequiresPrediction));

    // 空白字串等同缺席
    req.prediction_ref = Some("   ".into());
    assert_eq!(validate_create(&req), Err(RejectReason::MoneyRequiresPrediction));

    // 合法錢類題（有 prediction_ref + params_line）通過
    assert_eq!(validate_create(&money_req()), Ok(()));

    // is_money_op=false 不受此限
    let mut req = base_req();
    req.money = Some(Money { is_money_op: false, amount_twd: None, entity_id: None, executor_prepare_cmd: None });
    assert_eq!(validate_create(&req), Ok(()));
}

/// INVARIANT 2：錢不因超時自動動。
/// 防線一：validate 拒收 money + on_expire=pick_option。
/// 防線二：就算資料層混進這種題，狀態機超時也絕不發 RunDefault(pick_option)。
#[test]
fn inv_money_never_auto_expire() {
    // 防線一
    let mut req = money_req();
    req.default_action = DefaultAction { on_expire: OnExpire::PickOption, option_id: Some("a".into()) };
    assert_eq!(validate_create(&req), Err(RejectReason::MoneyNeverAutoExpire));

    // 防線二：繞過 validate 直接造出髒資料
    let policy = Policy::default();
    let mut req = money_req();
    req.deadline_at = Some(now() + Duration::hours(1));
    let mut q = pending_question(req, now(), &policy);
    q.default_action = DefaultAction { on_expire: OnExpire::PickOption, option_id: Some("a".into()) };

    let (state, fx) = transition(&q, Event::Tick, now() + Duration::hours(1), &policy, &GuardState::default());
    assert_eq!(state, State::Expired, "錢類題可以過期歸檔，但不可自動執行");
    assert!(
        !fx.iter().any(|e| matches!(e, Effect::RunDefault { action }
            if action.on_expire == OnExpire::PickOption)),
        "絕不對錢類題發出 pick_option 執行效果：{fx:?}"
    );
    assert!(has_discord(&fx, DiscordReason::MoneyExpired), "錢類超時必須 Discord 留痕");

    // 錢類 T1 48h 預設超時同樣不自動動
    let mut req = money_req();
    req.tier = Tier::T1;
    let mut q = pending_question(req, now(), &policy);
    q.default_action = DefaultAction { on_expire: OnExpire::PickOption, option_id: Some("a".into()) };
    let (state, fx) = transition(&q, Event::Tick, now() + Duration::hours(48), &policy, &GuardState::default());
    assert_eq!(state, State::Expired);
    assert!(!fx.iter().any(|e| matches!(e, Effect::RunDefault { action }
        if action.on_expire == OnExpire::PickOption)));
}

/// INVARIANT 3：所有非 internal actionable question 都進 15 分鐘循環，tier/預算不排除。
#[test]
fn inv_force_cycle_eligible() {
    let policy = Policy::default();
    for tier in [Tier::T0, Tier::T1, Tier::T2, Tier::T3] {
        let mut req = base_req();
        req.tier = tier;
        let mut q = pending_question(req, now(), &policy);
        let warn_at = q.warn_at.expect("新題必須持久化 warn_at");
        let force_at = q.force_at.expect("新題必須持久化 force_at");
        assert_eq!(warn_at, now() + Duration::minutes(12));
        assert_eq!(force_at, now() + Duration::minutes(15));
        assert_eq!(q.cycle_number, 1);

        let (s, fx) = transition(&q, Event::Tick, warn_at, &policy, &GuardState::default());
        assert_eq!(s, State::Warned, "{tier:?} 必須進 warning");
        apply(&mut q, s, &fx);
        let (s, _) = transition(&q, Event::Tick, force_at, &policy, &GuardState::default());
        assert_eq!(s, State::Forced, "{tier:?} 必須可 forced");
    }
}

/// INVARIANT 4：dismiss 未回答介面會無上限重啟完整 15 分鐘週期。
#[test]
fn inv_dismiss_restarts_cycle() {
    let policy = Policy::default();
    let mut q = pending_question(base_req(), now(), &policy);
    for round in 1..=10u32 {
        let warn_at = q.warn_at.unwrap();
        let force_at = q.force_at.unwrap();
        let (s, fx) = transition(&q, Event::Tick, warn_at, &policy, &GuardState::default());
        apply(&mut q, s, &fx);
        let (s, fx) = transition(&q, Event::Tick, force_at, &policy, &GuardState::default());
        apply(&mut q, s, &fx);
        assert_eq!(q.state, State::Forced);

        let dismissed_at = force_at + Duration::seconds(1);
        let (s, fx) = transition(&q, Event::Dismiss, dismissed_at, &policy, &GuardState::default());
        apply(&mut q, s, &fx);
        assert_eq!(q.state, State::Pending);
        assert_eq!(q.cycle_number, round + 1);
        assert_eq!(q.cycle_started_at, Some(dismissed_at));
        assert_eq!(q.warn_at, Some(dismissed_at + Duration::minutes(12)));
        assert_eq!(q.force_at, Some(dismissed_at + Duration::minutes(15)));
        assert_eq!(q.tier, Tier::T0, "dismiss 不得降級");
    }
}

/// INVARIANT 5：guards 可讓 warning 發生，但 force 必須 deferred；解除後立即恢復。
#[test]
fn inv_guards_defer_force() {
    let policy = Policy::default();
    let blocked: Vec<(&str, GuardState)> = vec![
        ("screen_shared", GuardState { screen_shared: true, ..GuardState::default() }),
        ("camera_on", GuardState { camera_on: true, ..GuardState::default() }),
        ("mic_on", GuardState { mic_on: true, ..GuardState::default() }),
        ("frontmost_fullscreen", GuardState { frontmost_fullscreen: true, ..GuardState::default() }),
        ("quiet_hours", GuardState { quiet_hours: true, ..GuardState::default() }),
        ("dnd", GuardState { dnd: true, ..GuardState::default() }),
        ("kill_switch", GuardState { kill_switch_until: Some(now() + Duration::hours(4)), ..GuardState::default() }),
    ];

    for (name, guards) in blocked {
        let mut q = pending_question(base_req(), now(), &policy);
        let warn_at = q.warn_at.unwrap();
        let force_at = q.force_at.unwrap();
        let (s, fx) = transition(&q, Event::Tick, warn_at, &policy, &guards);
        assert_eq!(s, State::Warned, "guard [{name}] 不得擋 warning");
        apply(&mut q, s, &fx);

        let cycle = q.cycle_started_at;
        let (s, fx) = transition(&q, Event::Tick, force_at, &policy, &guards);
        assert_eq!(s, State::Warned, "guard [{name}] 必須 defer force");
        assert!(!has_notify(&fx, NotifyKind::ForcedOverlay));
        apply(&mut q, s, &fx);
        assert_eq!(q.cycle_started_at, cycle, "deferred 不重設週期");

        let (s, fx) = transition(
            &q,
            Event::Tick,
            force_at + Duration::seconds(1),
            &policy,
            &GuardState::default(),
        );
        assert_eq!(s, State::Forced, "guard [{name}] 解除後立即恢復");
        assert!(has_notify(&fx, NotifyKind::ForcedOverlay));
    }
}

/// INVARIANT 11：gate=red 的題必為多選項 choice。
/// （「永不進一鍵執行通道」的執行面由 server/UI 層測；core 擋題型。）
#[test]
fn inv_red_needs_options() {
    // red + confirm / text / kongko_review → reject
    for kind in [Kind::Confirm, Kind::Text, Kind::KongkoReview] {
        let mut req = base_req();
        req.gate = Some(Gate::Red);
        req.kind = kind;
        req.options = vec![];
        assert_eq!(
            validate_create(&req),
            Err(RejectReason::RedGateNeedsChoice),
            "red + {kind:?} 必須被拒"
        );
    }

    // red + choice 但選項不足 → 一樣拒（十秒可決 lint）
    let mut req = base_req();
    req.gate = Some(Gate::Red);
    req.options.truncate(1);
    let err = validate_create(&req).unwrap_err();
    assert_eq!(err, RejectReason::ChoiceOptionsOutOfRange { len: 1 });
    assert!(err.is_malformed());

    // red + choice 2..=4 選項 → 通過
    let mut req = base_req();
    req.gate = Some(Gate::Red);
    assert_eq!(validate_create(&req), Ok(()));
}

/// INVARIANT 14：任何題 pending 壽命 ≤ 14 天（T2 上限）；到期必轉終態。
#[test]
fn inv_max_pending_age() {
    let policy = Policy::default();
    let cap = Duration::days(policy.expiry.t2_supersede_after_d as i64);

    // T0（無 deadline、guards 永遠擋著蓋不了板的最壞情境）→ 14 天必 expired
    let q = pending_question(base_req(), now(), &policy);
    let always_blocked = GuardState { dnd: true, ..GuardState::default() };
    let (s, fx) = transition(&q, Event::Tick, now() + cap, &policy, &always_blocked);
    assert_eq!(s, State::Expired, "T0 卡滿 14 天必轉終態");
    assert!(has_ledger(&fx, TerminalKind::Expired));

    // T2 → 14 天 superseded
    let mut req = base_req();
    req.tier = Tier::T2;
    let q = pending_question(req, now(), &policy);
    let (s, _) = transition(&q, Event::Tick, now() + cap, &policy, &GuardState::default());
    assert_eq!(s, State::Superseded);

    // T1 在 48h 就走預設，遠早於 14 天上限
    let mut req = base_req();
    req.tier = Tier::T1;
    let q = pending_question(req, now(), &policy);
    let (s, _) = transition(&q, Event::Tick, now() + Duration::hours(48), &policy, &GuardState::default());
    assert!(s.is_terminal());

    // 14 天前的最後一刻還不強制轉終態
    let q = pending_question(base_req(), now(), &policy);
    let (s, _) = transition(&q, Event::Tick, now() + cap - Duration::minutes(1), &policy, &always_blocked);
    assert!(!s.is_terminal(), "14 天前不能提早收斂（got {s:?}）");

    // 就算狀態卡在 snoozed / warned，14 天一樣強制收斂
    let mut q = pending_question(base_req(), now(), &policy);
    q.state = State::Snoozed;
    q.snooze_count = 1;
    let (s, _) = transition(&q, Event::Tick, now() + cap, &policy, &always_blocked);
    assert!(s.is_terminal(), "snoozed 卡 14 天也必轉終態（got {s:?}）");
}

/// INVARIANT 8（core 層）：upsert 合併不動 first_proposed、不重置倒數。
/// 「不發新通知」由合併函數零 Effect 的形狀保證——server upsert 路徑只落 DB。
#[test]
fn inv_upsert_no_reset() {
    let policy = Policy::default();
    let mut q = pending_question(base_req(), now(), &policy);
    // 讓題目有「歷史」：已排程的倒數、已 snooze 過
    q.snooze_count = 1;
    q.forced_today_used = true;
    let original = q.clone();

    // 三天後同 id 再建題，帶新證據
    let later = now() + Duration::days(3);
    let mut incoming = base_req();
    incoming.evidence = vec![EvidenceLine { text: "ROAS 掉到 1.9".into(), source: "meta-api".into() }];

    let merged = apply_upsert(&q, &incoming, later);

    // 只准動的：last_seen、evidence
    assert_eq!(merged.last_seen, later);
    assert_eq!(merged.evidence, incoming.evidence);

    // 不准動的全清單
    assert_eq!(merged.first_proposed, original.first_proposed, "first_proposed 是資歷，upsert 不動");
    assert_eq!(merged.next_transition_at, original.next_transition_at, "不重置倒數");
    assert_eq!(merged.created_at, original.created_at);
    assert_eq!(merged.state, original.state);
    assert_eq!(merged.snooze_count, original.snooze_count);
    assert_eq!(merged.forced_today_used, original.forced_today_used);
    assert_eq!(merged.tier, original.tier);
    assert_eq!(merged.answer, original.answer);

    // 空證據的重複建題 → 保留舊證據
    let mut empty_ev = base_req();
    empty_ev.evidence = vec![];
    let merged2 = apply_upsert(&q, &empty_ev, later);
    assert_eq!(merged2.evidence, original.evidence);
}
