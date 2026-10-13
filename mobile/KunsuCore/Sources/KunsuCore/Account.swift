import Foundation

/// 手機登入帳號（PRD §31.2；BDD multi-user AC-MU-04）：寄 email 連結，使用者在任何裝置點，手機自動登入。
public enum LoginFlow: Equatable, Sendable {
    case idle(error: String? = nil)
    case sending(email: String, since: Date)
    case waiting(email: String, poll: String, since: Date)
    case done(session: String)

    /// 跟登入連結一樣 15 分鐘。
    public static let wait: TimeInterval = 15 * 60

    public func submit(_ raw: String, now: Date) -> LoginFlow {
        let email = raw.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        let parts = email.split(separator: "@")
        guard parts.count == 2, !parts[0].isEmpty, parts[1].contains("."), !email.contains(" ") else {
            return .idle(error: "請輸入正確的 email")
        }
        return .sending(email: email, since: now)
    }

    public func sent(poll: String) -> LoginFlow {
        guard case .sending(let email, let since) = self else { return self }
        return .waiting(email: email, poll: poll, since: since)
    }

    public func polled(_ result: AccountAPI.PollResult, now: Date) -> LoginFlow {
        guard case .waiting(_, _, let since) = self else { return self }
        switch result {
        case .session(let s): return .done(session: s)
        case .expired: return .idle(error: "連結已過期，請重新寄一次")
        case .pending: return now.timeIntervalSince(since) > LoginFlow.wait ? .idle(error: "連結已過期，請重新寄一次") : self
        }
    }

    public func failed(_ error: String) -> LoginFlow { .idle(error: error) }
}

/// 帳號服務的請求（docs/design/accounts.md）。帳號服務跟中轉站同一台。
public enum AccountAPI {
    public static let defaultBase = "https://api.kunsu.app"

    public enum PollResult: Equatable, Sendable {
        case pending
        case expired
        case session(String)
    }

    /// wss://host/v1 → https://host
    public static func base(relay: String) -> String {
        var s = relay.replacingOccurrences(of: "wss://", with: "https://").replacingOccurrences(of: "ws://", with: "http://")
        while s.hasSuffix("/") { s.removeLast() }
        if s.hasSuffix("/v1") { s.removeLast(3) }
        return s
    }

    private static func request(_ base: String, _ path: String, method: String, session: String? = nil, body: [String: String]? = nil) -> URLRequest {
        var r = URLRequest(url: URL(string: base + path)!)
        r.httpMethod = method
        r.timeoutInterval = 20
        if let session { r.setValue("Bearer \(session)", forHTTPHeaderField: "Authorization") }
        if let body {
            r.setValue("application/json", forHTTPHeaderField: "Content-Type")
            r.httpBody = try? JSONSerialization.data(withJSONObject: body)
        }
        return r
    }

    public static func startLogin(base: String, email: String) -> URLRequest {
        request(base, "/auth/email/start", method: "POST", body: ["email": email, "client": "ios"])
    }

    public static func poll(base: String, poll: String) -> URLRequest {
        request(base, "/auth/poll", method: "POST", body: ["poll": poll])
    }

    public static func account(base: String, session: String) -> URLRequest {
        request(base, "/account", method: "GET", session: session)
    }

    public static func deleteAccount(base: String, session: String) -> URLRequest {
        request(base, "/account", method: "DELETE", session: session)
    }

    public static func pollResult(status: Int, body: Data) -> PollResult {
        switch status {
        case 200:
            let s = (try? JSONSerialization.jsonObject(with: body) as? [String: Any])?["session"] as? String
            return s.map(PollResult.session) ?? .expired
        case 202: return .pending
        default: return .expired
        }
    }
}

/// 問候（AC-MA-18）：依時間；有帳號顯示名稱就接名字，不寫死任何人。
public enum Greeting {
    public static func text(hour: Int, name: String?) -> String {
        let base = (5..<11).contains(hour) ? "早安" : (11..<18).contains(hour) ? "午安" : "晚安"
        guard let n = name?.trimmingCharacters(in: .whitespacesAndNewlines), !n.isEmpty else { return base }
        return "\(base)，\(n)"
    }
}
