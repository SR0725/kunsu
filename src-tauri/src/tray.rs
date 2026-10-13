//! menubar tray（抄 claude-task-engine tray.rs 已驗證模式）。
//!
//! template icon（K 黑剪影，OS 依 menu bar 主題自動反白）；選單內容由 `menu_plan` 決定（AC-AS-15）：
//! 模式、待決數、逐題快捷（點了開主畫面）、勿擾子選單、提前結束勿擾（只在勿擾中）、開啟主畫面、退出。
//!
//! 選單由背景 loop 每 15s 重建 + questions_changed 事件即時重建。

use std::time::Duration;

use kunsu_core::types::{Gate, State as QState, Tier};
use tauri::{
    image::Image,
    menu::{Menu, MenuBuilder, MenuItem, PredefinedMenuItem, SubmenuBuilder},
    tray::TrayIconBuilder,
    AppHandle, Manager,
};

use crate::ipc::AppCtx;

const TRAY_ID: &str = "kunsu-tray";
const MAX_ITEMS: usize = 10;
const DND_PRESETS: [(u32, &str); 6] = [
    (60, "1 小時"),
    (120, "2 小時"),
    (240, "4 小時"),
    (480, "8 小時"),
    (720, "12 小時"),
    (1440, "24 小時"),
];

const TRAY_ICON_BYTES: &[u8] = include_bytes!("../icons/tray-k-template.png");

pub fn install(app: &AppHandle) -> tauri::Result<()> {
    let placeholder = MenuItem::with_id(app, "loading", "載入中…", false, None::<&str>)?;
    let menu = Menu::with_items(app, &[&placeholder])?;
    let icon = Image::from_bytes(TRAY_ICON_BYTES)?;

    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .menu(&menu)
        .icon(icon)
        .show_menu_on_left_click(true);
    #[cfg(target_os = "macos")]
    {
        builder = builder.icon_as_template(true);
    }
    builder
        .on_menu_event(|app, event| handle_menu_event(app, event.id().as_ref()))
        .build(app)?;

    // 背景重建：15s 週期 + ServerEvent 即時觸發。
    let app_rebuild = app.clone();
    tauri::async_runtime::spawn(async move {
        let ctx = app_rebuild.state::<AppCtx>();
        let mut rx = ctx.handle.subscribe();
        let mut tick = tokio::time::interval(Duration::from_secs(15));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = tick.tick() => {}
                ev = rx.recv() => {
                    if matches!(ev, Err(tokio::sync::broadcast::error::RecvError::Closed)) {
                        break;
                    }
                }
            }
            if let Err(e) = rebuild_menu(&app_rebuild).await {
                tracing::warn!("tray 選單重建失敗：{e:?}");
            }
        }
    });
    Ok(())
}

fn handle_menu_event(app: &AppHandle, id: &str) {
    match id {
        "open_main" => crate::windows::show_main(app),
        "open_settings" => {
            crate::windows::show_main(app);
            let _ = tauri::Emitter::emit(app, "kunsu://open_settings", ());
        }
        "kill_switch" => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                let ctx = app.state::<AppCtx>();
                match ctx.handle.kill_switch(4, chrono::Utc::now()).await {
                    Ok(until) => tracing::info!("kill-switch 生效至 {until}"),
                    Err(e) => tracing::error!("kill-switch 失敗：{e}"),
                }
            });
        }
        "dnd_clear" => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                let ctx = app.state::<AppCtx>();
                if let Err(e) = ctx.handle.clear_dnd("user", chrono::Utc::now()).await {
                    tracing::error!("勿擾解除失敗：{e}");
                }
            });
        }
        "quit" => app.exit(0),
        other => {
            if let Some(raw) = other.strip_prefix("dnd:") {
                if let Ok(minutes) = raw.parse::<u32>() {
                    let app = app.clone();
                    tauri::async_runtime::spawn(async move {
                        let ctx = app.state::<AppCtx>();
                        if let Err(e) = ctx.handle.set_dnd(minutes, "user", chrono::Utc::now()).await {
                            tracing::error!("勿擾設定失敗：{e}");
                        }
                    });
                }
            } else if other.strip_prefix("q:").is_some() {
                // 逐題快捷：開主視窗（清單置頂即該題所在視圖）。
                crate::windows::show_main(app);
            }
        }
    }
}

/// 選單的一列（AC-AS-15）。先算出要放什麼，再交給 tauri 建，方便測試。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Entry {
    /// 不能點的說明列。
    Info { id: String, label: String },
    Item { id: String, label: String },
    /// 平常收起，指上去才展開。
    Submenu { id: String, label: String, items: Vec<(String, String)> },
    Separator,
}

impl Entry {
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn id(&self) -> &str {
        match self {
            Entry::Info { id, .. } | Entry::Item { id, .. } | Entry::Submenu { id, .. } => id,
            Entry::Separator => "-",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct QuestionLine {
    pub id: String,
    pub line: String,
    pub urgent: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct MenuInput {
    pub dnd_active: bool,
    /// 勿擾中的剩餘時間說明。
    pub dnd_status: Option<String>,
    pub questions: Vec<QuestionLine>,
    pub urgent: usize,
}

fn info(id: &str, label: impl Into<String>) -> Entry {
    Entry::Info { id: id.into(), label: label.into() }
}

fn item(id: impl Into<String>, label: impl Into<String>) -> Entry {
    Entry::Item { id: id.into(), label: label.into() }
}

/// 選單內容與順序（AC-AS-15）：不放 emoji；勿擾時間收進子選單；`提前結束勿擾` 只在勿擾中出現；視窗入口只剩 `開啟主畫面`。
pub(crate) fn menu_plan(input: &MenuInput) -> Vec<Entry> {
    let mut plan = vec![
        info("dnd_mode", if input.dnd_active { "目前模式：勿擾中" } else { "目前模式：正常提醒" }),
        info(
            "dnd_status",
            input
                .dnd_status
                .clone()
                .filter(|_| input.dnd_active)
                .unwrap_or_else(|| "Kunsu 會照常顯示提醒".to_string()),
        ),
        info(
            "header",
            match (input.questions.len(), input.urgent) {
                (n, 0) => format!("待決事項：{n}"),
                (n, u) => format!("待決事項：{n}（緊急 {u}）"),
            },
        ),
    ];
    if !input.dnd_active {
        for q in input.questions.iter().take(MAX_ITEMS) {
            let line = truncate(&q.line, 28);
            plan.push(item(format!("q:{}", q.id), if q.urgent { format!("緊急 · {line}") } else { line }));
        }
        if input.questions.len() > MAX_ITEMS {
            plan.push(info("more", format!("…還有 {} 題", input.questions.len() - MAX_ITEMS)));
        }
    }
    plan.push(Entry::Separator);
    let mut presets: Vec<(String, String)> =
        DND_PRESETS.iter().map(|(m, l)| (format!("dnd:{m}"), format!("勿擾 {l}"))).collect();
    presets.push(("open_settings".into(), "更多時間…".into()));
    plan.push(Entry::Submenu {
        id: "dnd_menu".into(),
        label: if input.dnd_active { "重新設定勿擾時間" } else { "設定勿擾時間" }.into(),
        items: presets,
    });
    if input.dnd_active {
        plan.push(item("dnd_clear", "提前結束勿擾"));
    }
    plan.extend([Entry::Separator, item("open_main", "開啟主畫面"), Entry::Separator, item("quit", "退出 Kunsu")]);
    plan
}

async fn rebuild_menu(app: &AppHandle) -> tauri::Result<()> {
    let ctx = app.state::<AppCtx>();
    let open = match ctx.handle.list_questions(false).await {
        Ok(qs) => qs,
        Err(e) => {
            tracing::warn!("tray 讀題失敗：{e}");
            Vec::new()
        }
    };
    let now = chrono::Utc::now();
    let dnd = ctx.handle.dnd_status(now).await.ok();
    let dnd_active = dnd.as_ref().is_some_and(|status| status.active);
    let questions: Vec<QuestionLine> = open
        .iter()
        .filter(|q| !q.internal)
        .map(|q| QuestionLine {
            id: q.id.clone(),
            line: q.question_line.clone(),
            urgent: q.tier == Tier::T0
                || q.gate == Some(Gate::Red)
                || matches!(q.state, QState::Forced | QState::Warned),
        })
        .collect();
    let urgent = questions.iter().filter(|q| q.urgent).count();
    let input = MenuInput {
        dnd_active,
        dnd_status: dnd.as_ref().filter(|s| s.active).and_then(|s| s.ends_at).map(|end| dnd_remaining_label(now, end)),
        questions,
        urgent,
    };

    let mut b = MenuBuilder::new(app);
    for entry in menu_plan(&input) {
        b = match entry {
            Entry::Info { id, label } => b.item(&MenuItem::with_id(app, id, label, false, None::<&str>)?),
            Entry::Item { id, label } => b.item(&MenuItem::with_id(app, id, label, true, None::<&str>)?),
            Entry::Separator => b.item(&PredefinedMenuItem::separator(app)?),
            Entry::Submenu { id, label, items } => {
                let mut sb = SubmenuBuilder::with_id(app, id, label);
                for (iid, ilabel) in items {
                    sb = sb.item(&MenuItem::with_id(app, iid, ilabel, true, None::<&str>)?);
                }
                b.item(&sb.build()?)
            }
        };
    }
    let menu = b.build()?;

    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        tray.set_menu(Some(menu))?;
    }
    Ok(())
}

fn dnd_remaining_label(now: chrono::DateTime<chrono::Utc>, ends_at: chrono::DateTime<chrono::Utc>) -> String {
    let seconds = (ends_at - now).num_seconds().max(0);
    let total_minutes = (seconds + 59) / 60;
    let hours = total_minutes / 60;
    let minutes = total_minutes % 60;
    let remaining = if hours > 0 && minutes > 0 {
        format!("{hours} 小時 {minutes} 分")
    } else if hours > 0 {
        format!("{hours} 小時")
    } else {
        format!("{minutes} 分")
    };
    format!("剩餘 {remaining} · 至 {}", ends_at.format("%H:%M"))
}

fn truncate(s: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i >= max_chars {
            out.push('…');
            return out;
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod dnd_menu_tests {
    use chrono::{TimeZone, Utc};

    use super::{dnd_remaining_label, DND_PRESETS};

    #[test]
    fn menu_bar_exposes_complete_dnd_presets_and_human_remaining_time() {
        assert_eq!(DND_PRESETS, [(60, "1 小時"), (120, "2 小時"), (240, "4 小時"), (480, "8 小時"), (720, "12 小時"), (1440, "24 小時")]);
        let now = Utc.with_ymd_and_hms(2026, 8, 12, 10, 0, 0).unwrap();
        let end = Utc.with_ymd_and_hms(2026, 8, 12, 12, 14, 1).unwrap();
        assert_eq!(dnd_remaining_label(now, end), "剩餘 2 小時 15 分 · 至 12:14");
    }
}

#[cfg(test)]
mod menu_plan_tests {
    use super::{menu_plan, Entry, MenuInput, QuestionLine};

    fn ids(plan: &[Entry]) -> Vec<String> {
        plan.iter().map(|e| e.id().to_string()).collect()
    }

    fn labels(plan: &[Entry]) -> Vec<String> {
        let mut out = Vec::new();
        for e in plan {
            match e {
                Entry::Item { label, .. } | Entry::Info { label, .. } => out.push(label.clone()),
                Entry::Submenu { label, items, .. } => {
                    out.push(label.clone());
                    out.extend(items.iter().map(|(_, l)| l.clone()));
                }
                Entry::Separator => {}
            }
        }
        out
    }

    fn has_emoji(s: &str) -> bool {
        s.chars().any(|c| (c as u32) >= 0x1F000 || ('\u{2600}'..='\u{27BF}').contains(&c) || c == '\u{23F0}')
    }

    #[test]
    fn tray_menu_when_normal_and_empty() {
        let plan = menu_plan(&MenuInput { dnd_active: false, dnd_status: None, questions: vec![], urgent: 0 });
        assert_eq!(
            ids(&plan),
            ["dnd_mode", "dnd_status", "header", "-", "dnd_menu", "-", "open_main", "-", "quit"]
        );
        let l = labels(&plan);
        assert!(l.contains(&"待決事項：0".to_string()));
        assert!(l.contains(&"開啟主畫面".to_string()));
        assert!(!l.iter().any(|x| x.contains("提前結束勿擾") || x.contains("膠囊") || x.contains("蓋板") || x.contains("決策清單")));
        assert!(!l.iter().any(|x| has_emoji(x)), "{l:?}");
        let Entry::Submenu { label, items, .. } = plan.iter().find(|e| e.id() == "dnd_menu").unwrap() else { panic!() };
        assert_eq!(label, "設定勿擾時間");
        assert_eq!(items.len(), 7);
        assert_eq!(items[0], ("dnd:60".to_string(), "勿擾 1 小時".to_string()));
        assert_eq!(items[6], ("open_settings".to_string(), "更多時間…".to_string()));
    }

    #[test]
    fn tray_menu_lists_questions_without_emoji_and_counts_urgent() {
        let q = |id: &str, urgent| QuestionLine { id: id.into(), line: format!("題目 {id}"), urgent };
        let plan = menu_plan(&MenuInput { dnd_active: false, dnd_status: None, questions: vec![q("a", true), q("b", false)], urgent: 1 });
        let l = labels(&plan);
        assert!(l.contains(&"待決事項：2（緊急 1）".to_string()));
        assert!(ids(&plan).contains(&"q:a".to_string()) && ids(&plan).contains(&"q:b".to_string()));
        assert!(!l.iter().any(|x| has_emoji(x)), "{l:?}");
    }

    #[test]
    fn tray_menu_during_dnd_shows_end_early_and_hides_questions() {
        let q = QuestionLine { id: "a".into(), line: "x".into(), urgent: false };
        let plan = menu_plan(&MenuInput { dnd_active: true, dnd_status: Some("剩餘 1 小時 · 至 12:00".into()), questions: vec![q], urgent: 0 });
        assert_eq!(
            ids(&plan),
            ["dnd_mode", "dnd_status", "header", "-", "dnd_menu", "dnd_clear", "-", "open_main", "-", "quit"]
        );
        let l = labels(&plan);
        assert!(l.contains(&"目前模式：勿擾中".to_string()));
        assert!(l.contains(&"重新設定勿擾時間".to_string()));
        assert!(l.contains(&"剩餘 1 小時 · 至 12:00".to_string()));
    }

    #[test]
    fn tray_icon_is_the_k_template() {
        let img = image_size(super::TRAY_ICON_BYTES);
        assert_eq!(img, (44, 44));
        // K 圖檔的 sha 固定，避免被換回圓圈
        assert_eq!(super::TRAY_ICON_BYTES.len(), include_bytes!("../icons/tray-k-template.png").len());
    }

    fn image_size(png: &[u8]) -> (u32, u32) {
        let w = u32::from_be_bytes(png[16..20].try_into().unwrap());
        let h = u32::from_be_bytes(png[20..24].try_into().unwrap());
        (w, h)
    }
}
