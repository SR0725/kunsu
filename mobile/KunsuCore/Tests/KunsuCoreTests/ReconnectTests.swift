import XCTest
@testable import KunsuCore

/// AC-MA-17：舊連線的失敗不能把目前連線標成離線，也不能觸發重連（以前會無限閃）。
final class ReconnectTests: XCTestCase {
    func test_stale_failure_is_ignored() {
        var r = Reconnect()
        let a = r.open()
        XCTAssertEqual(r.alive(a), .online)
        let b = r.open() // 取代 a（例如重新連線）
        XCTAssertEqual(r.failed(a), .ignore, "被取代的 a 取消後回報失敗：不理")
        XCTAssertEqual(r.alive(a), .ignore, "舊連線的心跳也不算")
        XCTAssertEqual(r.alive(b), .online)
    }

    func test_current_failure_goes_offline_and_backs_off() {
        var r = Reconnect()
        let a = r.open()
        XCTAssertEqual(r.failed(a), .offline(retryAfter: 1))
        XCTAssertEqual(r.failed(a), .ignore, "同一條連線只算一次")
        let b = r.open()
        XCTAssertEqual(r.failed(b), .offline(retryAfter: 2))
        let c = r.open()
        XCTAssertEqual(r.failed(c), .offline(retryAfter: 4))
        var d = r.open()
        for _ in 0..<10 { _ = r.failed(d); d = r.open() }
        XCTAssertEqual(r.failed(d), .offline(retryAfter: 30), "最多 30 秒")
    }

    func test_coming_online_resets_backoff_and_reports_once() {
        var r = Reconnect()
        let a = r.open()
        _ = r.failed(a)
        let b = r.open()
        XCTAssertEqual(r.alive(b), .online)
        XCTAssertEqual(r.alive(b), .ignore, "已經在線，心跳不重複回報")
        XCTAssertEqual(r.failed(b), .offline(retryAfter: 1), "連上後退避歸零")
    }

    func test_closed_on_purpose_never_retries() {
        var r = Reconnect()
        let a = r.open()
        r.close()
        XCTAssertEqual(r.failed(a), .ignore)
        XCTAssertEqual(r.alive(a), .ignore)
    }
}
