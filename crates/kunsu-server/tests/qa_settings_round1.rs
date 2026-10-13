//! 獨立 QA 第 1 輪：主視窗設定的後端（PRD §27、CONTRACT §15、BDD settings、INV-26）。
//!
//! 設定檔與 HOME 一律在 tempdir（`set_settings_paths`），不碰使用者的 `~/.kunsu`。
//! 名稱以 `qa_gap_` 開頭的測試記錄的是「規格沒寫清楚、要使用者決定」的現況行為（目前會過）；其他測試照規格寫。

mod common;

use std::path::{Path, PathBuf};

use chrono::Duration;
use common::*;
use kunsu_core::policy::Policy;
use kunsu_core::types::{Channel, DefaultAction, OnExpire, State, Tier};
use kunsu_server::settings::{SettingsPaths, LOCAL_FILE_KEY};
use serde_json::{json, Map, Value};
use tempfile::TempDir;

const REPO_POLICY: &str = include_str!("../../../config/policy.toml");

struct Home {
    dir: TempDir,
}

impl Home {
    fn new() -> Home {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("policy.toml"), REPO_POLICY).unwrap();
        Home { dir }
    }
    fn paths(&self) -> SettingsPaths {
        SettingsPaths::in_dir(self.dir.path())
    }
    fn local(&self) -> PathBuf {
        self.dir.path().join("policy.local.toml")
    }
    fn entries(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(self.dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        names.sort();
        names
    }
}

async fn settings_ctx() -> (TestCtx, Home) {
    let ctx = TestCtx::new().await;
    let home = Home::new();
    ctx.handle.set_settings_paths(home.paths());
    (ctx, home)
}

fn changes(v: Value) -> Map<String, Value> {
    v.as_object().unwrap().clone()
}

fn set_mode(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
}

/// AC-ST-01（原子寫入＋序列化）：設定頁、熱載與另一個主視窗同時存不同的卡片——每一筆都要留下來（讀改寫不能互蓋），
/// 檔案永遠是完整合法的 TOML，不留暫存檔，生效 policy 與檔案一致。
#[tokio::test]
async fn qa_concurrent_saves_are_serialized_and_none_is_lost() {
    let (ctx, home) = settings_ctx().await;
    let writes: Vec<(&str, Value)> = vec![
        ("force.cycle_min", json!(40)),
        ("force.warn_before_min", json!(7)),
        ("windows.quiet_enabled", json!(false)),
        ("windows.quiet_hours", json!(["22:30", "06:45"])),
        ("expiry.t1_default_after_h", json!(72)),
        ("expiry.t2_supersede_after_d", json!(9)),
        ("quota.per_source_daily_create", json!(30)),
        ("fatigue.skip_rate_window_d", json!(21)),
        ("fatigue.skip_rate_downgrade", json!(0.35)),
    ];
    std::thread::scope(|s| {
        for (key, value) in &writes {
            let handle = ctx.handle.clone();
            s.spawn(move || {
                let mut m = Map::new();
                m.insert((*key).to_string(), value.clone());
                handle.settings_set(&m).unwrap_or_else(|e| panic!("{key}：{e}"));
            });
        }
        let handle = ctx.handle.clone();
        s.spawn(move || {
            for _ in 0..20 {
                handle.reload_policy_files().unwrap();
            }
        });
    });

    let raw = std::fs::read_to_string(home.local()).unwrap();
    let table: toml::Table = raw.parse().unwrap_or_else(|e| panic!("local 檔不是合法 TOML：{e}\n{raw}"));
    for (key, _) in &writes {
        let (section, name) = key.split_once('.').unwrap();
        assert!(table.get(section).and_then(|t| t.get(name)).is_some(), "{key} 被另一筆儲存蓋掉了：\n{raw}");
    }
    assert_eq!(home.entries(), vec!["policy.local.toml", "policy.toml"], "不留暫存檔");
    let p = ctx.handle.policy();
    assert_eq!((p.force.cycle_min, p.force.warn_before_min), (40, 7));
    assert!(!p.windows.quiet_enabled);
    assert_eq!((p.expiry.t1_default_after_h, p.expiry.t2_supersede_after_d), (72, 9));
    assert_eq!(p.quota.per_source_daily_create, 30);
    assert_eq!((p.fatigue.skip_rate_window_d, p.fatigue.skip_rate_downgrade), (21, 0.35));
    assert_eq!(ctx.handle.settings_get().unwrap().overridden.len(), writes.len());
}

/// AC-ST-01／02：寫不進去（目錄唯讀）時整筆拒絕——錯誤掛在 `policy.local.toml`、原本的檔案與生效 policy 完全不變、
/// 不留下半個暫存檔。
#[tokio::test]
async fn qa_unwritable_directory_rejects_whole_and_changes_nothing() {
    let (ctx, home) = settings_ctx().await;
    ctx.handle.settings_set(&changes(json!({"force.cycle_min": 25}))).unwrap();
    let before_file = std::fs::read(home.local()).unwrap();
    let before_policy = ctx.handle.policy();

    set_mode(home.dir.path(), 0o555);
    let probe = home.dir.path().join(".probe");
    if std::fs::write(&probe, "x").is_ok() {
        // root 不受權限限制，這個情境測不到
        let _ = std::fs::remove_file(&probe);
        set_mode(home.dir.path(), 0o755);
        return;
    }
    let result = ctx.handle.settings_set(&changes(json!({"quota.per_source_daily_create": 50})));
    let reset = ctx.handle.settings_reset(&[]);
    set_mode(home.dir.path(), 0o755);

    let rej = result.expect_err("唯讀目錄不能回報成功");
    assert_eq!(rej.errors.iter().map(|e| e.key.as_str()).collect::<Vec<_>>(), vec![LOCAL_FILE_KEY]);
    assert!(reset.is_err(), "回復預設也寫不進去");
    assert_eq!(std::fs::read(home.local()).unwrap(), before_file, "檔案不變");
    assert_eq!(ctx.handle.policy(), before_policy, "生效值不變");
    assert_eq!(home.entries(), vec!["policy.local.toml", "policy.toml"], "不留暫存檔");
}

/// AC-ST-03／熱載：使用者手動刪掉 local 檔＝全部回預設；熱載後生效值回到 base，設定頁不再標「已改過」。
#[tokio::test]
async fn qa_deleting_the_local_file_returns_to_base_on_reload() {
    let (ctx, home) = settings_ctx().await;
    ctx.handle.settings_set(&changes(json!({"force.cycle_min": 20, "windows.quiet_enabled": false}))).unwrap();
    assert_eq!(ctx.handle.policy().force.cycle_min, 20);
    std::fs::remove_file(home.local()).unwrap();
    ctx.handle.reload_policy_files().unwrap();
    assert_eq!(ctx.handle.policy(), Policy::from_toml_str(REPO_POLICY).unwrap());
    let snap = ctx.handle.settings_get().unwrap();
    assert!(snap.overridden.is_empty());
    assert!(snap.local_error.is_none());
    assert_eq!(snap.effective, snap.defaults);
}

/// AC-ST-05／INV-26（反方向）：把週期改短，既有題目的警告與強制時間不會被往前拉——t0+5～11 分鐘仍是 pending，
/// 依原本的 12／15 分鐘警告與強制，cycle_number 不變；之後新開始的週期才用 5 分鐘。
#[tokio::test]
async fn qa_shorter_cycle_never_pulls_an_existing_force_forward() {
    let (ctx, _home) = settings_ctx().await;
    let t0 = now();
    ctx.handle.create_question(base_req("pa-qa-short"), t0).await.unwrap();
    let q0 = ctx.handle.get_question("pa-qa-short").await.unwrap().unwrap();
    ctx.handle.settings_set(&changes(json!({"force.cycle_min": 5, "force.warn_before_min": 1}))).unwrap();

    for minute in [4, 5, 6, 9, 11] {
        ctx.handle.tick(t0 + Duration::minutes(minute)).await.unwrap();
        let q = ctx.handle.get_question("pa-qa-short").await.unwrap().unwrap();
        assert_eq!(q.state, State::Pending, "t0+{minute} 分鐘：既有題不能照新的 5 分鐘週期提早警告／強制");
        assert_eq!(
            (q.cycle_started_at, q.warn_at, q.force_at, q.cycle_number),
            (q0.cycle_started_at, q0.warn_at, q0.force_at, q0.cycle_number),
            "t0+{minute} 分鐘"
        );
    }
    ctx.handle.tick(t0 + Duration::minutes(12)).await.unwrap();
    assert_eq!(ctx.handle.get_question("pa-qa-short").await.unwrap().unwrap().state, State::Warned);
    ctx.handle.tick(t0 + Duration::minutes(15)).await.unwrap();
    let forced = ctx.handle.get_question("pa-qa-short").await.unwrap().unwrap();
    assert_eq!(forced.state, State::Forced);
    assert_eq!(forced.cycle_number, q0.cycle_number);

    // 稍後再問開始的新週期才用 5／1。
    let t1 = t0 + Duration::minutes(16);
    let next = ctx.handle.dismiss_question("pa-qa-short", t1).await.unwrap();
    assert_eq!(next.force_at, Some(t1 + Duration::minutes(5)));
    assert_eq!(next.warn_at, Some(t1 + Duration::minutes(4)));
}

/// AC-ST-13／CONTRACT §15「不讀、不回傳任何 token 或設定內容」：local 檔壞掉時，設定頁看到的錯誤與 settings_set 的拒絕
/// 只說位置與原因，不把檔案裡的字串（可能是使用者手貼的 webhook 或 token）原樣帶回畫面。
#[tokio::test]
async fn qa_local_errors_never_echo_file_contents() {
    let (ctx, home) = settings_ctx().await;
    let cases = [
        "[discord]\nwebhook = \"https://discord.com/api/webhooks/SECRET-HOOK\n",
        "[force]\ncycle_min = \"SECRET-VALUE\"\n",
        "[force]\ncycle_min = 500\n[discord]\nwebhook = \"https://discord.com/api/webhooks/SECRET-HOOK\"\n",
        "SECRET-TOKEN-LINE\n[force]\ncycle_min = 20\n",
    ];
    for raw in cases {
        std::fs::write(home.local(), raw).unwrap();
        ctx.handle.reload_policy_files().unwrap();
        let snap = ctx.handle.settings_get().unwrap();
        let err = snap.local_error.clone().unwrap_or_else(|| panic!("{raw:?} 應該被判定壞掉"));
        assert!(!err.contains("SECRET"), "設定頁錯誤帶出了檔案內容：{err}");
        let shown = serde_json::to_string(&snap).unwrap();
        assert!(!shown.contains("SECRET"), "settings_get 回應帶出了檔案內容：{shown}");
        if let Err(rej) = ctx.handle.settings_set(&changes(json!({"quota.per_source_daily_create": 20}))) {
            let shown = serde_json::to_string(&rej).unwrap();
            assert!(!shown.contains("SECRET"), "settings_set 拒絕訊息帶出了檔案內容：{shown}");
        }
    }
}

/// CONTRACT §15 settings_reset：鍵裡只要有一個不在表上就整筆拒絕，合法的那些也不移除。
#[tokio::test]
async fn qa_reset_with_one_unknown_key_changes_nothing() {
    let (ctx, home) = settings_ctx().await;
    ctx.handle.settings_set(&changes(json!({"force.cycle_min": 30, "quota.per_source_daily_create": 40}))).unwrap();
    let before = std::fs::read(home.local()).unwrap();
    let rej = ctx
        .handle
        .settings_reset(&["force.cycle_min".to_string(), "discord.webhook".to_string()])
        .unwrap_err();
    assert_eq!(rej.errors.iter().map(|e| e.key.as_str()).collect::<Vec<_>>(), vec!["discord.webhook"]);
    assert_eq!(std::fs::read(home.local()).unwrap(), before);
    assert_eq!(ctx.handle.policy().force.cycle_min, 30);
}

/// AC-ST-13：重裝換了 base（例如 repo 把週期改成 10），local 裡留著的提前警告 12 變成不合法——不崩潰、只用 base，
/// 設定頁點名那個鍵；把它改回範圍內就能存。
#[tokio::test]
async fn qa_reinstalled_base_that_invalidates_local_falls_back_to_base() {
    let (ctx, home) = settings_ctx().await;
    ctx.handle.settings_set(&changes(json!({"force.warn_before_min": 12}))).unwrap();
    let new_base = REPO_POLICY.replace("cycle_min = 15", "cycle_min = 10");
    assert_ne!(new_base, REPO_POLICY, "repo policy.toml 的寫法變了，請更新這個測試");
    std::fs::write(home.dir.path().join("policy.toml"), &new_base).unwrap();
    ctx.handle.reload_policy_files().unwrap();
    let base = Policy::from_toml_str(&new_base).unwrap();
    assert_eq!(ctx.handle.policy(), base, "只用 base，不套用部分值");
    let snap = ctx.handle.settings_get().unwrap();
    let err = snap.local_error.expect("要顯示錯誤");
    assert!(err.contains("force.warn_before_min"), "{err}");
    let fixed = ctx.handle.settings_set(&changes(json!({"force.warn_before_min": 4}))).unwrap();
    assert!(fixed.local_error.is_none());
    assert_eq!(ctx.handle.policy().force.warn_before_min, 4);
}

/// 規格（2026-10-09 主線代作者決定，翻轉 QA 第 1 輪的 qa_gap_lowering_expiry_retroactively_auto_answers_waiting_questions）：
/// 到期規則在建題時固定在題目上。把「一般題到期時間」從 48 改成 1 小時，已經在等的一般題不會在下一次 tick 到期、更不會被
/// 依預設選項代答；它照建題當下的 48 小時規則到期。存檔之後才建的題用新的 1 小時。
#[tokio::test]
async fn qa_lowering_expiry_never_expires_waiting_questions() {
    let (ctx, _home) = settings_ctx().await;
    let t0 = now();
    let pick_b = || DefaultAction { on_expire: OnExpire::PickOption, option_id: Some("b".into()) };
    let mut req = base_req("pa-qa-retro");
    req.tier = Tier::T1;
    req.default_action = pick_b();
    ctx.handle.create_question(req, t0).await.unwrap();
    // 建題時的規則是 48 小時；兩小時後使用者把到期時間改成 1 小時。
    let t2h = t0 + Duration::hours(2);
    ctx.handle.settings_set(&changes(json!({"expiry.t1_default_after_h": 1}))).unwrap();
    ctx.handle.tick(t2h).await.unwrap();
    let q = ctx.handle.get_question("pa-qa-retro").await.unwrap().unwrap();
    assert!(!q.state.is_terminal(), "設定一存，已在等的題不能照新規則到期（{:?}）", q.state);
    assert!(q.answer.is_none(), "Kunsu 不能因為設定改動替使用者作答：{:?}", q.answer);

    // 存檔之後才建的題用新規則：1 小時後依預設處理。
    let mut fresh = base_req("pa-qa-fresh");
    fresh.tier = Tier::T1;
    fresh.default_action = pick_b();
    ctx.handle.create_question(fresh, t2h).await.unwrap();
    ctx.handle.tick(t2h + Duration::minutes(61)).await.unwrap();
    let fresh = ctx.handle.get_question("pa-qa-fresh").await.unwrap().unwrap();
    assert_eq!(fresh.state, State::Expired, "新題照新的 1 小時規則到期");
    assert_eq!(fresh.answer.as_ref().map(|a| a.channel), Some(Channel::AutoPolicy));
    let old = ctx.handle.get_question("pa-qa-retro").await.unwrap().unwrap();
    assert!(!old.state.is_terminal(), "舊題仍照 48 小時（{:?}）", old.state);

    // 舊題照建題當下的 48 小時規則到期。
    ctx.handle.tick(t0 + Duration::hours(48)).await.unwrap();
    let old = ctx.handle.get_question("pa-qa-retro").await.unwrap().unwrap();
    assert_eq!(old.state, State::Expired, "第 48 小時照原規則到期");
}

/// 規格（2026-10-09 主線代作者決定，翻轉 QA 第 1 輪的 qa_gap_strategy_supersede_setting_also_expires_non_strategy_questions）：
/// 設定頁的「題目最長等待天數」就是所有題目的壽命上限（INV-14）——策略題（T2）到期由新題取代、其他題到期結案。它跟其他到期參數
/// 一樣在建題時固定：設成 1 天之後建的題，一般題（48 小時規則還沒到）第 25 小時也到期；設定之前建的題不受影響。
#[tokio::test]
async fn qa_max_pending_days_caps_every_tier_of_new_questions() {
    let (ctx, _home) = settings_ctx().await;
    let t0 = now();
    ctx.handle.create_question(base_req("pa-qa-before"), t0).await.unwrap();
    ctx.handle.settings_set(&changes(json!({"expiry.t2_supersede_after_d": 1}))).unwrap();
    ctx.handle.create_question(base_req("pa-qa-t1-age"), t0).await.unwrap();
    let mut t2 = base_req("pa-qa-t2-age");
    t2.tier = Tier::T2;
    t2.source.routine_id = Some("qa-t2-age".into());
    ctx.handle.create_question(t2, t0).await.unwrap();

    ctx.handle.tick(t0 + Duration::hours(25)).await.unwrap();
    let t1 = ctx.handle.get_question("pa-qa-t1-age").await.unwrap().unwrap();
    assert_eq!(t1.tier, Tier::T1);
    assert_eq!(t1.state, State::Expired, "一般題也受最長等待天數管（48 小時規則還沒到）");
    let t2 = ctx.handle.get_question("pa-qa-t2-age").await.unwrap().unwrap();
    assert_eq!(t2.tier, Tier::T2);
    assert_eq!(t2.state, State::Superseded, "策略題到期由新題取代");
    let before = ctx.handle.get_question("pa-qa-before").await.unwrap().unwrap();
    assert!(!before.state.is_terminal(), "設定之前建的題照建題當下的 14 天（{:?}）", before.state);
}
