//! 強制蓋板呈現的握手狀態機（docs/bdd/overlay-presentation.md AC-OP-04～07）。
//!
//! 蓋板視窗只有在網頁回報「這個轉場的可操作內容已掛載」後才會到前景；1.5 秒沒收到
//! 確認就隱藏、改開主視窗並重新載入蓋板網頁。Esc 在網頁 2 秒內沒回應時由原生層完成
//! dismissal。純狀態機：輸入事件與單調時間（毫秒），輸出要 shell 執行的命令，方便表驅動測試。

use std::collections::HashMap;

use serde::Serialize;

pub const MOUNT_TIMEOUT_MS: u64 = 1_500;
pub const ESCAPE_TIMEOUT_MS: u64 = 2_000;
pub const ALERT_AFTER_TIMEOUTS: u32 = 3;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Transition {
    pub id: u64,
    pub origin_x: f64,
    pub origin_y: f64,
    pub origin_size: f64,
    pub source: &'static str,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Cmd {
    /// 視窗仍隱藏時套上目標螢幕完整 frame。
    ApplyFrame,
    /// 把轉場送給蓋板視窗（只送蓋板，不廣播）。
    Emit(Transition),
    /// 讀回 frame 驗證後帶到前景並取得鍵盤焦點。
    ShowAndFocus,
    Hide,
    /// 重新載入蓋板網頁（就緒逾時後）。
    ReloadWeb,
    /// 開主視窗並聚焦待決題（就緒逾時的後備）。
    OpenMainFallback,
    /// 連續就緒逾時告警。
    Alert(u32),
    /// 網頁沒處理 Esc：原生層對目前題目做 dismissal 並隱藏。
    NativeDismiss,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Phase {
    Hidden,
    Pending { transition: Transition, requested_at: u64 },
    Shown { transition: Transition },
}

#[derive(Debug)]
pub struct Presenter {
    phase: Phase,
    web_ready: bool,
    next_id: u64,
    consecutive_timeouts: u32,
    escape_at: Option<u64>,
}

impl Default for Presenter {
    fn default() -> Self {
        Self::new()
    }
}

impl Presenter {
    pub fn new() -> Self {
        Self { phase: Phase::Hidden, web_ready: false, next_id: 1, consecutive_timeouts: 0, escape_at: None }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn phase(&self) -> &Phase {
        &self.phase
    }

    /// 最近一次建立的轉場 id（0＝還沒有）：蓋板 session 的身分。收起後不變，直到下一個新轉場——晚到的 dismissal 仍算在
    /// 同一個 session（`DismissLedger`）。
    pub fn last_transition_id(&self) -> u64 {
        self.next_id - 1
    }

    /// 蓋板 session 是否開著（等待掛載或已顯示）。
    pub fn session_open(&self) -> bool {
        !matches!(self.phase, Phase::Hidden)
    }

    /// 網頁掛載時主動拉取目前待播的轉場，避免事件在監聽註冊前遺失。
    pub fn pending_transition(&self) -> Option<Transition> {
        match &self.phase {
            Phase::Pending { transition, .. } => Some(transition.clone()),
            _ => None,
        }
    }

    pub fn request_show(&mut self, origin: (f64, f64, f64), source: &'static str, now: u64) -> Vec<Cmd> {
        match &self.phase {
            Phase::Shown { .. } => Vec::new(),
            Phase::Pending { transition, .. } => {
                if self.web_ready { vec![Cmd::Emit(transition.clone())] } else { Vec::new() }
            }
            Phase::Hidden => {
                let transition = Transition {
                    id: self.next_id,
                    origin_x: origin.0,
                    origin_y: origin.1,
                    origin_size: origin.2,
                    source,
                };
                self.next_id += 1;
                self.phase = Phase::Pending { transition: transition.clone(), requested_at: now };
                let mut cmds = vec![Cmd::ApplyFrame];
                if self.web_ready {
                    cmds.push(Cmd::Emit(transition));
                }
                cmds
            }
        }
    }

    pub fn web_ready(&mut self) -> Vec<Cmd> {
        self.web_ready = true;
        match &self.phase {
            Phase::Pending { transition, .. } => vec![Cmd::Emit(transition.clone())],
            _ => Vec::new(),
        }
    }

    pub fn mounted(&mut self, id: u64) -> Vec<Cmd> {
        match &self.phase {
            Phase::Pending { transition, .. } if transition.id == id => {
                self.phase = Phase::Shown { transition: transition.clone() };
                self.consecutive_timeouts = 0;
                vec![Cmd::ShowAndFocus]
            }
            _ => Vec::new(),
        }
    }

    pub fn escape_pressed(&mut self, now: u64) {
        if matches!(self.phase, Phase::Shown { .. }) && self.escape_at.is_none() {
            self.escape_at = Some(now);
        }
    }

    pub fn escape_acked(&mut self) {
        self.escape_at = None;
    }

    pub fn tick(&mut self, now: u64) -> Vec<Cmd> {
        if let Phase::Pending { requested_at, .. } = &self.phase
            && now.saturating_sub(*requested_at) >= MOUNT_TIMEOUT_MS
        {
            self.phase = Phase::Hidden;
            self.web_ready = false;
            self.escape_at = None;
            self.consecutive_timeouts += 1;
            let mut cmds = vec![Cmd::Hide, Cmd::ReloadWeb, Cmd::OpenMainFallback];
            if self.consecutive_timeouts == ALERT_AFTER_TIMEOUTS {
                cmds.push(Cmd::Alert(self.consecutive_timeouts));
            }
            return cmds;
        }
        if let (Phase::Shown { .. }, Some(at)) = (&self.phase, self.escape_at)
            && now.saturating_sub(at) >= ESCAPE_TIMEOUT_MS
        {
            self.phase = Phase::Hidden;
            self.escape_at = None;
            return vec![Cmd::NativeDismiss, Cmd::Hide];
        }
        Vec::new()
    }

    /// 網頁卡住（畫布佔住主執行緒，AC-EC-09b）：先隱藏再重新載入；之後要由呼叫端重新 `request_show`，
    /// 等重新載入的網頁回報就緒與掛載才再帶到前景（INV 22）。
    pub fn web_hung(&mut self) -> Vec<Cmd> {
        let was_open = self.session_open();
        self.phase = Phase::Hidden;
        self.web_ready = false;
        self.escape_at = None;
        if was_open { vec![Cmd::Hide, Cmd::ReloadWeb] } else { vec![Cmd::ReloadWeb] }
    }

    /// 卡住的復原（AC-EC-09b）：隱藏並重新載入；第二個值表示要不要重新呈現——只有卡住當下 session 還開著才要
    /// （Esc 原生後備、勿擾、kill-switch 已經收掉的不叫回）。卡住期間使用者按的 Esc（網頁沒確認、原生後備還沒到期）
    /// 不能被吞：照 Esc 原生後備完成 dismissal、隱藏，不重新呈現（CLAUDE.md §6）。
    pub fn hang_recovery(&mut self) -> (Vec<Cmd>, bool) {
        let open = self.session_open();
        let escape_pending = matches!(self.phase, Phase::Shown { .. }) && self.escape_at.is_some();
        let mut cmds = Vec::new();
        if escape_pending {
            cmds.push(Cmd::NativeDismiss);
        }
        cmds.extend(self.web_hung());
        (cmds, open && !escape_pending)
    }

    pub fn hide(&mut self) -> Vec<Cmd> {
        self.phase = Phase::Hidden;
        self.escape_at = None;
        vec![Cmd::Hide]
    }
}

/// 同一個蓋板 session 裡已經記過 dismissal 的題（AC-OP-06，2026-10-09 決策面板 QA 第 3 輪）：蓋板網頁的「稍後再問」
/// （經殼層的 `dismiss_question`）與原生 Esc 後備共用這本帳、排隊執行，同一題在同一個 session 只記一次——網頁的
/// dismissal 超過 2 秒才成功、或部分失敗交給原生後備時，都不會重複記。值是記完之後那一題的週期編號。
#[derive(Debug, Default)]
pub struct DismissLedger {
    session: Option<u64>,
    done: HashMap<String, u32>,
}

impl DismissLedger {
    /// 換到另一個蓋板 session（新轉場）時清空；同一個 session 重複呼叫不清。
    pub fn begin(&mut self, session: u64) {
        if self.session != Some(session) {
            self.session = Some(session);
            self.done.clear();
        }
    }

    /// 記下這一題已記過 dismissal（`cycle_after`＝記完之後的週期編號）。
    pub fn record(&mut self, id: &str, cycle_after: u32) {
        self.done.insert(id.to_string(), cycle_after);
    }

    /// 這一題（目前的週期編號、是不是強制）在這個 session 是否已經記過、而且之後沒有再到期。又到期＝又是強制、週期編號
    /// ≥ 記錄的——那是新的一次強制，照樣記。讀到的資料比記錄舊（週期編號較小）也算已記過。
    pub fn already_dismissed(&self, id: &str, cycle: u32, forced: bool) -> bool {
        self.done.get(id).is_some_and(|&after| !(forced && cycle >= after))
    }
}

/// 原生 dismissal 的候選題（`dismiss_targets` 的輸入；只放判斷需要的欄位，方便表驅動測試）。
#[derive(Debug, Clone, Copy)]
pub struct DismissCandidate<'a> {
    pub id: &'a str,
    pub forced: bool,
    pub terminal: bool,
    pub internal: bool,
}

/// 原生層完成的 dismissal（Esc 2 秒後備、畫布卡住期間的 Esc）要記哪些題（AC-OP-06、AC-EC-09b）：與網頁「稍後再問」
/// 同一個範圍——所有非 internal 的強制題，加上網頁最後回報的目前題目（非終態、非 internal、還在清單裡）；每題一次。
/// 只處理最早逾時的一題會讓剩下的強制題又把蓋板叫回來。
pub fn dismiss_targets(open: &[DismissCandidate<'_>], current: Option<&str>) -> Vec<String> {
    let mut out: Vec<String> = open
        .iter()
        .filter(|c| c.forced && !c.terminal && !c.internal)
        .map(|c| c.id.to_string())
        .collect();
    if let Some(id) = current
        && open.iter().any(|c| c.id == id && !c.terminal && !c.internal)
        && !out.iter().any(|known| known == id)
    {
        out.push(id.to_string());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORIGIN: (f64, f64, f64) = (16.0, 61.0, 18.0);

    fn pending_id(p: &Presenter) -> u64 {
        match p.phase() {
            Phase::Pending { transition, .. } => transition.id,
            other => panic!("預期 Pending，實際 {other:?}"),
        }
    }

    /// INV 22：窮舉所有長度 ≤6 的事件序列，ShowAndFocus 只會出現在「目前這一次轉場已送給網頁、網頁也回報
    /// 那一個 id 已渲染」之後。網頁只能回報它收到過的 id（含前一次 session 的過期 id）；未就緒、過期 id、錯的 id、
    /// 逾時、Esc、隱藏、卡住復原（含重新呈現）的任何順序都不會把視窗帶到前景。
    /// 同時檢查 Esc 永遠有效（CLAUDE.md §6、AC-EC-09b）：網頁沒確認的 Esc，不論之後是 Esc 逾時還是畫布卡住，都恰好
    /// 完成一次原生 dismissal，卡住復原不會把它叫回來；沒有待處理的 Esc 就不會憑空 dismissal。
    #[test]
    fn inv_overlay_never_blank() {
        #[derive(Clone, Copy, Debug)]
        enum Ev {
            Show,
            Ready,
            /// 網頁回報它最後收到的 id（可能是過期的）。
            MountLatest,
            /// 網頁回報它最早收到的 id（前一次 session）。
            MountOldest,
            Timeout,
            Escape,
            /// 網頁確認自己處理了 Esc。
            EscAck,
            Hide,
            /// 畫布卡住主執行緒，殼層的看門狗判定網頁沒有回應（AC-EC-09b）：卡住復原，需要時重新呈現。
            Hung,
        }
        const EVS: [Ev; 9] = [
            Ev::Show,
            Ev::Ready,
            Ev::MountLatest,
            Ev::MountOldest,
            Ev::Timeout,
            Ev::Escape,
            Ev::EscAck,
            Ev::Hide,
            Ev::Hung,
        ];
        fn run(seq: &[Ev]) {
            let mut p = Presenter::new();
            let mut received: Vec<u64> = Vec::new();
            let mut now = 0u64;
            // 參考模型：蓋板顯示中按下、網頁還沒確認、原生層也還沒完成的 Esc。
            let mut esc_pending = false;
            for ev in seq {
                now += 10;
                let pending = match p.phase() {
                    Phase::Pending { transition, .. } => Some(transition.id),
                    _ => None,
                };
                let shown = matches!(p.phase(), Phase::Shown { .. });
                let esc_before = esc_pending;
                let (acked, cmds) = match ev {
                    Ev::Show => (None, p.request_show(ORIGIN, "notch", now)),
                    Ev::Ready => (None, p.web_ready()),
                    Ev::MountLatest => match received.last().copied() {
                        Some(id) => (Some(id), p.mounted(id)),
                        None => (None, Vec::new()),
                    },
                    Ev::MountOldest => match received.first().copied() {
                        Some(id) => (Some(id), p.mounted(id)),
                        None => (None, Vec::new()),
                    },
                    Ev::Timeout => {
                        now += MOUNT_TIMEOUT_MS + ESCAPE_TIMEOUT_MS;
                        (None, p.tick(now))
                    }
                    Ev::Escape => {
                        p.escape_pressed(now);
                        esc_pending |= shown;
                        (None, Vec::new())
                    }
                    Ev::EscAck => {
                        p.escape_acked();
                        esc_pending = false;
                        (None, Vec::new())
                    }
                    Ev::Hide => {
                        esc_pending = false;
                        (None, p.hide())
                    }
                    Ev::Hung => {
                        // windows.rs：卡住復原後，只有要重新呈現時才再 request_show。
                        let (mut cmds, represent) = p.hang_recovery();
                        assert!(
                            !(esc_before && represent),
                            "卡住期間按的 Esc 被吞掉，蓋板還被叫回來：{seq:?}"
                        );
                        if represent {
                            cmds.extend(p.request_show(ORIGIN, "canvas_hang", now));
                        }
                        esc_pending = false;
                        (None, cmds)
                    }
                };
                if matches!(ev, Ev::Timeout) {
                    esc_pending = false;
                }
                if matches!(ev, Ev::Timeout | Ev::Hung) {
                    let dismissed = cmds.iter().filter(|c| matches!(c, Cmd::NativeDismiss)).count();
                    assert_eq!(
                        dismissed,
                        usize::from(esc_before),
                        "待處理的 Esc 要恰好完成一次原生 dismissal，沒有就不能 dismissal：{seq:?}（{cmds:?}）"
                    );
                } else {
                    assert!(!cmds.contains(&Cmd::NativeDismiss), "{seq:?}");
                }
                for cmd in &cmds {
                    match cmd {
                        Cmd::Emit(t) => received.push(t.id),
                        Cmd::ShowAndFocus => {
                            assert!(
                                acked.is_some() && acked == pending,
                                "顯示的不是目前這一次轉場：{seq:?}（acked={acked:?}, pending={pending:?}）"
                            );
                        }
                        _ => {}
                    }
                }
            }
        }
        fn walk(prefix: &mut Vec<Ev>, depth: usize) {
            run(prefix);
            if depth == 0 {
                return;
            }
            for ev in EVS {
                prefix.push(ev);
                walk(prefix, depth - 1);
                prefix.pop();
            }
        }
        walk(&mut Vec::new(), 6);
    }

    #[test]
    fn show_before_web_ready_never_orders_the_window_front() {
        let mut p = Presenter::new();
        let cmds = p.request_show(ORIGIN, "notch", 0);
        assert_eq!(cmds, vec![Cmd::ApplyFrame]);
        assert!(p.session_open());
        assert!(!cmds.contains(&Cmd::ShowAndFocus));
    }

    #[test]
    fn web_ready_delivers_the_pending_transition_and_pull_returns_it() {
        let mut p = Presenter::new();
        p.request_show(ORIGIN, "system", 0);
        let id = pending_id(&p);
        assert_eq!(p.pending_transition().map(|t| t.id), Some(id));
        let cmds = p.web_ready();
        assert!(matches!(cmds.as_slice(), [Cmd::Emit(t)] if t.id == id));
    }

    #[test]
    fn show_after_web_ready_emits_immediately() {
        let mut p = Presenter::new();
        assert!(p.web_ready().is_empty());
        let cmds = p.request_show(ORIGIN, "notch", 0);
        assert!(matches!(cmds.as_slice(), [Cmd::ApplyFrame, Cmd::Emit(_)]));
    }

    #[test]
    fn only_the_matching_mount_ack_shows_the_window() {
        let mut p = Presenter::new();
        p.web_ready();
        p.request_show(ORIGIN, "notch", 0);
        let id = pending_id(&p);
        assert!(p.mounted(id + 99).is_empty());
        assert_eq!(p.mounted(id), vec![Cmd::ShowAndFocus]);
        assert!(matches!(p.phase(), Phase::Shown { .. }));
        assert!(p.pending_transition().is_none());
    }

    #[test]
    fn repeated_show_while_pending_resends_the_same_transition() {
        let mut p = Presenter::new();
        p.web_ready();
        p.request_show(ORIGIN, "system", 0);
        let id = pending_id(&p);
        let cmds = p.request_show(ORIGIN, "system", 300);
        assert!(matches!(cmds.as_slice(), [Cmd::Emit(t)] if t.id == id), "{cmds:?}");
    }

    #[test]
    fn repeated_show_while_shown_is_a_no_op() {
        let mut p = Presenter::new();
        p.web_ready();
        p.request_show(ORIGIN, "system", 0);
        let id = pending_id(&p);
        p.mounted(id);
        assert!(p.request_show(ORIGIN, "system", 500).is_empty());
    }

    #[test]
    fn mount_timeout_falls_back_to_main_window_and_reloads_web() {
        let mut p = Presenter::new();
        p.web_ready();
        p.request_show(ORIGIN, "system", 0);
        assert!(p.tick(MOUNT_TIMEOUT_MS - 1).is_empty());
        let cmds = p.tick(MOUNT_TIMEOUT_MS);
        assert_eq!(cmds, vec![Cmd::Hide, Cmd::ReloadWeb, Cmd::OpenMainFallback]);
        assert_eq!(p.phase(), &Phase::Hidden);
        assert!(!p.session_open());
        // 重載後網頁未就緒：下一次顯示請求不能直接送事件。
        assert_eq!(p.request_show(ORIGIN, "system", 2_000), vec![Cmd::ApplyFrame]);
    }

    #[test]
    fn third_consecutive_timeout_raises_an_alert_and_mount_resets_the_count() {
        let mut p = Presenter::new();
        let mut now = 0;
        for round in 1..=3 {
            p.web_ready();
            p.request_show(ORIGIN, "system", now);
            now += MOUNT_TIMEOUT_MS;
            let cmds = p.tick(now);
            assert_eq!(cmds.contains(&Cmd::Alert(3)), round == 3, "round {round}: {cmds:?}");
            now += 10;
        }
        p.web_ready();
        p.request_show(ORIGIN, "system", now);
        let id = pending_id(&p);
        p.mounted(id);
        p.hide();
        p.web_ready();
        p.request_show(ORIGIN, "system", now + 10);
        let cmds = p.tick(now + 10 + MOUNT_TIMEOUT_MS);
        assert!(!cmds.iter().any(|cmd| matches!(cmd, Cmd::Alert(_))), "掛載成功後計數歸零");
    }

    #[test]
    fn unanswered_escape_is_completed_natively() {
        let mut p = Presenter::new();
        p.web_ready();
        p.request_show(ORIGIN, "system", 0);
        let id = pending_id(&p);
        p.mounted(id);
        p.escape_pressed(1_000);
        assert!(p.tick(1_000 + ESCAPE_TIMEOUT_MS - 1).is_empty());
        assert_eq!(p.tick(1_000 + ESCAPE_TIMEOUT_MS), vec![Cmd::NativeDismiss, Cmd::Hide]);
        assert_eq!(p.phase(), &Phase::Hidden);
    }

    #[test]
    fn acknowledged_escape_is_left_to_the_web() {
        let mut p = Presenter::new();
        p.web_ready();
        p.request_show(ORIGIN, "system", 0);
        let id = pending_id(&p);
        p.mounted(id);
        p.escape_pressed(1_000);
        p.escape_acked();
        assert!(p.tick(10_000).is_empty());
    }

    #[test]
    fn escape_while_hidden_does_nothing() {
        let mut p = Presenter::new();
        p.escape_pressed(0);
        assert!(p.tick(ESCAPE_TIMEOUT_MS * 2).is_empty());
    }

    #[test]
    fn hide_clears_everything_and_transition_ids_increase() {
        let mut p = Presenter::new();
        p.web_ready();
        p.request_show(ORIGIN, "system", 0);
        let first = pending_id(&p);
        assert_eq!(p.hide(), vec![Cmd::Hide]);
        assert!(!p.session_open());
        assert!(p.tick(MOUNT_TIMEOUT_MS * 2).is_empty(), "隱藏後不再逾時");
        p.request_show(ORIGIN, "system", 5_000);
        assert!(pending_id(&p) > first);
    }

    /// AC-EC-09b：網頁卡住 → 先隱藏再重新載入；重新載入完成、目前這一次轉場掛載後才再帶到前景（不留空白蓋板）。
    #[test]
    fn hung_web_hides_reloads_and_presents_again() {
        let mut p = Presenter::new();
        p.web_ready();
        p.request_show(ORIGIN, "notch", 0);
        let old = pending_id(&p);
        assert_eq!(p.mounted(old), vec![Cmd::ShowAndFocus]);

        assert_eq!(p.web_hung(), vec![Cmd::Hide, Cmd::ReloadWeb]);
        assert!(!p.session_open());
        let cmds = p.request_show(ORIGIN, "canvas_hang", 10);
        assert!(
            !cmds.iter().any(|c| matches!(c, Cmd::Emit(_) | Cmd::ShowAndFocus)),
            "網頁重新載入完成前不送轉場、不顯示：{cmds:?}"
        );
        let fresh = pending_id(&p);
        assert!(matches!(p.web_ready().as_slice(), [Cmd::Emit(t)] if t.id == fresh));
        assert_eq!(p.mounted(old), Vec::<Cmd>::new(), "卡住前的轉場 id 不算");
        assert_eq!(p.mounted(fresh), vec![Cmd::ShowAndFocus]);
    }

    #[test]
    fn hung_while_hidden_only_reloads() {
        let mut p = Presenter::new();
        p.web_ready();
        assert_eq!(p.web_hung(), vec![Cmd::ReloadWeb]);
        assert!(p.request_show(ORIGIN, "notch", 0).iter().all(|c| !matches!(c, Cmd::Emit(_))));
    }

    /// AC-EC-09b：卡住後只在當下蓋板 session 還開著時重新呈現；Esc 後備、勿擾、kill-switch 收掉的不叫回。
    #[test]
    fn hang_recovery_only_represents_an_open_session() {
        let mut p = Presenter::new();
        p.web_ready();
        p.request_show(ORIGIN, "notch", 0);
        let id = pending_id(&p);
        p.mounted(id);
        assert_eq!(p.hang_recovery(), (vec![Cmd::Hide, Cmd::ReloadWeb], true));

        let mut esc = Presenter::new();
        esc.web_ready();
        esc.request_show(ORIGIN, "notch", 0);
        let id = pending_id(&esc);
        esc.mounted(id);
        esc.escape_pressed(1_000);
        assert_eq!(esc.tick(3_000), vec![Cmd::NativeDismiss, Cmd::Hide]);
        assert_eq!(esc.hang_recovery(), (vec![Cmd::ReloadWeb], false), "Esc 已收掉");

        let mut dnd = Presenter::new();
        dnd.web_ready();
        dnd.request_show(ORIGIN, "notch", 0);
        dnd.hide();
        assert_eq!(dnd.hang_recovery(), (vec![Cmd::ReloadWeb], false), "勿擾／kill-switch 已收掉");
    }

    /// 證據畫布第 5 輪 QA（AC-EC-09b、AC-OP-06）：原生完成的 dismissal 與網頁「稍後再問」同一個範圍——所有非 internal
    /// 的強制題，加上網頁最後回報的目前題目（非終態、非 internal）；每題一次、不重複。舊版只處理最早逾時的一題：剩下的
    /// 強制題又把蓋板叫回來，目前題目不是強制題時也不記 dismissal。
    #[test]
    fn native_dismissal_matches_the_web_later_scope() {
        let c = |id: &'static str, forced: bool, terminal: bool, internal: bool| DismissCandidate { id, forced, terminal, internal };
        let open = [
            c("q-forced-1", true, false, false),
            c("q-forced-2", true, false, false),
            c("q-current", false, false, false),
            c("q-pending", false, false, false),
            c("q-doctor", true, false, true),
            c("q-done", false, true, false),
        ];
        assert_eq!(dismiss_targets(&open, Some("q-current")), vec!["q-forced-1", "q-forced-2", "q-current"]);
        assert_eq!(dismiss_targets(&open, Some("q-forced-2")), vec!["q-forced-1", "q-forced-2"], "不重複");
        assert_eq!(dismiss_targets(&open, None), vec!["q-forced-1", "q-forced-2"]);
        assert_eq!(dismiss_targets(&open, Some("q-done")), vec!["q-forced-1", "q-forced-2"], "已結案的目前題目不算");
        assert_eq!(dismiss_targets(&open, Some("q-doctor")), vec!["q-forced-1", "q-forced-2"], "internal 不算");
        assert_eq!(dismiss_targets(&open, Some("q-gone")), vec!["q-forced-1", "q-forced-2"], "清單裡找不到的不算");
    }

    /// QA 第 4 輪（AC-EC-09b、CLAUDE.md §6）：畫布卡住後使用者按 Esc（網頁沒反應，原生 2 秒後備還沒到期），看門狗先判定
    /// 卡住。這個 Esc 不能被吞：照樣完成原生 dismissal、隱藏，而且不重新呈現。
    #[test]
    fn escape_pressed_during_hang_is_completed_not_swallowed() {
        let mut p = Presenter::new();
        p.web_ready();
        p.request_show(ORIGIN, "notch", 0);
        let id = pending_id(&p);
        p.mounted(id);
        p.escape_pressed(2_500);
        assert!(p.tick(2_600).is_empty(), "原生 Esc 後備還沒到期");
        assert_eq!(p.hang_recovery(), (vec![Cmd::NativeDismiss, Cmd::Hide, Cmd::ReloadWeb], false));
        assert!(!p.session_open());
        assert!(p.tick(10_000).is_empty(), "Esc 只完成一次，不會之後再 dismissal 一次");

        // 網頁已確認處理了 Esc（它自己會關）：照一般卡住復原，殼層不重複 dismissal。
        let mut acked = Presenter::new();
        acked.web_ready();
        acked.request_show(ORIGIN, "notch", 0);
        let id = pending_id(&acked);
        acked.mounted(id);
        acked.escape_pressed(2_500);
        acked.escape_acked();
        let (cmds, _) = acked.hang_recovery();
        assert!(!cmds.contains(&Cmd::NativeDismiss));
    }

    /// 決策面板 QA 第 3 輪（AC-OP-06）：同一個蓋板 session 每題只記一次 dismissal。網頁「稍後再問」已記過的題（記完週期 3），
    /// 原生後備用剛讀到的資料（週期 3、已不是強制）判斷時跳過；網頁沒記過的強制題照樣記。舊版原生後備只看「強制題＋目前
    /// 題目」，網頁已經記過的目前題目再記一次（`cycle_number`、`snooze_count` 各 +2）。
    #[test]
    fn native_fallback_skips_questions_the_web_already_dismissed() {
        let mut ledger = DismissLedger::default();
        ledger.begin(7);
        ledger.record("q-current", 3);
        assert!(ledger.already_dismissed("q-current", 3, false), "網頁已記過、之後沒再到期");
        assert!(!ledger.already_dismissed("q-forced-2", 1, true), "網頁沒記過的強制題照樣記");
    }

    /// 網頁的 dismissal 超過 2 秒才成功（AC-OP-06）：兩條路排隊，先記的那一條記帳，晚到的那一條跳過——不論誰先。讀到的資料
    /// 若還是記之前的（週期 1、強制），也不會再記一次。
    #[test]
    fn web_dismissal_slower_than_the_escape_fallback_is_not_repeated() {
        let mut ledger = DismissLedger::default();
        ledger.begin(7);
        // 原生後備先記了 q-forced-2（記完週期 2），網頁晚到的那一次讀到最新資料（週期 2、pending）。
        ledger.record("q-forced-2", 2);
        assert!(ledger.already_dismissed("q-forced-2", 2, false));
        // 原生後備手上的清單是記之前讀的（週期 1、強制）：同樣跳過。
        assert!(ledger.already_dismissed("q-forced-2", 1, true));
    }

    /// 同一個 session 裡記過的題 15 分鐘後又到期（又是強制、週期編號 ≥ 記錄的）：那是新的一次強制，照樣記（AC-OP-06）。
    #[test]
    fn a_question_due_again_in_the_same_session_is_dismissed_again() {
        let mut ledger = DismissLedger::default();
        ledger.begin(7);
        ledger.record("q", 2);
        assert!(!ledger.already_dismissed("q", 2, true));
        assert!(!ledger.already_dismissed("q", 3, true));
    }

    /// 帳本只管同一個蓋板 session：新轉場開始就清空；蓋板收起後還是同一個 session（晚到的 dismissal 照樣去重）。
    #[test]
    fn a_new_session_forgets_earlier_dismissals() {
        let mut p = Presenter::new();
        assert_eq!(p.last_transition_id(), 0, "還沒有任何轉場");
        p.request_show(ORIGIN, "notch", 0);
        assert_eq!(p.last_transition_id(), 1);
        p.hide();
        assert_eq!(p.last_transition_id(), 1, "收起後仍是同一個 session");

        let mut ledger = DismissLedger::default();
        ledger.begin(p.last_transition_id());
        ledger.record("q", 2);
        ledger.begin(p.last_transition_id());
        assert!(ledger.already_dismissed("q", 2, false), "同一個 session 不清");
        p.request_show(ORIGIN, "notch", 10);
        ledger.begin(p.last_transition_id());
        assert!(!ledger.already_dismissed("q", 2, false), "新 session 不算之前記的");
    }
}
