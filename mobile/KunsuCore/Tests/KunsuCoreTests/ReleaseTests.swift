import XCTest
@testable import KunsuCore

/// AC-MA-18：上架前，問候不寫死任何人的名字；預設連 api.kunsu.app。
final class ReleaseTests: XCTestCase {
    func test_greeting_follows_time_and_account_name() {
        XCTAssertEqual(Greeting.text(hour: 7, name: nil), "早安")
        XCTAssertEqual(Greeting.text(hour: 14, name: "Alex"), "午安，Alex")
        XCTAssertEqual(Greeting.text(hour: 23, name: "  "), "晚安")
        XCTAssertEqual(Greeting.text(hour: 3, name: "Amy"), "晚安，Amy")
        XCTAssertEqual(Greeting.text(hour: 5, name: nil), "早安")
        XCTAssertEqual(Greeting.text(hour: 11, name: nil), "午安")
        XCTAssertEqual(Greeting.text(hour: 18, name: nil), "晚安")
    }

    func test_default_service_is_kunsu_app() {
        XCTAssertEqual(AccountAPI.defaultBase, "https://api.kunsu.app")
    }
}
