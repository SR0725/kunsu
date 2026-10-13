import XCTest
@testable import KunsuCore

/// AC-MU-04：手機用 email 連結登入（在任何裝置點都可以），可以刪除帳號。
final class LoginFlowTests: XCTestCase {
    let t0 = Date(timeIntervalSince1970: 1_000_000)

    func test_invalid_email_is_not_sent() {
        XCTAssertEqual(LoginFlow.idle().submit("nope", now: t0), .idle(error: "請輸入正確的 email"))
    }

    func test_send_wait_and_done() {
        var s = LoginFlow.idle().submit(" Alex@Example.com ", now: t0)
        XCTAssertEqual(s, .sending(email: "alex@example.com", since: t0))
        s = s.sent(poll: "p1")
        XCTAssertEqual(s, .waiting(email: "alex@example.com", poll: "p1", since: t0))
        s = s.polled(.pending, now: t0.addingTimeInterval(2))
        XCTAssertEqual(s, .waiting(email: "alex@example.com", poll: "p1", since: t0))
        s = s.polled(.session("tok"), now: t0.addingTimeInterval(4))
        XCTAssertEqual(s, .done(session: "tok"))
    }

    func test_expiry_and_errors() {
        let w = LoginFlow.waiting(email: "a@b.co", poll: "p", since: t0)
        XCTAssertEqual(w.polled(.pending, now: t0.addingTimeInterval(15 * 60 + 1)), .idle(error: "連結已過期，請重新寄一次"))
        XCTAssertEqual(w.polled(.expired, now: t0), .idle(error: "連結已過期，請重新寄一次"))
        XCTAssertEqual(w.failed("連不上帳號服務"), .idle(error: "連不上帳號服務"))
    }

    func test_api_base_follows_the_relay() {
        XCTAssertEqual(AccountAPI.base(relay: "wss://kunsu-relay.zeabur.app/v1"), "https://kunsu-relay.zeabur.app")
        XCTAssertEqual(AccountAPI.base(relay: "ws://127.0.0.1:8080/v1/"), "http://127.0.0.1:8080")
    }

    func test_requests() throws {
        let start = AccountAPI.startLogin(base: "https://x", email: "a@b.co")
        XCTAssertEqual(start.url?.absoluteString, "https://x/auth/email/start")
        XCTAssertEqual(start.httpMethod, "POST")
        XCTAssertEqual(try JSONSerialization.jsonObject(with: start.httpBody!) as? [String: String], ["email": "a@b.co", "client": "ios"])
        let del = AccountAPI.deleteAccount(base: "https://x", session: "tok")
        XCTAssertEqual(del.httpMethod, "DELETE")
        XCTAssertEqual(del.url?.absoluteString, "https://x/account")
        XCTAssertEqual(del.value(forHTTPHeaderField: "Authorization"), "Bearer tok")
        XCTAssertEqual(AccountAPI.pollResult(status: 202, body: Data()), .pending)
        XCTAssertEqual(AccountAPI.pollResult(status: 410, body: Data()), .expired)
        XCTAssertEqual(AccountAPI.pollResult(status: 200, body: Data(#"{"session":"s"}"#.utf8)), .session("s"))
    }
}
