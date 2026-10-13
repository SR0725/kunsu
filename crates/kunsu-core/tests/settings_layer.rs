//! 設定疊層（PRD §27、CONTRACT §15、BDD settings AC-ST-02／04／06／13、INV-26）：
//! `Policy::layered(base, local)` 純函數——local 只認可設定鍵、逐鍵驗證範圍、不合法整筆拒絕。

mod common;

use common::*;
use kunsu_core::policy::{Hhmm, LayerError, Policy, SettingsView, SETTABLE_KEYS};
use kunsu_core::guards::{DeferReason, GuardState};

/// 每個可設定鍵的 TOML 片段（`[section]\nkey = value`）。
fn local(key: &str, value: &str) -> String {
    let (section, name) = key.split_once('.').unwrap();
    format!("[{section}]\n{name} = {value}\n")
}

fn invalid_keys(err: LayerError) -> Vec<String> {
    match err {
        LayerError::Invalid(errors) => errors.into_iter().map(|e| e.key).collect(),
        LayerError::Syntax(msg) => panic!("預期逐鍵錯誤，收到語法錯誤：{msg}"),
    }
}

/// CONTRACT §15 範圍表：每個鍵的下界、上界都收（含端點），越界一律拒絕並帶鍵名與範圍；檔案層面不套用部分值。
#[test]
fn every_settable_key_accepts_its_bounds_and_rejects_beyond() {
    // (鍵, 合法值, 越界值, 訊息要提到的範圍)
    let table: &[(&str, &[&str], &[&str], &str)] = &[
        ("force.cycle_min", &["5", "120", "20"], &["4", "121", "0", "-1", "15.5", "\"15\""], "5–120"),
        ("force.warn_before_min", &["1", "14"], &["0", "15", "16", "-3", "2.5"], "1–14"),
        ("windows.quiet_enabled", &["true", "false"], &["1", "\"yes\""], ""),
        (
            "windows.quiet_hours",
            &["[\"23:00\", \"09:00\"]", "[\"00:00\", \"23:59\"]", "[\"22:30\", \"06:15\"]"],
            &[
                "[\"23:00\", \"23:00\"]",
                "[\"23:00\"]",
                "[\"23:00\", \"09:00\", \"10:00\"]",
                "[\"24:00\", \"09:00\"]",
                "[\"9:00\", \"23:00\"]",
                "[\"23:00:00\", \"09:00\"]",
                "[\"23：00\", \"09:00\"]",
                "\"23:00-09:00\"",
                "[2300, 900]",
            ],
            "",
        ),
        ("expiry.t1_default_after_h", &["1", "336", "48"], &["0", "337"], "1–336"),
        ("expiry.t2_supersede_after_d", &["1", "14"], &["0", "15", "30"], "1–14"),
        ("quota.per_source_daily_create", &["1", "100"], &["0", "101"], "1–100"),
        ("fatigue.skip_rate_window_d", &["1", "30"], &["0", "31"], "1–30"),
        ("fatigue.skip_rate_downgrade", &["0.1", "1.0", "1", "0.55"], &["0.09", "1.01", "0", "-0.5", "nan", "inf"], "10%–100%"),
    ];
    let mut covered: Vec<&str> = Vec::new();
    for (key, ok, bad, range) in table {
        covered.push(key);
        for v in *ok {
            let layered = Policy::layered("", Some(&local(key, v)))
                .unwrap_or_else(|e| panic!("{key} = {v} 應該合法：{e:?}"));
            assert_eq!(layered.overridden, vec![key.to_string()], "{key} = {v} 要記成被覆寫");
            assert!(layered.ignored.is_empty());
        }
        for v in *bad {
            let err = Policy::layered("", Some(&local(key, v)));
            let Err(LayerError::Invalid(errors)) = err else {
                panic!("{key} = {v} 應該被逐鍵拒絕，收到 {err:?}");
            };
            assert_eq!(errors.len(), 1, "{key} = {v}：只回這一鍵的錯誤 {errors:?}");
            assert_eq!(errors[0].key, *key);
            assert!(!errors[0].message.is_empty());
            assert!(errors[0].message.contains(range), "{key} 的訊息要帶範圍 {range}：{}", errors[0].message);
        }
    }
    let mut all: Vec<&str> = SETTABLE_KEYS.to_vec();
    all.sort();
    covered.sort();
    assert_eq!(all, covered, "範圍表要列舉每一個可設定鍵");
}

/// 疊上去的值真的生效；沒給的鍵沿用 base（不是 Policy::default）。
#[test]
fn local_values_layer_over_base_key_by_key() {
    let base = "[force]\ncycle_min = 30\nwarn_before_min = 5\n[quota]\nper_source_daily_create = 20\n";
    let local = "[force]\ncycle_min = 20\n[windows]\nquiet_hours = [\"22:00\", \"07:30\"]\nquiet_enabled = false\n\
                 [expiry]\nt1_default_after_h = 72\nt2_supersede_after_d = 10\n[fatigue]\nskip_rate_window_d = 14\nskip_rate_downgrade = 0.8\n";
    let l = Policy::layered(base, Some(local)).unwrap();
    let p = &l.policy;
    assert_eq!(p.force.cycle_min, 20);
    assert_eq!(p.force.warn_before_min, 5, "沒覆寫的鍵沿用 base");
    assert_eq!(p.force.deferred_retry_s, 1);
    assert_eq!(p.windows.quiet_hours, [Hhmm::new(22, 0), Hhmm::new(7, 30)]);
    assert!(!p.windows.quiet_enabled);
    assert_eq!(p.expiry.t1_default_after_h, 72);
    assert_eq!(p.expiry.t2_supersede_after_d, 10);
    assert_eq!(p.quota.per_source_daily_create, 20, "base 的值");
    assert_eq!(p.fatigue.skip_rate_window_d, 14);
    assert_eq!(p.fatigue.skip_rate_downgrade, 0.8);
    assert_eq!(
        l.overridden,
        vec![
            "force.cycle_min",
            "windows.quiet_enabled",
            "windows.quiet_hours",
            "expiry.t1_default_after_h",
            "expiry.t2_supersede_after_d",
            "fatigue.skip_rate_window_d",
            "fatigue.skip_rate_downgrade",
        ],
        "依 CONTRACT §15 表的順序列出"
    );

    // 沒有 local、空白 local：就是 base。
    let only_base = Policy::layered(base, None).unwrap();
    assert_eq!(only_base.policy, Policy::from_toml_str(base).unwrap());
    assert!(only_base.overridden.is_empty());
    assert_eq!(Policy::layered(base, Some("  \n# 只有註解\n")).unwrap().policy, only_base.policy);
}

/// 錢的護欄與其他不開放的鍵（`deferred_retry_s`、`t1_batch`、`discord.webhook`、未知鍵）在 local 裡一律忽略並回報，
/// 不能從設定放寬（INV-26）。
#[test]
fn local_only_knows_the_settable_keys() {
    let local = "[force]\ncycle_min = 20\ndeferred_retry_s = 99\n[windows]\nt1_batch = [\"10:00\"]\n\
                 [discord]\nwebhook = \"https://example.invalid/hook\"\n[money]\nmax_twd = 999999\n[force_typo]\nx = 1\n";
    let l = Policy::layered("", Some(local)).unwrap();
    assert_eq!(l.policy.force.cycle_min, 20);
    assert_eq!(l.policy.force.deferred_retry_s, 1, "不開放的鍵不生效");
    assert_eq!(l.policy.windows.t1_batch, Policy::default().windows.t1_batch);
    assert_eq!(l.policy.discord.webhook, "");
    assert_eq!(l.overridden, vec!["force.cycle_min"]);
    for k in ["force.deferred_retry_s", "windows.t1_batch", "discord.webhook", "money.max_twd", "force_typo.x"] {
        assert!(l.ignored.iter().any(|i| i == k), "{k} 要回報成被忽略：{:?}", l.ignored);
    }
}

/// 提前警告一定小於週期長度（不論哪一邊來自 base）；任一鍵不合法整筆拒絕，不套用部分值（AC-ST-02、AC-ST-13）。
#[test]
fn warn_stays_below_cycle_and_bad_files_apply_nothing() {
    // BDD 場景 2：週期 15、提前警告 15。
    let err = Policy::layered("", Some("[force]\nwarn_before_min = 15\n")).unwrap_err();
    let LayerError::Invalid(errors) = err else { panic!("要逐鍵錯誤") };
    assert_eq!(errors[0].key, "force.warn_before_min");
    assert_eq!(errors[0].message, "提前警告要小於週期長度（1–14 分鐘）");

    // local 把週期縮短到比 base 的提前警告還短：錯在提前警告，範圍跟著新週期。
    let base = "[force]\ncycle_min = 60\nwarn_before_min = 10\n";
    let err = Policy::layered(base, Some("[force]\ncycle_min = 8\n")).unwrap_err();
    assert_eq!(invalid_keys(err.clone()), vec!["force.warn_before_min"]);
    let LayerError::Invalid(errors) = err else { unreachable!() };
    assert!(errors[0].message.contains("1–7"), "{}", errors[0].message);
    // 兩個一起改到合法就收。
    assert!(Policy::layered(base, Some("[force]\ncycle_min = 8\nwarn_before_min = 2\n")).is_ok());

    // 多個鍵壞掉：全部回報，順序依表。
    let err = Policy::layered("", Some("[quota]\nper_source_daily_create = 0\n[force]\ncycle_min = 500\n")).unwrap_err();
    assert_eq!(invalid_keys(err), vec!["force.cycle_min", "quota.per_source_daily_create"]);

    // 語法錯：Syntax，不是部分套用。
    assert!(matches!(Policy::layered("", Some("[force\ncycle_min = 20")), Err(LayerError::Syntax(_))));
    // section 不是表、型別不對也是逐鍵錯誤。
    assert_eq!(invalid_keys(Policy::layered("", Some("[force]\ncycle_min = \"20\"\n")).unwrap_err()), vec!["force.cycle_min"]);
}

/// AC-ST-13：local 檔語法壞掉時，錯誤是一行人話（設定頁頂端與 doctor 都只放得下一行），帶行號、不整段貼原檔。
#[test]
fn syntax_errors_are_one_readable_line() {
    let Err(LayerError::Syntax(msg)) = Policy::layered("", Some("# 由 Kunsu 設定頁管理\n[force\ncycle_min = 20\n")) else {
        panic!("要回語法錯誤");
    };
    assert!(!msg.contains('\n'), "只有一行：{msg:?}");
    assert!(msg.contains("第 2 行"), "帶行號：{msg}");
    assert!(!msg.contains("cycle_min = 20"), "不貼原檔內容：{msg}");
}

/// base 本身不合法時照舊退回 `Policy::default()`，再疊 local（CONTRACT §15）。
#[test]
fn invalid_base_falls_back_to_defaults_before_layering() {
    let l = Policy::layered("[force]\ncycle_minn = 15\n", Some("[force]\ncycle_min = 25\n")).unwrap();
    let mut expected = Policy::default();
    expected.force.cycle_min = 25;
    assert_eq!(l.policy, expected);
}

/// INV-14：策略題最長只等 14 天——設定不能放寬（AC-ST-06）。
#[test]
fn t2_supersede_can_never_exceed_fourteen_days() {
    for days in [15, 21, 365, 10_000] {
        let err = Policy::layered("", Some(&local("expiry.t2_supersede_after_d", &days.to_string()))).unwrap_err();
        let LayerError::Invalid(errors) = err else { panic!("要逐鍵錯誤") };
        assert_eq!(errors[0].key, "expiry.t2_supersede_after_d");
        assert!(errors[0].message.contains("14 天"), "{}", errors[0].message);
    }
}

/// AC-ST-04：睡眠時間開關。關掉時 `in_quiet_hours` 恆 false，guard 不再因安靜時段擋強制；預設開。
#[test]
fn quiet_enabled_switch_turns_quiet_hours_off() {
    let night = ts("2026-07-31T15:30:00Z"); // 台北 23:30
    let on = Policy::default();
    assert!(on.windows.quiet_enabled, "預設開");
    assert!(on.in_quiet_hours(night));
    assert!(GuardState::default().blocks_force(night, &on));
    assert_eq!(GuardState::default().defer_reasons(night, &on).reasons, vec![DeferReason::QuietHours]);

    let off = Policy::layered("", Some("[windows]\nquiet_enabled = false\n")).unwrap().policy;
    assert!(!off.in_quiet_hours(night));
    assert!(!GuardState::default().blocks_force(night, &off));
    let reasons = GuardState::default().defer_reasons(night, &off);
    assert!(reasons.reasons.is_empty() && reasons.quiet_until.is_none());

    // policy.toml 也能直接寫（CONTRACT §10）；沒寫就是 true。
    assert!(!Policy::from_toml_str("[windows]\nquiet_enabled = false\n").unwrap().windows.quiet_enabled);
    assert!(Policy::from_toml_str("[windows]\nquiet_hours = [\"23:00\", \"09:00\"]\n").unwrap().windows.quiet_enabled);
}

/// `SettingsView`（settings_get 的 effective／defaults）：只露出可設定的值，quiet hours 拆成開始與結束 `HH:MM`。
#[test]
fn settings_view_mirrors_the_settable_values() {
    let view = SettingsView::from(&Policy::default());
    assert_eq!(
        serde_json::to_value(&view).unwrap(),
        serde_json::json!({
            "force": {"cycle_min": 15, "warn_before_min": 3},
            "windows": {"quiet_enabled": true, "quiet_start": "23:00", "quiet_end": "09:00"},
            "expiry": {"t1_default_after_h": 48, "t2_supersede_after_d": 14},
            "quota": {"per_source_daily_create": 12},
            "fatigue": {"skip_rate_window_d": 7, "skip_rate_downgrade": 0.5}
        })
    );
    let p = Policy::layered("", Some("[windows]\nquiet_hours = [\"22:15\", \"06:45\"]\nquiet_enabled = false\n")).unwrap().policy;
    let v = serde_json::to_value(SettingsView::from(&p)).unwrap();
    assert_eq!(v["windows"], serde_json::json!({"quiet_enabled": false, "quiet_start": "22:15", "quiet_end": "06:45"}));
}
