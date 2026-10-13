//! 時區轉換集中地（CONTRACT 時區注意事項）。
//!
//! 規則：`Ts` 一律以 UTC 儲存與傳遞；時窗（t1_batch / quiet_hours）以台北
//! 本地時間解釋。台北無日光節約時間，固定 UTC+8，用 `FixedOffset` 即可，
//! 不需要 tz database。所有 UTC↔Taipei 轉換必須經過本模組，禁止散落各處。

use chrono::{DateTime, Duration, FixedOffset, NaiveDate, NaiveTime, TimeZone, Utc};

/// 全系統統一時間戳型別：UTC 存。
pub type Ts = DateTime<Utc>;

/// 台北固定 UTC+8。
pub fn taipei_offset() -> FixedOffset {
    FixedOffset::east_opt(8 * 3600).expect("UTC+8 恆為合法 offset")
}

/// UTC 時間戳 → 台北牆鐘時間。
pub fn to_taipei(ts: Ts) -> DateTime<FixedOffset> {
    ts.with_timezone(&taipei_offset())
}

/// 該時刻的台北本地時刻（time-of-day）。
pub fn taipei_time(ts: Ts) -> NaiveTime {
    to_taipei(ts).time()
}

/// 該時刻的台北本地日期（`first_proposed` 等 Date 欄位以此為準）。
pub fn taipei_date(ts: Ts) -> NaiveDate {
    to_taipei(ts).date_naive()
}

/// `ts` 是否落在台北本地時窗 `[start, end)`。
/// `start > end` 表示跨午夜視窗（例：quiet hours 23:00–09:00）。
pub fn in_taipei_window(ts: Ts, start: NaiveTime, end: NaiveTime) -> bool {
    let t = taipei_time(ts);
    if start <= end {
        t >= start && t < end
    } else {
        t >= start || t < end
    }
}

/// `after` 之後（嚴格大於）下一次台北本地時刻 `at` 出現的 UTC 時間戳。
/// 用於 T1 批次窗排程。
pub fn next_taipei_occurrence(after: Ts, at: NaiveTime) -> Ts {
    let local = to_taipei(after);
    let today = local.date_naive();
    let candidate = taipei_offset()
        .from_local_datetime(&today.and_time(at))
        .single()
        .expect("固定 offset 無 DST，本地時間唯一");
    let candidate = candidate.with_timezone(&Utc);
    if candidate > after {
        candidate
    } else {
        candidate + Duration::days(1)
    }
}
