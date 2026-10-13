//! 主視窗設定的後端（PRD §27、CONTRACT §15、BDD settings AC-ST-01..06／10／11／13／14、INV-26）。
//!
//! 設定檔一律在 tempdir（`set_settings_paths`），HOME 也是 tempdir——不碰使用者的 `~/.kunsu`、`~/.claude*`、`~/.codex`。

mod common;

use std::path::{Path, PathBuf};

use chrono::Duration;
use common::*;
use kunsu_core::policy::Policy;
use kunsu_core::types::{State, Tier};
use kunsu_server::settings::{
    doctor_line, effective_policy, IntegrationStatus, SettingsPaths, StatusEnv, LOCAL_FILE_KEY,
};
use kunsu_server::ServerError;
use serde_json::{json, Map, Value};
use tempfile::TempDir;

const REPO_POLICY: &str = include_str!("../../../config/policy.toml");

/// 測試用 `~/.kunsu`：base 是 repo 的 config/policy.toml（安裝時複製的那份）。
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
    fn base(&self) -> PathBuf {
        self.dir.path().join("policy.toml")
    }
    fn local(&self) -> PathBuf {
        self.dir.path().join("policy.local.toml")
    }
    fn local_bytes(&self) -> Option<Vec<u8>> {
        std::fs::read(self.local()).ok()
    }
    fn local_table(&self) -> toml::Table {
        std::fs::read_to_string(self.local()).unwrap().parse::<toml::Table>().unwrap()
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

fn error_keys(rej: &kunsu_server::settings::SettingsRejection) -> Vec<String> {
    rej.errors.iter().map(|e| e.key.clone()).collect()
}

/// AC-ST-01：儲存只寫 local 檔的可設定鍵（原子寫入、檔頭標明由設定頁管理），base 不變，生效 policy 立即改變；
/// 重開（新 handle／啟動時讀兩檔）與重裝（重新複製 base）後仍在。
#[tokio::test]
async fn ac_st_01_save_writes_only_the_local_file_and_applies_at_once() {
    let (ctx, home) = settings_ctx().await;
    let base_before = std::fs::read(home.base()).unwrap();

    let snap = ctx.handle.settings_set(&changes(json!({"force.cycle_min": 20}))).expect("合法值要存得進去");
    assert_eq!(snap.effective.force.cycle_min, 20);
    assert_eq!(snap.defaults.force.cycle_min, 15, "預設值是 base 的值");
    assert_eq!(snap.overridden, vec!["force.cycle_min"]);
    assert_eq!(snap.local_path, home.local().to_string_lossy());
    assert!(snap.local_error.is_none());
    assert_eq!(ctx.handle.policy().force.cycle_min, 20, "不等熱載，立即生效");

    let raw = std::fs::read_to_string(home.local()).unwrap();
    assert!(raw.starts_with("# 由 Kunsu 設定頁管理"), "檔頭註解：{raw}");
    let mut expected = toml::Table::new();
    let mut force = toml::Table::new();
    force.insert("cycle_min".into(), toml::Value::Integer(20));
    expected.insert("force".into(), toml::Value::Table(force));
    assert_eq!(home.local_table(), expected, "只多了 force.cycle_min = 20");
    assert_eq!(std::fs::read(home.base()).unwrap(), base_before, "base 不變");
    assert_eq!(home.entries(), vec!["policy.local.toml", "policy.toml"], "原子寫入不留暫存檔");

    // 再存另一張卡片：前一個值保留。
    let snap = ctx
        .handle
        .settings_set(&changes(json!({"windows.quiet_hours": ["22:00", "07:00"], "windows.quiet_enabled": true})))
        .unwrap();
    assert_eq!(snap.effective.force.cycle_min, 20);
    assert_eq!(snap.overridden, vec!["force.cycle_min", "windows.quiet_enabled", "windows.quiet_hours"]);
    assert_eq!(
        serde_json::to_value(&snap.effective.windows).unwrap(),
        json!({"quiet_enabled": true, "quiet_start": "22:00", "quiet_end": "07:00"})
    );

    // 重開：另一個 handle 讀同一份檔，啟動路徑（effective_policy）也讀得到。
    let reopened = ctx.reopen().await;
    reopened.set_settings_paths(home.paths());
    let got = reopened.settings_get().unwrap();
    assert_eq!(got.effective.force.cycle_min, 20);
    assert!(got.overridden.contains(&"force.cycle_min".to_string()), "仍標「已改過」");
    let base = std::fs::read_to_string(home.base()).unwrap();
    let local = std::fs::read_to_string(home.local()).unwrap();
    let boot = effective_policy(Some(&base), Some(&local));
    assert_eq!(boot.policy.force.cycle_min, 20);
    assert!(boot.local_error.is_none());

    // 重裝：build-install 重新複製 base；local 不動，熱載後仍是 20。
    std::fs::write(home.base(), REPO_POLICY).unwrap();
    ctx.handle.reload_policy_files().unwrap();
    assert_eq!(ctx.handle.policy().force.cycle_min, 20);
    assert_eq!(ctx.handle.settings_get().unwrap().effective.force.cycle_min, 20);
}

/// AC-ST-02：超出範圍整筆拒絕——回逐鍵 `{errors: [{key, message}]}`，local 檔與生效 policy 完全不變。
#[tokio::test]
async fn ac_st_02_out_of_range_is_rejected_whole_and_changes_nothing() {
    let (ctx, home) = settings_ctx().await;

    // 還沒有 local 檔時被拒：不建檔。
    let rej = ctx.handle.settings_set(&changes(json!({"force.warn_before_min": 15}))).unwrap_err();
    assert_eq!(error_keys(&rej), vec!["force.warn_before_min"]);
    assert_eq!(rej.errors[0].message, "提前警告要小於週期長度（1–14 分鐘）");
    assert!(!home.local().exists(), "被拒時不建 local 檔");
    assert_eq!(ctx.handle.policy(), Policy::from_toml_str(REPO_POLICY).unwrap());

    ctx.handle.settings_set(&changes(json!({"force.cycle_min": 30}))).unwrap();
    let local_before = home.local_bytes().unwrap();
    let policy_before = ctx.handle.policy();

    // 合法的鍵和不合法的鍵一起送：整筆拒絕，合法那個也不寫。
    let rej = ctx
        .handle
        .settings_set(&changes(json!({"quota.per_source_daily_create": 40, "force.warn_before_min": 30})))
        .unwrap_err();
    assert_eq!(error_keys(&rej), vec!["force.warn_before_min"]);
    assert!(rej.errors[0].message.contains("1–29"), "範圍跟著目前週期：{}", rej.errors[0].message);
    assert_eq!(
        serde_json::to_value(&rej).unwrap(),
        json!({"errors": [{"key": "force.warn_before_min", "message": rej.errors[0].message}]}),
        "IPC 形狀 {{errors: [{{key, message}}]}}"
    );

    // 不在表上的鍵、SettingsView 的讀取面鍵名、null、型別不對、INV-14 上限。
    for bad in [
        json!({"force.deferred_retry_s": 5}),
        json!({"discord.webhook": "https://example.invalid"}),
        json!({"windows.t1_batch": ["10:00"]}),
        json!({"windows.quiet_start": "22:00"}),
        json!({"force.cycle_min": null}),
        json!({"force.cycle_min": "20"}),
        json!({"force.cycle_min": 20.5}),
        json!({"windows.quiet_enabled": 1}),
        json!({"windows.quiet_hours": "23:00-09:00"}),
        json!({"windows.quiet_hours": ["23:00", "23:00"]}),
        json!({"expiry.t2_supersede_after_d": 15}),
        json!({"fatigue.skip_rate_downgrade": 0.05}),
        json!({"cycle_min": 20}),
    ] {
        let rej = ctx.handle.settings_set(&changes(bad.clone())).unwrap_err();
        let key = bad.as_object().unwrap().keys().next().unwrap().clone();
        assert_eq!(error_keys(&rej), vec![key.clone()], "{bad}");
        assert!(!rej.errors[0].message.is_empty());
        assert_eq!(home.local_bytes().unwrap(), local_before, "{bad}：檔案不變");
        assert_eq!(ctx.handle.policy(), policy_before, "{bad}：生效值不變");
    }
    // 空的 changes 不算錯，也不改任何東西。
    let snap = ctx.handle.settings_set(&Map::new()).unwrap();
    assert_eq!(snap.effective.force.cycle_min, 30);
    assert_eq!(home.local_bytes().unwrap(), local_before);
}

/// AC-ST-02／13（自我稽核補）：一次列出所有錯的鍵（不在表上的鍵與超出範圍的鍵一起），設定頁一次標完；local 檔語法壞掉時
/// 拒絕訊息是一行（行號＋原因），不貼原檔。
#[tokio::test]
async fn ac_st_02_rejection_lists_every_bad_key_in_one_line_each() {
    let (ctx, home) = settings_ctx().await;
    let rej = ctx
        .handle
        .settings_set(&changes(json!({"force.warn_before_min": 20, "windows.quiet_start": "22:00", "quota.per_source_daily_create": 0})))
        .unwrap_err();
    assert_eq!(error_keys(&rej), vec!["force.warn_before_min", "quota.per_source_daily_create", "windows.quiet_start"]);
    assert!(!home.local().exists());

    std::fs::write(home.local(), "# 註解\n[force\ncycle_min = 20\n").unwrap();
    let rej = ctx.handle.settings_set(&changes(json!({"force.cycle_min": 20}))).unwrap_err();
    assert_eq!(error_keys(&rej), vec![LOCAL_FILE_KEY]);
    let msg = &rej.errors[0].message;
    assert!(!msg.contains('\n') && msg.contains("第 2 行"), "{msg:?}");
    assert!(msg.contains("全部回復預設"), "{msg}");
}

/// AC-ST-03：回復預設移除這張卡片的鍵、立即生效；`[]` 全部回復；回復後的組合不合法就整筆拒絕。
#[tokio::test]
async fn ac_st_03_reset_removes_the_card_keys_and_applies_at_once() {
    let (ctx, home) = settings_ctx().await;
    ctx.handle
        .settings_set(&changes(json!({"force.cycle_min": 30, "force.warn_before_min": 5, "quota.per_source_daily_create": 40})))
        .unwrap();

    let snap = ctx
        .handle
        .settings_reset(&["force.cycle_min".to_string(), "force.warn_before_min".to_string()])
        .unwrap();
    assert_eq!(snap.effective.force.cycle_min, 15);
    assert_eq!(snap.effective.force.warn_before_min, 3);
    assert_eq!(snap.overridden, vec!["quota.per_source_daily_create"]);
    assert_eq!(ctx.handle.policy().force.cycle_min, 15, "立即生效");
    let table = home.local_table();
    assert!(table.get("force").is_none(), "force 鍵已移除：{table:?}");
    assert_eq!(table["quota"]["per_source_daily_create"].as_integer(), Some(40));

    // 回復後剩下的組合不合法：整筆拒絕、檔案不變。
    ctx.handle.settings_set(&changes(json!({"force.cycle_min": 60, "force.warn_before_min": 30}))).unwrap();
    let before = home.local_bytes().unwrap();
    let rej = ctx.handle.settings_reset(&["force.cycle_min".to_string()]).unwrap_err();
    assert_eq!(error_keys(&rej), vec!["force.warn_before_min"]);
    assert_eq!(home.local_bytes().unwrap(), before);
    assert_eq!(ctx.handle.policy().force.cycle_min, 60);

    // 不在表上的鍵拒絕。
    let rej = ctx.handle.settings_reset(&["force.deferred_retry_s".to_string()]).unwrap_err();
    assert_eq!(error_keys(&rej), vec!["force.deferred_retry_s"]);

    // 全部回復預設。
    let snap = ctx.handle.settings_reset(&[]).unwrap();
    assert!(snap.overridden.is_empty());
    assert_eq!(snap.effective, snap.defaults);
    assert_eq!(ctx.handle.policy(), Policy::from_toml_str(REPO_POLICY).unwrap());
    let raw = std::fs::read_to_string(home.local()).unwrap();
    assert!(raw.starts_with("# 由 Kunsu 設定頁管理"));
    assert!(home.local_table().is_empty(), "只剩檔頭：{raw}");
}

/// AC-ST-04：睡眠時間開關控制 guard——開著時到期不強制（不重設週期），關掉後立即強制，週期時間不變。
#[tokio::test]
async fn ac_st_04_quiet_switch_controls_the_guard_without_resetting_cycles() {
    let (ctx, _home) = settings_ctx().await;
    let created = ts("2026-07-31T15:10:00Z"); // 台北 23:10，安靜時段內
    ctx.handle.create_question(base_req("pa-quiet-1"), created).await.unwrap();
    let q0 = ctx.handle.get_question("pa-quiet-1").await.unwrap().unwrap();
    assert_eq!(q0.force_at, Some(created + Duration::minutes(15)));

    let night = ts("2026-07-31T15:30:00Z"); // 已過 force_at
    ctx.handle.tick(created + Duration::minutes(12)).await.unwrap();
    ctx.handle.tick(night).await.unwrap();
    let q = ctx.handle.get_question("pa-quiet-1").await.unwrap().unwrap();
    assert_ne!(q.state, State::Forced, "睡眠時間內不強制");
    assert!(!ctx.handle.guards_all_green(night).await.unwrap());
    assert!(ctx.handle.defer_reasons(night).await.unwrap().quiet_until.is_some());

    ctx.handle.settings_set(&changes(json!({"windows.quiet_enabled": false}))).unwrap();
    assert!(ctx.handle.guards_all_green(night).await.unwrap(), "關掉後安靜時段不再擋");
    assert!(ctx.handle.defer_reasons(night).await.unwrap().reasons.is_empty());
    ctx.handle.tick(night + Duration::seconds(5)).await.unwrap();
    let q = ctx.handle.get_question("pa-quiet-1").await.unwrap().unwrap();
    assert_eq!(q.state, State::Forced, "關掉睡眠時間後依原進度強制");
    assert_eq!((q.cycle_started_at, q.warn_at, q.force_at, q.cycle_number), (q0.cycle_started_at, q0.warn_at, q0.force_at, q0.cycle_number));

    // 改時間：白天設成安靜時段也立即生效。
    ctx.handle
        .settings_set(&changes(json!({"windows.quiet_enabled": true, "windows.quiet_hours": ["13:00", "15:00"]})))
        .unwrap();
    assert!(!ctx.handle.guards_all_green(now()).await.unwrap(), "台北 14:00 落在 13:00–15:00");
}

/// AC-ST-05：強制詢問參數只影響新的週期；既有題目的 cycle_started_at／warn_at／force_at 不變。
#[tokio::test]
async fn ac_st_05_force_cycle_changes_only_apply_to_new_cycles() {
    let (ctx, _home) = settings_ctx().await;
    let t0 = now();
    ctx.handle.create_question(base_req("pa-cycle-old"), t0).await.unwrap();
    ctx.handle.create_question(base_req("pa-cycle-keep"), t0).await.unwrap();
    let old = ctx.handle.get_question("pa-cycle-old").await.unwrap().unwrap();
    assert_eq!(old.force_at, Some(t0 + Duration::minutes(15)));

    ctx.handle.settings_set(&changes(json!({"force.cycle_min": 30, "force.warn_before_min": 5}))).unwrap();
    for id in ["pa-cycle-old", "pa-cycle-keep"] {
        let q = ctx.handle.get_question(id).await.unwrap().unwrap();
        assert_eq!((q.cycle_started_at, q.warn_at, q.force_at, q.cycle_number), (old.cycle_started_at, old.warn_at, old.force_at, old.cycle_number), "{id}");
    }

    // 新題用新值。
    let t1 = t0 + Duration::minutes(1);
    ctx.handle.create_question(base_req("pa-cycle-new"), t1).await.unwrap();
    let new = ctx.handle.get_question("pa-cycle-new").await.unwrap().unwrap();
    assert_eq!(new.force_at, Some(t1 + Duration::minutes(30)));
    assert_eq!(new.warn_at, Some(t1 + Duration::minutes(25)));

    // 稍後再問開始的新週期用新值。
    let t2 = t0 + Duration::minutes(2);
    let dismissed = ctx.handle.dismiss_question("pa-cycle-old", t2).await.unwrap();
    assert_eq!(dismissed.force_at, Some(t2 + Duration::minutes(30)));
    assert_eq!(dismissed.warn_at, Some(t2 + Duration::minutes(25)));
    assert_eq!(dismissed.cycle_number, old.cycle_number + 1);

    // 沒被碰的題照原本時間：12 分鐘警告、15 分鐘強制（白天 guards 全綠）。
    ctx.handle.tick(t0 + Duration::minutes(12)).await.unwrap();
    assert_eq!(ctx.handle.get_question("pa-cycle-keep").await.unwrap().unwrap().state, State::Warned);
    ctx.handle.tick(t0 + Duration::minutes(15)).await.unwrap();
    assert_eq!(ctx.handle.get_question("pa-cycle-keep").await.unwrap().unwrap().state, State::Forced);
}

/// AC-ST-06：到期、配額與疲勞存下後立即套用在之後的判斷。
#[tokio::test]
async fn ac_st_06_expiry_and_quota_apply_to_later_decisions() {
    let (ctx, _home) = settings_ctx().await;
    ctx.handle
        .settings_set(&changes(json!({
            "expiry.t1_default_after_h": 1,
            "quota.per_source_daily_create": 1,
            "fatigue.skip_rate_window_d": 30,
            "fatigue.skip_rate_downgrade": 0.9
        })))
        .unwrap();
    let p = ctx.handle.policy();
    assert_eq!((p.fatigue.skip_rate_window_d, p.fatigue.skip_rate_downgrade), (30, 0.9));

    let mut req = base_req("pa-expiry-1");
    req.tier = Tier::T1;
    ctx.handle.create_question(req, now()).await.unwrap();
    let second = ctx.handle.create_question(base_req("pa-expiry-2"), now()).await;
    assert!(matches!(second, Err(ServerError::QuotaExceeded { limit: 1, .. })), "每來源每日 1 題：{second:?}");

    ctx.handle.tick(now() + Duration::minutes(61)).await.unwrap();
    let q = ctx.handle.get_question("pa-expiry-1").await.unwrap().unwrap();
    assert_eq!(q.state, State::Expired, "一般題 1 小時沒回答就到期");
}

/// AC-ST-13：local 檔壞掉（語法錯或超出範圍）→ 生效只用 base、不崩潰；settings_get 帶錯誤與位置；全部回復預設可恢復。
#[tokio::test]
async fn ac_st_13_broken_local_file_uses_base_and_reset_all_recovers() {
    let (ctx, home) = settings_ctx().await;
    let base_policy = Policy::from_toml_str(REPO_POLICY).unwrap();
    ctx.handle.settings_set(&changes(json!({"force.cycle_min": 40}))).unwrap();

    // 超出範圍：只用 base，錯誤點名那個鍵。
    std::fs::write(home.local(), "[force]\ncycle_min = 500\n[quota]\nper_source_daily_create = 30\n").unwrap();
    ctx.handle.reload_policy_files().unwrap();
    assert_eq!(ctx.handle.policy(), base_policy, "不套用部分值");
    let got = ctx.handle.settings_get().unwrap();
    let err = got.local_error.clone().expect("要顯示錯誤");
    assert!(err.contains("force.cycle_min"), "{err}");
    assert_eq!(got.effective, got.defaults);
    assert!(got.overridden.is_empty());
    assert_eq!(got.local_path, home.local().to_string_lossy());
    // 把壞掉的鍵改回範圍內就能存（另一個合法鍵保留）。
    let fixed = ctx.handle.settings_set(&changes(json!({"force.cycle_min": 25}))).unwrap();
    assert!(fixed.local_error.is_none());
    assert_eq!(fixed.effective.force.cycle_min, 25);
    assert_eq!(fixed.effective.quota.per_source_daily_create, 30);

    // 語法錯：只用 base；settings_set 拒絕並指向檔案；全部回復預設清空。
    std::fs::write(home.local(), "[force\ncycle_min = 20").unwrap();
    ctx.handle.reload_policy_files().unwrap();
    assert_eq!(ctx.handle.policy(), base_policy);
    let got = ctx.handle.settings_get().unwrap();
    assert!(got.local_error.is_some());
    let rej = ctx.handle.settings_set(&changes(json!({"force.cycle_min": 20}))).unwrap_err();
    assert_eq!(error_keys(&rej), vec![LOCAL_FILE_KEY]);
    // 語法壞掉時，回復任何一張卡片都清空整個檔。
    let snap = ctx.handle.settings_reset(&["quota.per_source_daily_create".to_string()]).unwrap();
    assert!(snap.local_error.is_none());
    assert!(home.local_table().is_empty());
    assert_eq!(ctx.handle.policy(), base_policy);

    // 啟動路徑：壞檔不崩潰，只用 base 並帶錯誤。
    let boot = effective_policy(Some(REPO_POLICY), Some("[force\n"));
    assert_eq!(boot.policy, base_policy);
    assert!(boot.local_error.is_some());
    // base 讀不到（啟動逾時）：預設值再疊 local。
    let boot = effective_policy(None, Some("[force]\ncycle_min = 45\n"));
    assert_eq!(boot.policy.force.cycle_min, 45);
}

/// 熱載（CONTRACT §15）：base 壞掉沿用上一次的生效值；local 壞掉只用 base；兩檔正常就疊層。
#[tokio::test]
async fn hot_reload_reads_both_files() {
    let (ctx, home) = settings_ctx().await;
    std::fs::write(home.local(), "[force]\ncycle_min = 33\n").unwrap();
    ctx.handle.reload_policy_files().unwrap();
    assert_eq!(ctx.handle.policy().force.cycle_min, 33);

    std::fs::write(home.base(), "[force]\ncycle_minn = 1\n").unwrap();
    assert!(ctx.handle.reload_policy_files().is_err(), "base 壞掉要回報");
    assert_eq!(ctx.handle.policy().force.cycle_min, 33, "base 壞掉沿用上一次");

    std::fs::write(home.base(), "[force]\ncycle_min = 15\nwarn_before_min = 4\n").unwrap();
    ctx.handle.reload_policy_files().unwrap();
    let p = ctx.handle.policy();
    assert_eq!((p.force.cycle_min, p.force.warn_before_min), (33, 4));
}

/// 路徑沒注入時 settings_* 回錯誤、不碰任何檔案（測試與 kunsu-serverd 永遠不會寫到真的 ~/.kunsu）。
#[tokio::test]
async fn settings_need_injected_paths() {
    let ctx = TestCtx::new().await;
    let rej = ctx.handle.settings_get().unwrap_err();
    assert_eq!(error_keys(&rej), vec![LOCAL_FILE_KEY]);
    let rej = ctx.handle.settings_set(&changes(json!({"force.cycle_min": 20}))).unwrap_err();
    assert_eq!(error_keys(&rej), vec![LOCAL_FILE_KEY]);
    assert!(ctx.handle.settings_reset(&[]).is_err());
    assert_eq!(ctx.handle.policy(), Policy::default());
}

/// INV-26：有待決題、有在跑的週期時做各種儲存、回復與熱載——沒有任何題目被回答、取消或改變 cycle timestamp／cycle_number，
/// 不發任何事件、不寫帳本；只有 local 檔與生效 policy 改變。
#[tokio::test]
async fn inv_settings_never_touch_questions() {
    let (ctx, home) = settings_ctx().await;
    let t0 = now();
    for id in ["pa-inv26-pending", "pa-inv26-warned", "pa-inv26-forced", "pa-inv26-dismissed", "pa-inv26-answered"] {
        ctx.handle.create_question(base_req(id), t0).await.unwrap();
    }
    let mut t2 = base_req("pa-inv26-t2");
    t2.tier = Tier::T2;
    t2.source.routine_id = Some("inv26-t2".into());
    ctx.handle.create_question(t2, t0).await.unwrap();
    ctx.handle.dismiss_question("pa-inv26-dismissed", t0 + Duration::minutes(1)).await.unwrap();
    ctx.handle.answer_question("pa-inv26-answered", ui_answer("a"), t0 + Duration::minutes(2)).await.unwrap();
    // 12 分鐘警告、15 分鐘強制；稍後再問的那題晚一分鐘，停在警告。
    ctx.handle.tick(t0 + Duration::minutes(12)).await.unwrap();
    ctx.handle.tick(t0 + Duration::minutes(15)).await.unwrap();
    let mut ids: Vec<String> = Vec::new();
    for q in ctx.handle.list_questions(true).await.unwrap() {
        ids.push(q.id.clone());
    }
    assert!(ids.len() >= 6);
    let states: Vec<State> = ctx.handle.list_questions(true).await.unwrap().iter().map(|q| q.state).collect();
    assert!(states.contains(&State::Forced) && states.contains(&State::Answered), "{states:?}");

    let snapshot = |qs: Vec<kunsu_core::types::Question>| -> Vec<Value> {
        let mut v: Vec<Value> = qs.iter().map(|q| serde_json::to_value(q).unwrap()).collect();
        v.sort_by_key(|q| q["id"].as_str().unwrap().to_string());
        v
    };
    let before = snapshot(ctx.handle.list_questions(true).await.unwrap());
    let ledger_dir = ctx.vault.path().join("logs").join("asks");
    let ledger_before = dir_bytes(&ledger_dir);
    let mut rx = ctx.handle.subscribe();

    let ops: Vec<Box<dyn Fn() -> Result<(), String> + '_>> = vec![
        Box::new(|| ctx.handle.settings_set(&changes(json!({"force.cycle_min": 30}))).map(drop).map_err(|e| format!("{e:?}"))),
        Box::new(|| ctx.handle.settings_set(&changes(json!({"force.warn_before_min": 10}))).map(drop).map_err(|e| format!("{e:?}"))),
        Box::new(|| ctx.handle.settings_set(&changes(json!({"windows.quiet_enabled": false, "windows.quiet_hours": ["01:00", "02:00"]}))).map(drop).map_err(|e| format!("{e:?}"))),
        Box::new(|| {
            ctx.handle
                .settings_set(&changes(json!({
                    "expiry.t1_default_after_h": 1,
                    "expiry.t2_supersede_after_d": 1,
                    "quota.per_source_daily_create": 1,
                    "fatigue.skip_rate_window_d": 1,
                    "fatigue.skip_rate_downgrade": 0.1
                })))
                .map(drop)
                .map_err(|e| format!("{e:?}"))
        }),
        Box::new(|| match ctx.handle.settings_set(&changes(json!({"force.cycle_min": 5, "force.warn_before_min": 5}))) {
            Err(_) => Ok(()),
            Ok(_) => Err("不合法的值竟然存進去了".into()),
        }),
        Box::new(|| ctx.handle.settings_reset(&["force.cycle_min".to_string(), "force.warn_before_min".to_string()]).map(drop).map_err(|e| format!("{e:?}"))),
        Box::new(|| ctx.handle.settings_get().map(drop).map_err(|e| format!("{e:?}"))),
        Box::new(|| ctx.handle.reload_policy_files()),
        Box::new(|| ctx.handle.settings_reset(&[]).map(drop).map_err(|e| format!("{e:?}"))),
    ];
    for (i, op) in ops.iter().enumerate() {
        op().unwrap_or_else(|e| panic!("第 {i} 個操作失敗：{e}"));
        assert_eq!(snapshot(ctx.handle.list_questions(true).await.unwrap()), before, "第 {i} 個設定操作動到了題目");
        assert!(drain_events(&mut rx).is_empty(), "第 {i} 個設定操作發了事件");
        assert_eq!(dir_bytes(&ledger_dir), ledger_before, "第 {i} 個設定操作寫了帳本");
    }
    // 壞掉的 local 檔熱載也一樣。
    std::fs::write(home.local(), "[force\n").unwrap();
    ctx.handle.reload_policy_files().unwrap();
    assert_eq!(snapshot(ctx.handle.list_questions(true).await.unwrap()), before);

    // 到期參數在建題時固定（2026-10-09）：把一般題到期時間與最長等待天數都調到最低，時間繼續走——已在等的題（含一般題與策略題）
    // 不會因為設定改動而提早到期、被取代或被依預設作答；只有週期照常推進。
    ctx.handle.settings_reset(&[]).unwrap();
    ctx.handle
        .settings_set(&changes(json!({"expiry.t1_default_after_h": 1, "expiry.t2_supersede_after_d": 1})))
        .unwrap();
    let waiting: Vec<String> = ctx
        .handle
        .list_questions(true)
        .await
        .unwrap()
        .iter()
        .filter(|q| !q.state.is_terminal())
        .map(|q| q.id.clone())
        .collect();
    assert!(waiting.len() >= 5, "{waiting:?}");
    for hours in [2, 25] {
        ctx.handle.tick(t0 + Duration::hours(hours)).await.unwrap();
        for id in &waiting {
            let q = ctx.handle.get_question(id).await.unwrap().unwrap();
            assert!(!q.state.is_terminal(), "t0+{hours}h：{id} 因為調低的到期設定提早結案（{:?}）", q.state);
            assert!(q.answer.is_none(), "t0+{hours}h：{id} 被代答");
        }
    }
}

fn dir_bytes(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<(String, Vec<u8>)> = rd
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_file())
        .map(|p| (p.file_name().unwrap().to_string_lossy().to_string(), std::fs::read(&p).unwrap()))
        .collect();
    out.sort();
    out
}

fn status_env(home: &Path) -> StatusEnv {
    StatusEnv {
        server_addr: "127.0.0.1:4999".into(),
        home: home.to_path_buf(),
        kunsu_dir: home.join(".kunsu"),
    }
}

fn integration<'a>(status: &'a kunsu_server::settings::AppStatus, id: &str) -> &'a kunsu_server::settings::Integration {
    status.integrations.iter().find(|i| i.id == id).unwrap_or_else(|| panic!("缺 {id}"))
}

/// AC-ST-10／AC-ST-11（資料面）：app_status 回版本、運作時間、位址、路徑、帳本健康與四項 Agent 連線的真實狀態；
/// 未設定給可複製指令（與 kunsu doctor 同源）；只看存在與鍵，任何 token 與設定內容都不出現在回應裡。
#[tokio::test]
async fn app_status_reports_real_integrations_without_secrets() {
    let ctx = TestCtx::new().await;
    let home = TempDir::new().unwrap();
    let env = status_env(home.path());

    let s = ctx.handle.app_status(&env, now()).await.unwrap();
    assert_eq!(s.version, env!("CARGO_PKG_VERSION"));
    assert_eq!(s.server_addr, "127.0.0.1:4999");
    assert_eq!(s.vault_root, ctx.vault.path().to_string_lossy());
    assert_eq!(s.db_path, ctx.db_path.to_string_lossy());
    let v = serde_json::to_value(&s).unwrap();
    assert_eq!(v["ledger"], json!({"backlog": 0, "open_json_dirty": false, "stuck_s": 0}));
    assert!(v["uptime_s"].is_u64());
    let ids: Vec<&str> = s.integrations.iter().map(|i| i.id.as_str()).collect();
    assert_eq!(ids, vec!["claude_mcp", "codex_mcp", "cli", "skill"]);
    for i in &s.integrations {
        assert_eq!(i.status, IntegrationStatus::Missing, "{}：什麼都沒裝", i.id);
        assert!(!i.detail.is_empty());
    }
    let kunsu_dir = home.path().join(".kunsu");
    assert_eq!(
        integration(&s, "claude_mcp").fix.as_deref(),
        Some(
            format!(
                "claude mcp add --scope user --transport http kunsu http://127.0.0.1:4777/mcp --header \"Authorization: Bearer $(cat {}/token)\"",
                kunsu_dir.display()
            )
            .as_str()
        )
    );
    assert_eq!(
        integration(&s, "codex_mcp").fix.as_deref(),
        Some("python3 ~/.kunsu/plugins/kunsu/scripts/manage.py install"),
        "Codex 走本機外掛安裝，貼上就能用（2026-10-09）"
    );
    assert!(integration(&s, "cli").fix.as_deref().unwrap().contains("open -a Kunsu"));
    assert!(integration(&s, "skill").fix.is_none(), "skill 沒有 repo 內正本，不給指令");
    // fix 與 kunsu doctor 印的同源（drift 檢查）。
    let doctor = include_str!("../../kunsu-cli/src/main.rs");
    assert!(doctor.contains("claude mcp add --scope user --transport http kunsu http://127.0.0.1:4777/mcp"));
    assert!(doctor.contains("--header \\\"Authorization: Bearer $(cat {}/token)\\\""));
    assert!(doctor.contains("python3 ~/.kunsu/plugins/kunsu/scripts/manage.py install"), "doctor 印同一行 Codex 修復指令");

    // 全部裝好（設定裡塞了假的 token，回應裡不能出現）。
    std::fs::write(
        home.path().join(".claude.json"),
        json!({"mcpServers": {"kunsu": {"type": "http", "headers": {"Authorization": "Bearer SECRET-CLAUDE-1"}}}, "oauth": "SECRET-OAUTH"}).to_string(),
    )
    .unwrap();
    std::fs::create_dir_all(home.path().join(".codex")).unwrap();
    std::fs::write(
        home.path().join(".codex/config.toml"),
        "[mcp_servers.kunsu]\nurl = \"http://127.0.0.1:4777/mcp\"\nhttp_headers = { Authorization = \"Bearer SECRET-CODEX-1\" }\n",
    )
    .unwrap();
    std::fs::create_dir_all(kunsu_dir.join("bin")).unwrap();
    std::fs::write(kunsu_dir.join("bin/kunsu"), "#!/bin/sh\n").unwrap();
    std::fs::write(kunsu_dir.join("token"), "SECRET-KUNSU-TOKEN").unwrap();
    set_mode(&kunsu_dir.join("bin/kunsu"), 0o755);
    std::fs::create_dir_all(home.path().join(".claude/skills/kunsu")).unwrap();
    std::fs::write(home.path().join(".claude/skills/kunsu/SKILL.md"), "---\nname: kunsu\n---\nSECRET-SKILL-BODY\n").unwrap();
    let s = ctx.handle.app_status(&env, now()).await.unwrap();
    for i in &s.integrations {
        assert_eq!(i.status, IntegrationStatus::Ok, "{}：{}", i.id, i.detail);
        assert!(i.fix.is_none(), "{} 已設定不給指令", i.id);
    }
    let raw = serde_json::to_string(&s).unwrap();
    assert!(!raw.contains("SECRET"), "回應不能帶 token 或設定內容：{raw}");

    // Codex：直接連線停用不算；啟用的本機外掛算（AC-LP-05 doctor 規則）；其他外掛不算；格式壞掉＝無法確認。
    for (raw, expected) in [
        ("[mcp_servers.kunsu]\nenabled = false\nurl = \"x\"\n", IntegrationStatus::Missing),
        ("[plugins.\"kunsu@personal\"]\nenabled = true\n", IntegrationStatus::Ok),
        ("[plugins.\"kunsu@personal\"]\nenabled = false\n", IntegrationStatus::Missing),
        ("[plugins.\"other@personal\"]\nenabled = true\n", IntegrationStatus::Missing),
        ("[mcp_servers.kunsu\nurl = ", IntegrationStatus::Unknown),
    ] {
        std::fs::write(home.path().join(".codex/config.toml"), raw).unwrap();
        let s = ctx.handle.app_status(&env, now()).await.unwrap();
        assert_eq!(integration(&s, "codex_mcp").status, expected, "{raw}");
    }

    // Claude 設定檔壞掉＝無法確認；沒有 kunsu 鍵＝未設定。
    std::fs::write(home.path().join(".claude.json"), "{not json").unwrap();
    let s = ctx.handle.app_status(&env, now()).await.unwrap();
    assert_eq!(integration(&s, "claude_mcp").status, IntegrationStatus::Unknown);
    std::fs::write(home.path().join(".claude.json"), json!({"mcpServers": {"other": {}}}).to_string()).unwrap();
    let s = ctx.handle.app_status(&env, now()).await.unwrap();
    assert_eq!(integration(&s, "claude_mcp").status, IntegrationStatus::Missing);
    assert!(integration(&s, "claude_mcp").fix.is_some());

    // CLI 不可執行＝未設定。
    set_mode(&kunsu_dir.join("bin/kunsu"), 0o644);
    let s = ctx.handle.app_status(&env, now()).await.unwrap();
    assert_eq!(integration(&s, "cli").status, IntegrationStatus::Missing);
}

fn set_mode(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
}

/// `kunsu doctor` 的一行（不算失敗）：有沒有個人覆寫、local 檔壞掉時說清楚。
#[test]
fn doctor_line_reports_personal_overrides() {
    let path = Path::new("/tmp/x/policy.local.toml");
    let none = doctor_line(Some(REPO_POLICY), None, path);
    assert!(none.contains("沒有個人覆寫"), "{none}");
    let some = doctor_line(Some(REPO_POLICY), Some("[force]\ncycle_min = 20\n[windows]\nquiet_enabled = false\n"), path);
    assert!(some.contains("force.cycle_min") && some.contains("windows.quiet_enabled"), "{some}");
    assert!(some.contains("/tmp/x/policy.local.toml"), "{some}");
    let broken = doctor_line(Some(REPO_POLICY), Some("[force\n"), path);
    assert!(broken.contains("壞掉"), "{broken}");
}
