import XCTest
@testable import KunsuCore

/// AC-MA-02（2026-10-12 改）：只有真的用手指拉滿再放開才載入歷史。
final class PullTrackerTests: XCTestCase {
    func test_threshold_is_a_full_pull() {
        XCTAssertEqual(Pull.threshold, 120)
    }

    func test_only_overscroll_past_the_top_counts() {
        XCTAssertEqual(PullTracker.overscroll(offsetY: 300, topInset: 64), 0, "內容靜止在下面不算拉")
        XCTAssertEqual(PullTracker.overscroll(offsetY: -64, topInset: 64), 0)
        XCTAssertEqual(PullTracker.overscroll(offsetY: -164, topInset: 64), 100)
    }

    func test_a_tap_never_loads() {
        var t = PullTracker()
        XCTAssertFalse(t.end(more: true))
        t.update(500) // 沒有手指拖著（例如慣性或版面變化）
        XCTAssertFalse(t.end(more: true))
    }

    func test_short_pull_springs_back() {
        var t = PullTracker()
        t.begin()
        XCTAssertFalse(t.update(60))
        XCTAssertEqual(t.phase(more: true), .pulling(progress: 0.5))
        XCTAssertFalse(t.end(more: true))
        XCTAssertEqual(t.phase(more: true), .idle)
    }

    func test_full_pull_ticks_once_per_crossing_and_loads_on_release() {
        var t = PullTracker()
        t.begin()
        XCTAssertFalse(t.update(119))
        XCTAssertTrue(t.update(120), "跨過門檻震一下")
        XCTAssertEqual(t.phase(more: true), .ready)
        XCTAssertFalse(t.update(140), "已經滿了不再震")
        XCTAssertFalse(t.update(100))
        XCTAssertEqual(t.phase(more: true), .pulling(progress: 100.0 / 120.0), "拉回去就變回未滿")
        XCTAssertTrue(t.update(125), "再跨過一次才再震")
        XCTAssertTrue(t.end(more: true))
        XCTAssertFalse(t.end(more: true), "放開一次只載入一次")
    }

    func test_released_below_after_reaching_does_not_load() {
        var t = PullTracker()
        t.begin()
        _ = t.update(130)
        _ = t.update(80)
        XCTAssertFalse(t.end(more: true))
    }

    func test_nothing_more_never_loads() {
        var t = PullTracker()
        t.more = false
        t.begin()
        XCTAssertFalse(t.update(200), "沒有更早的紀錄就不震")
        XCTAssertEqual(t.phase(more: false), .noMore)
        XCTAssertFalse(t.end(more: false))
    }
}
