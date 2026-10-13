import Foundation

// 手機 App 的資料模型（PRD §30；BDD mobile-app）。正式版由 Mac 上的 Kunsu 送來，現在由 Fixtures 提供。

public enum Agent: String, CaseIterable, Codable, Equatable, Sendable {
    case claude, codex
    public var name: String { self == .claude ? "Claude" : "Codex" }
}

public enum Turn: Equatable, Sendable {
    case user(String)
    case ai(String)
    case tool(String)
    case system(String)
}

public enum SessionStatus: Equatable, Sendable {
    case reply
    case permission
    case running(String)
    case idle
}

public struct Permission: Equatable, Sendable {
    public var command: String
    public var reason: String
    public init(command: String, reason: String) { self.command = command; self.reason = reason }
}

public struct Session: Equatable, Sendable {
    public var id: String
    public var name: String
    public var agent: Agent
    public var status: SessionStatus
    public var turns: [Turn]
    public var permission: Permission?

    public init(id: String, name: String? = nil, agent: Agent, status: SessionStatus, turns: [Turn], permission: Permission? = nil) {
        self.id = id; self.name = name ?? id; self.agent = agent; self.status = status; self.turns = turns; self.permission = permission
    }

    public var lastAI: String {
        for t in turns.reversed() { if case .ai(let s) = t { return s } }
        return ""
    }
}

public struct Option: Equatable, Sendable {
    public var label: String
    public var detail: String?
    public var recommended: Bool
    /// Kunsu 題目的選項 id（接上 Mac 時送回去的 choice_id）。
    public var id: String?
    public init(_ label: String, detail: String? = nil, recommended: Bool = false, id: String? = nil) {
        self.label = label; self.detail = detail; self.recommended = recommended; self.id = id
    }
}

public struct MoneyParam: Equatable, Sendable {
    public var label: String
    public var value: String
    public init(_ label: String, _ value: String) { self.label = label; self.value = value }
}

public struct Decision: Equatable, Sendable {
    public var id: String
    public var agent: Agent
    public var project: String
    public var sessionID: String?
    public var question: String
    public var why: String
    public var options: [Option]
    public var money: [MoneyParam]?
    public var time: String
    /// 文字題：沒有選項時用打字回答。
    public var allowText: Bool
    /// 題目附的證據行（「前因後果？」用）。
    public var context: [String]
    /// 草稿題的版本（所見即所答）。
    public var revision: Int?
    public var contentHash: String?
    /// 這題有原對話（Mac 上的 Kunsu 說的）：可以「問本人」，Mac 再看它有沒有在等。
    public var hasConversation = false

    public init(id: String, agent: Agent, project: String, sessionID: String?, question: String, why: String,
                options: [Option], money: [MoneyParam]? = nil, time: String, allowText: Bool = false,
                context: [String] = [], revision: Int? = nil, contentHash: String? = nil) {
        self.id = id; self.agent = agent; self.project = project; self.sessionID = sessionID
        self.question = question; self.why = why; self.options = options; self.money = money; self.time = time
        self.allowText = allowText; self.context = context; self.revision = revision; self.contentHash = contentHash
    }
    public var isMoney: Bool { money != nil }
}

public struct HistoryEntry: Equatable, Sendable, Identifiable {
    public var id: String
    public var time: String
    public var agent: Agent
    public var project: String
    public var question: String
    public var answer: String
    public var result: String
    public var sessionID: String?

    public init(id: String = UUID().uuidString, time: String, agent: Agent, project: String, question: String,
                answer: String, result: String, sessionID: String? = nil) {
        self.id = id; self.time = time; self.agent = agent; self.project = project; self.question = question
        self.answer = answer; self.result = result; self.sessionID = sessionID
    }
}

public struct HistoryDay: Equatable, Sendable, Identifiable {
    public var name: String
    public var entries: [HistoryEntry]
    public var id: String { name }
    public init(_ name: String, _ entries: [HistoryEntry]) { self.name = name; self.entries = entries }
}

public enum FollowUpBy: Equatable, Sendable { case relay, agent }
public enum FollowUpStatus: Equatable, Sendable { case thinking, answered, failed }
public enum FollowUpKind: Equatable, Sendable {
    case why, context, text(String)
    public var question: String {
        switch self {
        case .why: "為什麼？"
        case .context: "前因後果？"
        case .text(let t): t
        }
    }
}

public struct FollowUp: Equatable, Sendable, Identifiable {
    public var id: String
    public var kind: FollowUpKind
    public var answer: String?
    public var quote: [String]?
    public var by: FollowUpBy
    public var status: FollowUpStatus
    /// 署名：「軍師（Claude）」或「kunsu 本人」。
    public var signature: String
    public var question: String { kind.question }
}

public struct Snapshot: Equatable, Sendable {
    public var agent: Agent
    public var project: String
    public var question: String
}

/// 這次打開裡的一件事：軍師的問題、你的答案與追問。
public struct Exchange: Equatable, Sendable, Identifiable {
    public var id: String
    public var itemID: String
    public var answer: String?
    public var snapshot: Snapshot?
    public var followUps: [FollowUp] = []
    public var historyID: String?
    public var skipped: Bool { answer == Engine.skipLabel }
}

public enum ChatEntry: Equatable, Sendable {
    case me(String)
    case relay(String)
    case relayThinking
    case task(sessionID: String)
}

public enum Entry: Equatable, Sendable, Identifiable {
    case exchange(Exchange)
    case chat(id: String, ChatEntry)

    public var id: String {
        switch self {
        case .exchange(let e): e.id
        case .chat(let id, _): id
        }
    }
    public var exchange: Exchange? { if case .exchange(let e) = self { return e }; return nil }
    public var chat: ChatEntry? { if case .chat(_, let c) = self { return c }; return nil }
}

public struct BoardState: Equatable, Sendable {
    public var decisions: [Decision]
    public var sessions: [String: Session]
    /// 依新到舊：history[0] 是今晚。
    public var history: [HistoryDay]
    public var loadedDays: Int = 0
    public var timeline: [Entry] = []
    public var skipped: [String] = []
    public var relay: Agent = .claude
    /// 這次打開裡答過的事；重新打開時併進今晚的紀錄。
    public var visit: [HistoryEntry] = []
    public var projects: [String]
    /// 「問本人」要中轉 AI 與原對話都接上才開（接上 Mac 的第一版先關）。
    public var forwardEnabled = true
    /// 這次答過的題：Mac 的快照不能把它們加回來（撤回窗內答案還沒送出）。
    public var answeredIDs: Set<String> = []

    public init(decisions: [Decision], sessions: [String: Session], history: [HistoryDay], relay: Agent = .claude,
                projects: [String] = ["kunsu", "ads-review", "blog-site", "demo-shop"]) {
        self.decisions = decisions; self.sessions = sessions; self.history = history; self.relay = relay; self.projects = projects
    }
}

public enum ItemKind: Equatable, Sendable {
    case decision(Decision)
    case permission(Session)
    case reply(Session)
}

/// 需要你的一件事。
public struct Item: Equatable, Sendable, Identifiable {
    public var id: String
    public var kind: ItemKind
    public var agent: Agent
    public var project: String
    public var question: String
    public var sessionID: String?

    public var kindName: String {
        switch kind {
        case .decision: "decision"
        case .permission: "permission"
        case .reply: "reply"
        }
    }
}
