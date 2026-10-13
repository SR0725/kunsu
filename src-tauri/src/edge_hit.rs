//! 邊緣瀏海的命中判斷（PRD §19、BDD edge-notch 場景 11b／14／19；INV `inv_edge_notch_click_through`）。
//!
//! 舞台視窗貼滿螢幕、預設穿透；原生層輪詢游標，只有游標在網頁回報的命中區內（或在瀏海上按下、正在拖曳）才接收滑鼠事件。
//! 命中區就是畫出來的多邊形（與 SVG 填色同一組點，不加容差）：看得到的黑色都點得到，看不到的地方不攔截；網頁沒畫出瀏海時
//! 回報空命中區。純函數：輸入游標位置、命中區、接管狀態與滑鼠鍵狀態，輸出是否穿透、是否結束拖曳、下次輪詢間隔。

use serde::Deserialize;

/// 網頁回報的命中多邊形（舞台本地 CSS px，左上為原點）：扁平 `[x0, y0, x1, y1, …]`。
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(from = "Vec<f64>")]
pub struct HitShape {
    pts: Vec<(f64, f64)>,
    min: (f64, f64),
    max: (f64, f64),
}

impl From<Vec<f64>> for HitShape {
    fn from(flat: Vec<f64>) -> Self {
        let pts: Vec<(f64, f64)> =
            flat.as_chunks::<2>().0.iter().map(|&[x, y]| (x, y)).filter(|p| p.0.is_finite() && p.1.is_finite()).collect();
        let mut min = (f64::INFINITY, f64::INFINITY);
        let mut max = (f64::NEG_INFINITY, f64::NEG_INFINITY);
        for &(x, y) in &pts {
            min = (min.0.min(x), min.1.min(y));
            max = (max.0.max(x), max.1.max(y));
        }
        HitShape { pts, min, max }
    }
}

impl HitShape {
    /// even-odd 點在多邊形內（與網頁測試同一規則）。少於三點的多邊形不攔截任何東西。
    pub fn contains(&self, (x, y): (f64, f64)) -> bool {
        let n = self.pts.len();
        if n < 3 {
            return false;
        }
        let mut c = false;
        let mut j = n - 1;
        for i in 0..n {
            let (xi, yi) = self.pts[i];
            let (xj, yj) = self.pts[j];
            if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
                c = !c;
            }
            j = i;
        }
        c
    }

    /// 游標到外接矩形的直線距離（在矩形內為 0；不比到多邊形本身近，用來決定輪詢頻率是保守的）。
    fn bbox_distance(&self, (x, y): (f64, f64)) -> f64 {
        if self.pts.len() < 3 {
            return f64::INFINITY;
        }
        let dx = (self.min.0 - x).max(0.0).max(x - self.max.0);
        let dy = (self.min.1 - y).max(0.0).max(y - self.max.1);
        dx.hypot(dy)
    }
}

/// 假設游標最快 3000 px/s：輪詢間隔＝距離 ÷ 這個速度，游標在抵達瀏海前一定已切換成可點。
pub const POINTER_MAX_SPEED_PX_PER_MS: f64 = 3.0;
/// 命中區內、拖曳中與很近時的輪詢間隔（約 60 Hz）。
pub const POLL_MIN_MS: u64 = 16;
/// 最慢的輪詢間隔（750 px 以外；4 Hz）。
pub const POLL_MAX_MS: u64 = 250;
/// 游標停在命中區內時，每隔這麼久重送一次「在瀏海上」。
pub const HEARTBEAT_MS: u64 = 1000;

/// 暫時接管全螢幕指標事件的理由（場景 14）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Grab {
    /// 網頁回報正在拖曳。
    pub dragging: bool,
    /// 原生按下鎖定：在瀏海上按下、滑鼠鍵還沒放開（[`press_latch`]），不等網頁回報開始拖曳。
    pub pressed: bool,
}

impl Grab {
    pub const NONE: Grab = Grab { dragging: false, pressed: false };
    #[cfg(test)]
    pub const DRAG: Grab = Grab { dragging: true, pressed: false };
    #[cfg(test)]
    pub const PRESS: Grab = Grab { dragging: false, pressed: true };
}

/// 原生按下鎖定（場景 14）：輪詢先看到滑鼠鍵放開（`was_down` 為 false）、再看到按下時游標在命中區內，就鎖定到滑鼠鍵全部
/// 放開；已鎖定就維持。在別處按下後拖進瀏海不算（那次按下不是在瀏海上）。
pub fn press_latch(held: bool, was_down: bool, buttons_down: bool, inside: bool) -> bool {
    buttons_down && (held || (!was_down && inside))
}

/// 網頁回報命中區後的接管狀態（INV 24）：命中區被清空（瀏海不再畫出，例如拖曳中所有題目在別處結案）時，網頁回報的拖曳
/// 與原生按下鎖定一律解除——之後瀏海再出現也不帶著舊拖曳接管別的 app。有命中區時拖曳照網頁回報，按下鎖定保留到滑鼠鍵放開。
pub fn grab_after_report(prev: Grab, shapes_empty: bool, dragging: bool) -> Grab {
    if shapes_empty {
        return Grab::NONE;
    }
    Grab { dragging, pressed: prev.pressed }
}

/// 舞台被隱藏（勿擾、⌘⇧K、沒有待決題）或重新顯示前的接管狀態（INV 24）：網頁回報的拖曳與原生按下鎖定一律解除。
/// 第二個值＝要不要發 `kunsu://edge_drag_end`：之前有任一個在接管才通知網頁照「沒畫出瀏海」清理（不清的話網頁之後每次
/// 幾何改變都回報「拖曳中」，原生會重新接管全螢幕指標事件）；解除後再呼叫就不再通知，隱藏期間的閒置輪詢不重發。
pub fn grab_after_hide(prev: Grab) -> (Grab, bool) {
    (Grab::NONE, prev.dragging || prev.pressed)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Decision {
    /// true＝舞台視窗穿透（不接收滑鼠事件）。
    pub ignore_mouse: bool,
    /// 拖曳中滑鼠鍵已全部放開：原生層強制結束拖曳接管。
    pub end_drag: bool,
    /// 游標在命中區內（給網頁的進出通知用）。
    pub inside: bool,
    pub next_poll_ms: u64,
}

/// 依游標到瀏海的距離決定下一次輪詢（場景 14／19）：16–250 ms；位置不明或沒有命中區用最慢的。
pub fn poll_interval(distance: Option<f64>) -> u64 {
    let Some(d) = distance.filter(|d| d.is_finite()) else { return POLL_MAX_MS };
    let ms = (d.max(0.0) / POINTER_MAX_SPEED_PX_PER_MS).floor() as u64;
    ms.clamp(POLL_MIN_MS, POLL_MAX_MS)
}

pub fn decide(cursor: Option<(f64, f64)>, shapes: &[HitShape], grab: Grab, buttons_down: bool) -> Decision {
    let inside = cursor.is_some_and(|c| shapes.iter().any(|s| s.contains(c)));
    // 在瀏海上按下或拖曳中，且滑鼠鍵還按著：接管全螢幕指標事件。
    if (grab.dragging || grab.pressed) && buttons_down {
        return Decision { ignore_mouse: false, end_drag: false, inside, next_poll_ms: POLL_MIN_MS };
    }
    // 網頁回報拖曳中但鍵已放開（網頁可能漏收 pointerup）→ 原生層結束；只是按下鎖定則網頁沒在拖曳，不必通知。
    let end_drag = grab.dragging;
    let distance = cursor.and_then(|c| shapes.iter().map(|s| s.bbox_distance(c)).reduce(f64::min));
    Decision {
        ignore_mouse: !inside,
        end_drag,
        inside,
        next_poll_ms: if inside || end_drag { POLL_MIN_MS } else { poll_interval(distance) },
    }
}

/// 原生進出通知（場景 11b）：進出命中區時送一次；停在命中區內每 `HEARTBEAT_MS` 重送「在瀏海上」。
/// `prev` 是上次送出的值（None＝還沒送過），`since_last_ms` 是距上次送出的時間。
pub fn pointer_signal(prev: Option<bool>, inside: bool, since_last_ms: u64) -> Option<bool> {
    if prev != Some(inside) {
        return Some(inside);
    }
    (inside && since_last_ms >= HEARTBEAT_MS).then_some(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 一段 1/4 圓弧（圓心 c、半徑 r、從角度 a0 到 a1，弧度），細分 32 段。
    fn arc(c: (f64, f64), r: f64, a0: f64, a1: f64) -> Vec<f64> {
        let mut out = Vec::new();
        for i in 0..=32 {
            let a = a0 + (a1 - a0) * f64::from(i) / 32.0;
            out.push(c.0 + r * a.cos());
            out.push(c.1 + r * a.sin());
        }
        out
    }

    /// 左上 L 形，形狀與網頁產生的一致：兩臂往螢幕外延伸 40 px；自由端內側凸角半徑 12。
    /// 左臂 x∈[-40,32]、y∈[-40,300]，下端內側角圓角（圓心 (20,288)）；上臂 x∈[-40,260]、y∈[-40,32]，右端內側角圓角
    /// （圓心 (248,20)）。
    fn band() -> Vec<HitShape> {
        let half_pi = std::f64::consts::FRAC_PI_2;
        let mut left = vec![-40.0, -40.0, 32.0, -40.0, 32.0, 288.0];
        left.extend(arc((20.0, 288.0), 12.0, 0.0, half_pi));
        left.extend([-40.0, 300.0]);
        let mut top = vec![-40.0, -40.0, 260.0, -40.0, 260.0, 20.0];
        top.extend(arc((248.0, 20.0), 12.0, 0.0, half_pi));
        top.extend([-40.0, 32.0]);
        vec![HitShape::from(left), HitShape::from(top)]
    }

    /// 與多邊形無關的解析式：點是否在上面那個 L 形裡；離圓弧太近（0.05 px 內）回 None 不比較。
    fn oracle(x: f64, y: f64) -> Option<bool> {
        let rounded = |x: f64, y: f64, x0: f64, x1: f64, y0: f64, y1: f64, c: (f64, f64)| -> Option<bool> {
            if x < x0 || x > x1 || y < y0 || y > y1 {
                return Some(false);
            }
            if x > c.0 && y > c.1 {
                let d = (x - c.0).hypot(y - c.1);
                if (d - 12.0).abs() < 0.05 {
                    return None;
                }
                return Some(d < 12.0);
            }
            Some(true)
        };
        let a = rounded(x, y, -40.0, 32.0, -40.0, 300.0, (20.0, 288.0))?;
        let b = rounded(x, y, -40.0, 260.0, -40.0, 32.0, (248.0, 20.0))?;
        Some(a || b)
    }

    #[test]
    fn inside_the_band_receives_events() {
        let d = decide(Some((10.0, 120.0)), &band(), Grab::NONE, false);
        assert!(!d.ignore_mouse);
        assert!(d.inside);
        assert_eq!(d.next_poll_ms, POLL_MIN_MS);
    }

    /// INV 24：命中區＝畫出來的形狀；命中區外一律穿透（含 L 形內側、選單列、沒有命中區、游標位置不明）；
    /// 在瀏海上按下（原生鎖定）或拖曳中接管，滑鼠鍵放開即結束。網頁沒畫出瀏海時回報空命中區：整個舞台穿透。
    #[test]
    fn inv_edge_notch_click_through() {
        let shapes = band();
        let mut compared = 0;
        for x in (0..1440).step_by(3) {
            for y in (0..900).step_by(3) {
                let (x, y) = (f64::from(x) + 0.5, f64::from(y) + 0.5);
                let Some(inside) = oracle(x, y) else { continue };
                compared += 1;
                let d = decide(Some((x, y)), &shapes, Grab::NONE, false);
                assert_eq!(d.ignore_mouse, !inside, "({x}, {y})");
                assert!(!d.end_drag);
                // 拖曳中：滑鼠鍵按著 → 接管；放開 → 立即結束並恢復穿透判斷。
                let held = decide(Some((x, y)), &shapes, Grab::DRAG, true);
                assert!(!held.ignore_mouse && !held.end_drag);
                let released = decide(Some((x, y)), &shapes, Grab::DRAG, false);
                assert!(released.end_drag, "網頁漏收 pointerup 也要結束");
                assert_eq!(released.ignore_mouse, !inside);
                // 在瀏海上按下、網頁還沒回報拖曳：按著就接管；放開立即恢復穿透判斷（不送 drag_end，網頁沒在拖曳）。
                let pressed = decide(Some((x, y)), &shapes, Grab::PRESS, true);
                assert!(!pressed.ignore_mouse && !pressed.end_drag);
                let let_go = decide(Some((x, y)), &shapes, Grab::PRESS, false);
                assert_eq!(let_go.ignore_mouse, !inside);
                assert!(!let_go.end_drag);
                // 沒畫出瀏海（網頁回報空命中區）：任何地方都穿透，按著滑鼠也不接管。
                assert!(decide(Some((x, y)), &[], Grab::NONE, true).ignore_mouse);
                assert!(!press_latch(false, false, true, decide(Some((x, y)), &[], Grab::NONE, true).inside));
            }
        }
        assert!(compared > 100_000);
        assert!(decide(Some((150.0, 150.0)), &shapes, Grab::NONE, false).ignore_mouse, "L 形內側是空白區，要穿透");
        assert!(decide(Some((500.0, 10.0)), &shapes, Grab::NONE, false).ignore_mouse, "選單列要穿透");
        assert!(decide(Some((33.0, 120.0)), &shapes, Grab::NONE, false).ignore_mouse, "不加容差：黑色外 1 px 就穿透");
        assert!(decide(Some((10.0, 10.0)), &[], Grab::NONE, false).ignore_mouse, "沒有命中區＝全穿透");
        assert!(decide(None, &shapes, Grab::NONE, false).ignore_mouse, "游標位置不明＝穿透");
        assert!(decide(None, &shapes, Grab::DRAG, false).end_drag);

        // QA 第 3 輪：命中區被清空（瀏海不再畫出，例如拖曳中所有題目在別處結案）時立即解除接管——網頁回報的拖曳與原生
        // 按下鎖定都不算；之後瀏海再出現時也不帶著舊拖曳。
        let both = Grab { dragging: true, pressed: true };
        assert_eq!(grab_after_report(both, true, true), Grab::NONE, "清空命中區：網頁同時回報拖曳也不算");
        assert_eq!(grab_after_report(Grab::DRAG, true, false), Grab::NONE);
        assert_eq!(grab_after_report(Grab::PRESS, true, false), Grab::NONE);
        // 有命中區：拖曳照網頁回報，按下鎖定保留到滑鼠鍵放開
        assert_eq!(grab_after_report(Grab::PRESS, false, true), both);
        assert_eq!(grab_after_report(both, false, false), Grab::PRESS);
        assert_eq!(grab_after_report(Grab::NONE, false, false), Grab::NONE);
        // 清空後滑鼠鍵還按著（例如正在拖別的 app 的東西）：整個舞台穿透、不送 drag_end，鎖定也不會因為按著而重新成立
        let cleared = grab_after_report(both, true, false);
        for &(x, y) in &[(10.0, 120.0), (150.0, 150.0), (900.0, 700.0)] {
            let d = decide(Some((x, y)), &[], cleared, true);
            assert!(d.ignore_mouse && !d.end_drag, "({x}, {y})");
            assert!(!press_latch(cleared.pressed, true, true, d.inside));
        }
        // 瀏海再出現（新的命中區、網頁不再回報拖曳）而滑鼠鍵還按著：命中區外照樣穿透
        let again = grab_after_report(cleared, false, false);
        assert!(decide(Some((400.0, 400.0)), &shapes, again, true).ignore_mouse);

        // QA 第 4 輪：拖曳中（或在瀏海上按著還沒放開時）舞台被隱藏（勿擾、⌘⇧K）——命中區沒被清空，原生層自己解除拖曳與
        // 按下鎖定，並通知網頁（`kunsu://edge_drag_end`）照「沒畫出瀏海」清理；隱藏期間閒置輪詢不重發。
        assert_eq!(grab_after_hide(Grab::DRAG), (Grab::NONE, true), "拖曳中被隱藏：解除並通知網頁");
        assert_eq!(grab_after_hide(Grab::PRESS), (Grab::NONE, true), "按著還沒拖時被隱藏：也解除並通知（網頁手勢以取消結束）");
        assert_eq!(grab_after_hide(both), (Grab::NONE, true));
        assert_eq!(grab_after_hide(Grab::NONE), (Grab::NONE, false), "沒在接管：不通知，隱藏期間的每次輪詢都不重發");
        let (hidden, _) = grab_after_hide(Grab::DRAG);
        assert_eq!(grab_after_hide(hidden), (Grab::NONE, false), "通知只發一次");
        // 再顯示時網頁已清掉拖曳（回報不拖曳）：滑鼠鍵按著（正在拖別的 app）也不接管，命中區外穿透
        let shown = grab_after_report(hidden, false, false);
        assert_eq!(shown, Grab::NONE);
        for &(x, y) in &[(150.0, 150.0), (400.0, 400.0), (900.0, 700.0)] {
            let d = decide(Some((x, y)), &shapes, shown, true);
            assert!(d.ignore_mouse && !d.end_drag, "({x}, {y})");
        }
    }

    /// 端點圓角外（矩形角落裡看不到的地方）穿透；圓角內點得到。
    #[test]
    fn rounded_cap_corner_is_click_through() {
        let shapes = band();
        assert!(decide(Some((31.0, 299.0)), &shapes, Grab::NONE, false).ignore_mouse);
        assert!(!decide(Some((25.0, 293.0)), &shapes, Grab::NONE, false).ignore_mouse);
        assert!(decide(Some((259.0, 31.0)), &shapes, Grab::NONE, false).ignore_mouse);
        assert!(!decide(Some((252.0, 24.0)), &shapes, Grab::NONE, false).ignore_mouse);
    }

    /// 輪詢間隔＝距離 ÷ 3 px/ms（16–250 ms）：任何不超過 3000 px/s 的游標在抵達前就已切換；300 px 外 ≤ 10 Hz。
    #[test]
    fn poll_interval_scales_with_distance() {
        assert_eq!(poll_interval(Some(0.0)), POLL_MIN_MS);
        assert_eq!(poll_interval(Some(30.0)), POLL_MIN_MS);
        assert_eq!(poll_interval(Some(150.0)), 50);
        assert_eq!(poll_interval(Some(300.0)), 100);
        assert_eq!(poll_interval(Some(750.0)), POLL_MAX_MS);
        assert_eq!(poll_interval(Some(5000.0)), POLL_MAX_MS);
        assert_eq!(poll_interval(None), POLL_MAX_MS);
        let mut prev = 0;
        for d in 0..3000 {
            let d = f64::from(d);
            let ms = poll_interval(Some(d));
            assert!(ms >= prev, "距離越遠間隔不會變短");
            prev = ms;
            assert!(ms as f64 <= (d / POINTER_MAX_SPEED_PX_PER_MS).max(POLL_MIN_MS as f64), "{d} px：游標可能在下一次輪詢前抵達");
            if d >= 300.0 {
                assert!(ms >= 100, "300 px 外 ≤ 10 Hz");
            }
        }
        let shapes = band();
        assert_eq!(decide(Some((900.0, 700.0)), &shapes, Grab::NONE, false).next_poll_ms, POLL_MAX_MS);
        // (150,150) 到左臂外接矩形 118 px
        assert_eq!(decide(Some((150.0, 150.0)), &shapes, Grab::NONE, false).next_poll_ms, 39);
        assert_eq!(decide(Some((900.0, 700.0)), &shapes, Grab::DRAG, true).next_poll_ms, POLL_MIN_MS, "拖曳中全速");
        assert_eq!(decide(Some((900.0, 700.0)), &[], Grab::NONE, false).next_poll_ms, POLL_MAX_MS);
    }

    /// AC-EN-14：在瀏海上按下後滑鼠鍵沒放開，就由原生持續接管（不等網頁回報開始拖曳）；放開立即結束。
    /// 必須先看到放開、再看到按下時游標在命中區內才鎖定：在別處按下後拖進瀏海不算。
    #[test]
    fn press_on_notch_holds_capture_until_release() {
        let shapes = band();
        let on = Some((10.0, 120.0));
        let off = Some((400.0, 400.0));
        // 一連串輪詢：(游標, 滑鼠鍵按著) → 期望是否接管
        let mut held = false;
        let mut was_down = true; // 舞台剛顯示：視為按著，必須先看到放開
        let mut run = |cursor: Option<(f64, f64)>, down: bool| -> bool {
            let inside = decide(cursor, &shapes, Grab::NONE, false).inside;
            held = press_latch(held, was_down, down, inside);
            was_down = down;
            !decide(cursor, &shapes, Grab { dragging: false, pressed: held }, down).ignore_mouse
        };
        assert!(!run(off, true), "舞台剛顯示時滑鼠鍵就按著：不鎖定");
        assert!(run(on, true), "游標在瀏海上本來就可點");
        assert!(!run(off, true), "還沒看到放開：拖離就穿透");
        assert!(!run(off, false));
        assert!(run(on, false), "移上瀏海");
        assert!(run(on, true), "在瀏海上按下 → 鎖定");
        assert!(run(off, true), "按著快速拖離瀏海：網頁還沒回報拖曳也持續接管");
        assert!(run(Some((900.0, 700.0)), true), "拖到螢幕另一頭也還接管");
        assert!(!run(off, false), "放開：立即恢復穿透");
        assert!(!run(off, true), "在瀏海外按下：不鎖定");
        assert!(run(on, true), "按著拖進瀏海：游標在命中區內才可點");
        assert!(!run(off, true), "按著再拖出去：不接管（不是在瀏海上按下的）");
        assert!(!run(None, false));
        // 純函數：放開一律解除；已鎖定就維持到放開
        assert!(!press_latch(true, true, false, true));
        assert!(press_latch(true, true, true, false));
        assert!(press_latch(false, false, true, true));
        assert!(!press_latch(false, true, true, true), "前一次就按著：不是這次在瀏海上按下的");
        assert!(!press_latch(false, false, true, false));
    }

    /// 原生進出通知（AC-EN-11）：進出命中區時送一次；停在命中區內每 1 秒重送「在瀏海上」。
    #[test]
    fn pointer_signal_on_transition_and_heartbeat() {
        assert_eq!(pointer_signal(None, true, 0), Some(true));
        assert_eq!(pointer_signal(None, false, 0), Some(false));
        assert_eq!(pointer_signal(Some(false), true, 5), Some(true));
        assert_eq!(pointer_signal(Some(true), false, 5), Some(false));
        assert_eq!(pointer_signal(Some(true), true, HEARTBEAT_MS - 1), None);
        assert_eq!(pointer_signal(Some(true), true, HEARTBEAT_MS), Some(true));
        assert_eq!(pointer_signal(Some(false), false, 99_999), None, "在外面不重送");
    }
}
