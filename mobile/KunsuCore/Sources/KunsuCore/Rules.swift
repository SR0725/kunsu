import Foundation

/// 向下拖出歷史紀錄（AC-MA-02）：拉滿門檻放開才載入。
public enum Pull {
    /// 要「整個往下拉」才算（2026-10-12 作者：點一下就打開歷史紀錄不對）。
    public static let threshold: Double = 120

    public enum Phase: Equatable, Sendable {
        case idle
        case pulling(progress: Double)
        case ready
        case noMore
    }

    public static func phase(distance: Double, more: Bool) -> Phase {
        guard distance > 0 else { return .idle }
        guard more else { return .noMore }
        return distance >= threshold ? .ready : .pulling(progress: distance / threshold)
    }

    /// 圓圈旁的提示字（作者 2026-10-12 定名）。
    public static func hint(_ p: Phase) -> String {
        switch p {
        case .noMore: "沒有更早的紀錄了"
        case .ready: "放開看歷史紀錄"
        default: "向下拖出歷史紀錄"
        }
    }
}

/// 下拉追蹤（AC-MA-02）：只算超出頂端的距離；只有手指拖著拉滿再放開才載入；跨過門檻時震一下（每次跨過一次）。
public struct PullTracker: Equatable, Sendable {
    public private(set) var distance: Double = 0
    public private(set) var dragging = false
    /// 有沒有更早的紀錄（沒有就不震、不載入）。
    public var more = true
    private var full = false

    public init() {}

    /// 捲動內容超出頂端多少：offsetY 是內容偏移，topInset 是內容上方留白。
    public static func overscroll(offsetY: Double, topInset: Double) -> Double {
        max(0, -(offsetY + topInset))
    }

    /// 手指開始拖。
    public mutating func begin() {
        dragging = true
        full = false
    }

    /// 新的下拉距離。回 true＝剛跨過門檻（震一下）。
    @discardableResult
    public mutating func update(_ d: Double) -> Bool {
        distance = max(0, d)
        guard dragging, more else { full = false; return false }
        let now = distance >= Pull.threshold
        defer { full = now }
        return now && !full
    }

    /// 手指放開。回 true＝載入一天。
    public mutating func end(more: Bool) -> Bool {
        let load = dragging && more && full && distance >= Pull.threshold
        dragging = false
        full = false
        distance = 0
        return load
    }

    public func phase(more: Bool) -> Pull.Phase {
        Pull.phase(distance: distance, more: more)
    }
}

/// 答案按鈕排法（AC-MA-04）：短選項兩欄，有一個長或有說明就一排一個。
public enum AnswerLayout: Equatable, Sendable {
    case twoColumns, stacked

    /// 中文字寬：半形字算 0.55。
    public static func width(_ s: String) -> Double {
        s.unicodeScalars.reduce(0) { $0 + ($1.value < 0x100 ? 0.55 : 1) }
    }

    public static func of(labels: [String], hasDetail: Bool) -> AnswerLayout {
        guard (2...4).contains(labels.count), !hasDetail, labels.allSatisfy({ width($0) <= 8 }) else { return .stacked }
        return .twoColumns
    }
}

/// 從 AI 訊息抽出 A／B／C 選項（AC-MA-05）。
public enum OptionParser {
    public struct Parsed: Equatable, Sendable {
        public var key: String
        public var label: String
    }

    private static func match(_ line: String) -> Parsed? {
        let t = line.trimmingCharacters(in: .whitespaces)
        guard let first = t.first, "ABCD".contains(first) else { return nil }
        let rest = t.dropFirst()
        guard let sep = rest.first, ".．、".contains(sep) else { return nil }
        let label = rest.dropFirst().trimmingCharacters(in: .whitespaces)
        return label.isEmpty ? nil : Parsed(key: String(first), label: label)
    }

    public static func parse(_ text: String) -> [Parsed] {
        text.split(separator: "\n").compactMap { match(String($0)) }
    }

    /// 去掉選項行之後的訊息本文。
    public static func body(_ text: String) -> String {
        text.split(separator: "\n", omittingEmptySubsequences: false)
            .filter { match(String($0)) == nil }
            .joined(separator: "\n")
            .trimmingCharacters(in: .whitespacesAndNewlines)
    }
}

public struct AnswerButton: Equatable, Sendable, Identifiable {
    public enum Action: Equatable, Sendable {
        case decision(id: String, option: Int)
        case permission(sessionID: String, allow: Bool)
        case reply(sessionID: String, text: String)
    }
    public var label: String
    public var detail: String?
    public var recommended: Bool
    public var action: Action
    public var id: String { label }
}

public enum TypingTarget: Equatable, Sendable {
    case reply(sessionID: String)
    case answerText(questionID: String)
    case followUp
    case disabled
    case relayChat
}

public enum AnswerOutcome: Equatable, Sendable {
    case needsConfirm
    case queued(UndoToken)
    case executed
    /// 動錢題選了不執行的選項：核對後直接送出（沒有撤回窗，核對畫面就是確認）。
    case sent
}

public struct UndoToken: Hashable, Sendable {
    public let raw: Int
}

public enum Effect: Equatable, Sendable {
    /// 答案已送出（撤回時間結束）：原對話接手。
    case resume(sessionID: String)
    /// 把正式答案送給 Mac 上的 Kunsu（channel=mobile）。
    case answer(questionID: String, choiceID: String?, text: String?, moneyConfirmed: Bool)
}

/// 送給 Mac 的中轉 AI 指令（docs/design/mobile-relay.md；CONTRACT §17）。
public enum RelayCommand {
    public static func followUp(id: String, questionID: String, kind: FollowUpKind) -> [String: String] {
        var c = ["type": "followup", "id": id, "question_id": questionID]
        switch kind {
        case .why: c["kind"] = "why"
        case .context: c["kind"] = "context"
        case .text(let t): c["kind"] = "text"; c["text"] = t
        }
        return c
    }

    public static func forward(id: String, questionID: String, text: String) -> [String: String] {
        ["type": "forward", "id": id, "question_id": questionID, "text": text]
    }

    public static func chat(id: String, text: String) -> [String: String] {
        ["type": "chat", "id": id, "text": text]
    }

    public static func startTask(id: String, agent: Agent, project: String, text: String) -> [String: String] {
        ["type": "start_task", "id": id, "agent": agent.rawValue, "project": project, "text": text]
    }
}
