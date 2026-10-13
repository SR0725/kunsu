//! CONTRACT §10 policy.toml 解析 + 時區轉換函數測試。

mod common;

use chrono::NaiveTime;
use common::*;
use kunsu_core::policy::Hhmm;
use kunsu_core::*;

fn hm(h: u32, m: u32) -> NaiveTime {
    NaiveTime::from_hms_opt(h, m, 0).unwrap()
}

/// 空檔案 → 全預設值（§10 表列值）。
#[test]
fn empty_toml_yields_contract_defaults() {
    let p = Policy::from_toml_str("").unwrap();
    assert_eq!(p.force.cycle_min, 15);
    assert_eq!(p.force.warn_before_min, 3);
    assert_eq!(p.force.deferred_retry_s, 1);
    assert_eq!(p.windows.t1_batch, vec![Hhmm::new(9, 45), Hhmm::new(21, 30)]);
    assert_eq!(p.windows.quiet_hours, [Hhmm::new(23, 0), Hhmm::new(9, 0)]);
    assert_eq!(p.expiry.t1_default_after_h, 48);
    assert_eq!(p.expiry.t2_supersede_after_d, 14);
    assert_eq!(p.quota.per_source_daily_create, 12);
    assert_eq!(p.fatigue.skip_rate_window_d, 7);
    assert_eq!(p.fatigue.skip_rate_downgrade, 0.5);
    assert_eq!(p.discord.webhook, "");
    assert_eq!(p, Policy::default());
}

/// repo 實際的 config/policy.toml 必須永遠解析得動（drift 檢查）。
#[test]
fn repo_policy_toml_parses() {
    let raw = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../config/policy.toml"
    ))
    .expect("repo config/policy.toml 必須存在");
    let p = Policy::from_toml_str(&raw).expect("repo policy.toml 必須可解析");
    // repo 檔目前應與契約預設一致
    assert_eq!(p, Policy::default());
}

/// 部分覆寫：只給的鍵生效，其餘落回預設。
#[test]
fn partial_override_keeps_defaults() {
    let p = Policy::from_toml_str(
        r#"
[force]
cycle_min = 20

[windows]
quiet_hours = ["22:00", "08:30"]
"#,
    )
    .unwrap();
    assert_eq!(p.force.cycle_min, 20);
    assert_eq!(p.force.warn_before_min, 3, "沒給的鍵用預設");
    assert_eq!(p.windows.quiet_hours, [Hhmm::new(22, 0), Hhmm::new(8, 30)]);
    assert_eq!(p.windows.t1_batch, Policy::default().windows.t1_batch);
}

/// 壞時窗字串與未知鍵都要大聲失敗（調參打錯字不准無聲吞）。
#[test]
fn bad_values_fail_loudly() {
    assert!(Policy::from_toml_str("[windows]\nquiet_hours = [\"23：00\", \"09:00\"]").is_err());
    assert!(Policy::from_toml_str("[windows]\nquiet_hours = [\"25:00\", \"09:00\"]").is_err());
    assert!(Policy::from_toml_str("[force]\ncycle_minn = 15").is_err(), "未知鍵拒收");
    assert!(Policy::from_toml_str("[force]\ncycle_min = -1").is_err());
    assert!(Policy::from_toml_str("[force]\ncycle_min = 2\nwarn_before_min = 3").is_err());
}

/// quiet hours 跨午夜語義（台北本地時間解釋）。
#[test]
fn quiet_hours_cross_midnight_taipei() {
    let p = Policy::default();
    // 台北 23:30 = 15:30Z → quiet
    assert!(p.in_quiet_hours(ts("2026-07-31T15:30:00Z")));
    // 台北 03:00 = 前一天 19:00Z → quiet
    assert!(p.in_quiet_hours(ts("2026-07-30T19:00:00Z")));
    // 台北 08:59 → quiet；09:00 整點 → 出關
    assert!(p.in_quiet_hours(ts("2026-07-31T00:59:00Z")));
    assert!(!p.in_quiet_hours(ts("2026-07-31T01:00:00Z")));
    // 台北 14:00 → 白天
    assert!(!p.in_quiet_hours(now()));
    // 台北 23:00 整點進入 quiet
    assert!(p.in_quiet_hours(ts("2026-07-31T15:00:00Z")));
}

/// T1 批次窗：after 之後最近一個台北 09:45 / 21:30。
#[test]
fn next_t1_batch_window() {
    let p = Policy::default();
    // 台北 14:00 → 下一窗是今天 21:30（= 13:30Z）
    assert_eq!(p.next_t1_batch(now()).unwrap(), ts("2026-07-31T13:30:00Z"));
    // 台北 22:00（14:00Z）→ 下一窗是明天 09:45（= 01:45Z）
    assert_eq!(
        p.next_t1_batch(ts("2026-07-31T14:00:00Z")).unwrap(),
        ts("2026-08-01T01:45:00Z")
    );
    // 正好在窗上（台北 21:30）→ 取「之後」的下一窗，不含當下
    assert_eq!(
        p.next_t1_batch(ts("2026-07-31T13:30:00Z")).unwrap(),
        ts("2026-08-01T01:45:00Z")
    );
}

/// 集中式時區函數：UTC 存、台北解釋。
#[test]
fn taipei_conversions() {
    use kunsu_core::time;
    // 2026-07-31T18:00Z = 台北 8/1 02:00 → 日期要進位
    let t = ts("2026-07-31T18:00:00Z");
    assert_eq!(time::taipei_date(t).to_string(), "2026-08-01");
    assert_eq!(time::taipei_time(t), hm(2, 0));
    // 窗口不跨午夜
    assert!(time::in_taipei_window(now(), hm(9, 0), hm(18, 0)));
    assert!(!time::in_taipei_window(now(), hm(15, 0), hm(18, 0)));
}

/// BDD edge-notch 場景 10c：到期被延後時瀏海說出原因——依顯示優先序列出所有命中的 guard；
/// 勿擾帶結束時刻，安靜時段帶下一個結束時刻（台北 09:00）。
#[test]
fn defer_reasons_follow_display_priority() {
    use kunsu_core::guards::{DeferReason, GuardState};
    let p = Policy::default();
    let day = now(); // 台北 14:00，非安靜時段
    let none = GuardState::default().defer_reasons(day, &p);
    assert!(none.reasons.is_empty() && none.dnd_until.is_none() && none.quiet_until.is_none());

    let until = day + chrono::Duration::minutes(40);
    let all = GuardState {
        screen_shared: true,
        camera_on: true,
        mic_on: true,
        frontmost_fullscreen: true,
        quiet_hours: false,
        dnd: false,
        kill_switch_until: Some(until),
        forced_today_count: 0,
    }
    .defer_reasons(day, &p);
    assert_eq!(
        all.reasons,
        vec![DeferReason::ScreenShared, DeferReason::CameraOn, DeferReason::MicOn, DeferReason::FrontmostFullscreen, DeferReason::Dnd]
    );
    assert_eq!(all.dnd_until, Some(until));

    // 勿擾已過期不算；系統專注模式旗標也歸「勿擾」。
    let expired = GuardState { kill_switch_until: Some(day - chrono::Duration::minutes(1)), ..Default::default() };
    assert!(expired.defer_reasons(day, &p).reasons.is_empty());
    let focus = GuardState { dnd: true, ..Default::default() }.defer_reasons(day, &p);
    assert_eq!(focus.reasons, vec![DeferReason::Dnd]);

    // 台北 23:30：安靜時段（policy 判斷，快照旗標沒設也算），結束在隔天台北 09:00 = 01:00Z。
    let night = ts("2026-07-31T15:30:00Z");
    let q = GuardState::default().defer_reasons(night, &p);
    assert_eq!(q.reasons, vec![DeferReason::QuietHours]);
    assert_eq!(q.quiet_until, Some(ts("2026-08-01T01:00:00Z")));
    assert_eq!(serde_json::to_value(&q.reasons).unwrap(), serde_json::json!(["quiet_hours"]));
}
