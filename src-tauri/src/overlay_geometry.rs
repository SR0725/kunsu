//! 強制蓋板的目標螢幕與 frame（docs/bdd/overlay-presentation.md AC-OP-01／02／03）。
//!
//! 全部採 Cocoa 全域座標：單位是點（point）、左下原點、`screens[0]` 是主螢幕。
//! 直接用 NSScreen.frame 設定蓋板 frame，不經 Tauri 的實體／邏輯換算，所以混用 2x
//! 內建螢幕與 1x 外接螢幕時不會放大縮小；也不再「先設位置、再設大小」，避免
//! `setContentSize` 以左下角為錨往上長高（2026-10-08 根因）。純函數，任何平台可測。

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub const fn new(x: f64, y: f64, w: f64, h: f64) -> Self {
        Self { x, y, w, h }
    }

    pub fn contains(&self, px: f64, py: f64) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }

    pub fn intersection_area(&self, other: &Rect) -> f64 {
        let w = (self.x + self.w).min(other.x + other.w) - self.x.max(other.x);
        let h = (self.y + self.h).min(other.y + other.h) - self.y.max(other.y);
        if w <= 0.0 || h <= 0.0 { 0.0 } else { w * h }
    }
}

/// AC-OP-02：瀏海（現行膠囊）所在螢幕 → 游標所在螢幕 → 主螢幕。
/// 錨點視窗跨兩個螢幕時取重疊面積最大者。
pub fn target_screen(
    screens: &[Rect],
    anchor_window: Option<Rect>,
    mouse: Option<(f64, f64)>,
) -> Option<usize> {
    if screens.is_empty() {
        return None;
    }
    if let Some(window) = anchor_window {
        let best = screens
            .iter()
            .enumerate()
            .map(|(index, screen)| (index, screen.intersection_area(&window)))
            .filter(|(_, area)| *area > 0.0)
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        if let Some((index, _)) = best {
            return Some(index);
        }
    }
    if let Some((mx, my)) = mouse
        && let Some(index) = screens.iter().position(|screen| screen.contains(mx, my))
    {
        return Some(index);
    }
    Some(0)
}

/// AC-OP-01：讀回的實際 frame 與目標誤差不超過 0.5 pt 才算對齊。
pub fn frame_matches(actual: Rect, target: Rect) -> bool {
    (actual.x - target.x).abs() <= 0.5
        && (actual.y - target.y).abs() <= 0.5
        && (actual.w - target.w).abs() <= 0.5
        && (actual.h - target.h).abs() <= 0.5
}

/// 沒有點擊、瀏海也還沒回報過圖示位置時的估計起點（舞台本地 CSS 點與圖示大小）。
pub const FALLBACK_ANCHOR: (f64, f64, f64) = (25.5, 27.0, 25.0);

/// AC-OP-02：轉場起點（舞台本地 CSS 點與圖示大小）——點擊的位置優先；沒有點擊（題目自動到期、tray、批次窗）用瀏海
/// 最後回報的圖示中心；都沒有才用估計值。
pub fn overlay_origin(click: Option<(f64, f64, f64)>, reported: Option<(f64, f64, f64)>) -> (f64, f64, f64) {
    click.or(reported).unwrap_or(FALLBACK_ANCHOR)
}

/// 網頁回報的圖示中心只收有限值、合理的大小（與 `open_overlay_from_capsule` 的檢查相同）。
pub fn valid_anchor(x: f64, y: f64, size: f64) -> bool {
    x.is_finite() && y.is_finite() && size.is_finite() && (8.0..=96.0).contains(&size)
}

/// 錨點視窗內的 CSS 點（左上原點）→ 蓋板本地 CSS 點（左上原點）。
/// 兩者都是點，與螢幕倍率無關。
pub fn anchor_in_overlay(anchor_window: Rect, local_x: f64, local_y: f64, screen: Rect) -> (f64, f64) {
    let global_x = anchor_window.x + local_x;
    let global_y = anchor_window.y + anchor_window.h - local_y;
    (global_x - screen.x, screen.y + screen.h - global_y)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 本機 2026-10-08 實測配置：內建 1728×1117@2x 在原點；BenQ 1x 在正上方；另一台在左方。
    fn ray_displays() -> Vec<Rect> {
        vec![
            Rect::new(0.0, 0.0, 1728.0, 1117.0),
            Rect::new(-176.0, 1117.0, 2560.0, 1440.0),
            Rect::new(-1920.0, 0.0, 1920.0, 1080.0),
        ]
    }

    /// 舊流程的模型：Tauri `set_position`（以當下舊高度把左上角對齊螢幕頂）→
    /// `setContentSize`（固定左下角往上長）。
    fn legacy_position_then_size(old: Rect, screen: Rect) -> Rect {
        let after_position = Rect::new(screen.x, screen.y + screen.h - old.h, old.w, old.h);
        Rect::new(after_position.x, after_position.y, screen.w, screen.h)
    }

    #[test]
    fn legacy_order_reproduces_the_reported_offset() {
        let screen = ray_displays()[0];
        let result = legacy_position_then_size(Rect::new(0.0, 0.0, 1280.0, 800.0), screen);
        let overflow_top = result.y + result.h - (screen.y + screen.h);
        assert_eq!(overflow_top, 317.0, "上緣超出螢幕 317 pt");
        assert!(((result.y - screen.y) / screen.h - 0.284).abs() < 0.001, "下方 28.4% 未覆蓋");
        assert!(!frame_matches(result, screen));
    }

    /// INV 23：不論膠囊或游標在哪（含跨螢幕、螢幕外），蓋板目標都是某一整個螢幕，且只有完全一致（≤0.5 pt）才算覆蓋；
    /// 舊流程「先移位置再改大小」的結果一律判為不合格。
    #[test]
    fn inv_overlay_covers_target_display() {
        let screens = ray_displays();
        let mut checked = 0;
        for ax in (-2400..=2800).step_by(400) {
            for ay in (-200..=2700).step_by(300) {
                let anchor = Rect::new(ax as f64, ay as f64, 32.0, 160.0);
                for mouse in [None, Some((ax as f64 + 5.0, ay as f64 + 5.0)), Some((-99999.0, -99999.0))] {
                    let index = target_screen(&screens, Some(anchor), mouse).expect("永遠選得到螢幕");
                    let target = screens[index];
                    assert!(frame_matches(target, target));
                    let center = (anchor.x + anchor.w / 2.0, anchor.y + anchor.h / 2.0);
                    if let Some(home) = screens.iter().position(|s| s.contains(center.0, center.1)) {
                        assert_eq!(index, home, "膠囊在哪個螢幕就蓋哪個螢幕");
                    }
                    for (dx, dy) in [(0.6, 0.0), (0.0, -0.6), (0.0, 317.0)] {
                        let shifted = Rect::new(target.x + dx, target.y + dy, target.w, target.h);
                        assert!(!frame_matches(shifted, target), "偏移 {dx},{dy} 不能算覆蓋");
                    }
                    let legacy = legacy_position_then_size(Rect::new(0.0, 0.0, 1280.0, 800.0), target);
                    assert!(!frame_matches(legacy, target) || target.h == 800.0);
                    checked += 1;
                }
            }
        }
        assert!(checked > 300);
    }

    #[test]
    fn target_frame_is_the_whole_display_regardless_of_previous_size() {
        let screens = ray_displays();
        for anchor in [None, Some(Rect::new(0.0, 900.0, 32.0, 160.0))] {
            let index = target_screen(&screens, anchor, None).expect("有螢幕");
            assert!(frame_matches(screens[index], screens[0]));
        }
    }

    #[test]
    fn anchor_window_screen_wins_then_mouse_then_primary() {
        let screens = ray_displays();
        let on_top_display = Rect::new(100.0, 2000.0, 32.0, 160.0);
        assert_eq!(target_screen(&screens, Some(on_top_display), Some((10.0, 10.0))), Some(1));
        assert_eq!(target_screen(&screens, None, Some((-500.0, 400.0))), Some(2));
        assert_eq!(target_screen(&screens, None, Some((99999.0, 99999.0))), Some(0));
        assert_eq!(target_screen(&screens, None, None), Some(0));
        assert_eq!(target_screen(&[], None, None), None);
    }

    #[test]
    fn straddling_anchor_uses_the_screen_with_the_largest_overlap() {
        let screens = ray_displays();
        let straddle = Rect::new(-10.0, 400.0, 40.0, 100.0);
        assert_eq!(target_screen(&screens, Some(straddle), None), Some(0));
    }

    #[test]
    fn frame_tolerance_is_half_a_point() {
        let target = Rect::new(0.0, 0.0, 1728.0, 1117.0);
        assert!(frame_matches(Rect::new(0.4, -0.4, 1728.5, 1116.5), target));
        assert!(!frame_matches(Rect::new(0.0, 0.0, 1728.0, 1116.0), target));
    }

    #[test]
    fn anchor_maps_into_overlay_points_on_any_display() {
        let screens = ray_displays();
        let notch = Rect::new(0.0, 1117.0 - 45.0 - 160.0, 32.0, 160.0);
        assert_eq!(anchor_in_overlay(notch, 16.0, 16.0, screens[0]), (16.0, 61.0));
        let on_top = Rect::new(-176.0, 1117.0 + 1440.0 - 200.0, 32.0, 200.0);
        assert_eq!(anchor_in_overlay(on_top, 16.0, 16.0, screens[1]), (16.0, 16.0));
    }

    /// AC-OP-02（決策面板 QA 第 2 輪）：題目自動到期、tray 開啟沒有點擊座標時，起點用瀏海最後回報的圖示中心；
    /// 舊版固定在 (25.5, 27)，瀏海在右邊或下邊時面板從左上角張開。
    #[test]
    fn origin_prefers_click_then_last_reported_notch_mark() {
        let click = Some((10.0, 300.0, 18.0));
        let reported = Some((1899.0, 1109.0, 18.0));
        assert_eq!(overlay_origin(click, reported), (10.0, 300.0, 18.0), "點擊永遠優先");
        assert_eq!(overlay_origin(None, reported), (1899.0, 1109.0, 18.0), "沒有點擊用最後回報");
        assert_eq!(overlay_origin(None, None), FALLBACK_ANCHOR, "都沒有才用估計值");

        // 換算到蓋板座標後落在瀏海所在的右下角，不是左上角。
        let screen = Rect::new(0.0, 0.0, 1920.0, 1117.0);
        let stage = screen; // 瀏海舞台貼滿整個螢幕
        let (lx, ly, _) = overlay_origin(None, reported);
        let (x, y) = anchor_in_overlay(stage, lx, ly, screen);
        assert!(x > 1800.0 && y > 1000.0, "({x}, {y})");
    }

    /// `edge_anchor` 只收有限值、合理的圖示大小（同 `open_overlay_from_capsule` 的檢查）。
    #[test]
    fn reported_anchor_must_be_finite_and_sane() {
        assert!(valid_anchor(10.0, 20.0, 18.0));
        assert!(!valid_anchor(f64::NAN, 20.0, 18.0));
        assert!(!valid_anchor(10.0, f64::INFINITY, 18.0));
        assert!(!valid_anchor(10.0, 20.0, 4.0));
        assert!(!valid_anchor(10.0, 20.0, 200.0));
    }
}
