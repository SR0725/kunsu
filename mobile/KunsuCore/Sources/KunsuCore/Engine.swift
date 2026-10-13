import Foundation

/// 軍師一問一答的狀態機（PRD §30、§30.7；BDD mobile-app）。純值運算，時間與 AI 回覆由 App 的服務層驅動。
public struct Engine: Sendable {
    public static let skipLabel = "等下再說"
    public static let pendingResult = "處理中…"

    public var state: BoardState
    private var pending: (token: UndoToken, snapshot: BoardState, effects: [Effect])?
    private var seq = 0
    /// 撤回時間被新的送出提前結束時產生的效果，由 App 取走執行。
    public private(set) var outbox: [Effect] = []

    public init(state: BoardState) {
        self.state = state
        if state.timeline.isEmpty { askNext() }
    }

    private mutating func nextID(_ prefix: String) -> String {
        seq += 1
        return "\(prefix)\(seq)"
    }

    // MARK: 待處理的事

    public var items: [Item] {
        var out: [Item] = state.decisions.map {
            Item(id: "d:\($0.id)", kind: .decision($0), agent: $0.agent, project: $0.project, question: $0.question, sessionID: $0.sessionID)
        }
        let sessions = state.sessions.values.sorted { $0.id < $1.id }
        for s in sessions where s.status == .permission && s.permission != nil {
            out.append(Item(id: "p:\(s.id)", kind: .permission(s), agent: s.agent, project: s.name,
                            question: "要執行 \(s.permission!.command)", sessionID: s.id))
        }
        for s in sessions where s.status == .reply {
            out.append(Item(id: "r:\(s.id)", kind: .reply(s), agent: s.agent, project: s.name, question: s.lastAI, sessionID: s.id))
        }
        let rank = { (id: String) in state.skipped.firstIndex(of: id) ?? -1 }
        return out.enumerated().sorted { a, b in
            let (ra, rb) = (rank(a.element.id), rank(b.element.id))
            return ra == rb ? a.offset < b.offset : ra < rb
        }.map(\.element)
    }

    public func item(id: String) -> Item? { items.first { $0.id == id } }

    public var current: Exchange? {
        guard let e = state.timeline.last(where: { $0.exchange != nil })?.exchange, e.answer == nil, item(id: e.itemID) != nil else { return nil }
        return e
    }
    public var currentItem: Item? { current.flatMap { item(id: $0.itemID) } }

    /// 現在這件以外，還在等你的事（待辦列）。
    public var waiting: [Item] { items.filter { $0.id != current?.itemID } }

    private var currentIndex: Int? {
        guard let c = current else { return nil }
        return state.timeline.lastIndex { $0.exchange?.id == c.id }
    }

    public mutating func askNext() {
        if let idx = state.timeline.lastIndex(where: { $0.exchange != nil }),
           let e = state.timeline[idx].exchange, e.answer == nil {
            if item(id: e.itemID) != nil { return }
            state.timeline.remove(at: idx)
        }
        guard let next = items.first else { return }
        state.timeline.append(.exchange(Exchange(id: nextID("x"), itemID: next.id)))
    }

    // MARK: 打開與紀錄

    public var visibleHistory: [HistoryDay] { Array(state.history.prefix(state.loadedDays)) }
    public var hasMoreHistory: Bool { state.loadedDays < state.history.count }

    /// 放開下拉：拉滿而且還有更早的紀錄才載入一天。
    @discardableResult
    public mutating func releasePull(distance: Double) -> Bool {
        guard Pull.phase(distance: distance, more: hasMoreHistory) == .ready else { return false }
        state.loadedDays += 1
        return true
    }

    /// 重新打開：這次答過的事併進今晚的紀錄，畫面回到乾淨。
    public mutating func openVisit() {
        commitPending()
        if !state.visit.isEmpty {
            if state.history.isEmpty { state.history = [HistoryDay("今晚", [])] }
            state.history[0].entries.append(contentsOf: state.visit)
            state.visit = []
        }
        state.timeline = []
        state.skipped = []
        state.loadedDays = 0
        askNext()
    }

    // MARK: 回答

    public func answers(for item: Item) -> [AnswerButton] {
        switch item.kind {
        case .decision(let d):
            return d.options.enumerated().map { i, o in
                AnswerButton(label: o.label, detail: o.detail ?? (d.isMoney && o.recommended ? "會先讓你核對金額" : nil),
                             recommended: o.recommended, action: .decision(id: d.id, option: i))
            }
        case .permission(let s):
            return [AnswerButton(label: "允許一次", detail: nil, recommended: false, action: .permission(sessionID: s.id, allow: true)),
                    AnswerButton(label: "拒絕", detail: nil, recommended: false, action: .permission(sessionID: s.id, allow: false))]
        case .reply(let s):
            let opts = OptionParser.parse(s.lastAI)
            if opts.isEmpty { return [AnswerButton(label: "繼續", detail: nil, recommended: false, action: .reply(sessionID: s.id, text: "繼續"))] }
            return opts.map { AnswerButton(label: $0.label, detail: nil, recommended: false, action: .reply(sessionID: s.id, text: "\($0.key)（\($0.label)）")) }
        }
    }

    public func layout(for item: Item) -> AnswerLayout {
        let a = answers(for: item)
        return AnswerLayout.of(labels: a.map(\.label), hasDetail: a.contains { $0.detail != nil })
    }

    @discardableResult
    public mutating func answerDecision(_ id: String, option: Int, confirmed: Bool = false) -> AnswerOutcome? {
        guard let i = state.decisions.firstIndex(where: { $0.id == id }), state.decisions[i].options.indices.contains(option) else { return nil }
        let d = state.decisions[i], o = d.options[option]
        guard let it = item(id: "d:\(id)") else { return nil }
        // 動錢題：每個選項都先過核對畫面（Mac 只收 money_confirmed 的錢題答案，CONTRACT §17）。
        if d.isMoney && !o.recommended {
            guard confirmed else { return .needsConfirm }
            commitPending()
            state.decisions.remove(at: i)
            record(it, answer: o.label, result: "已交給 \(d.agent.name)，錢不會動。")
            outbox.append(.answer(questionID: d.id, choiceID: o.id ?? o.label, text: nil, moneyConfirmed: true))
            askNext()
            return .sent
        }
        if d.isMoney && o.recommended {
            guard confirmed else { return .needsConfirm }
            // 核對後直接送出，不用 Face ID（2026-10-12）。
            commitPending()
            state.decisions.remove(at: i)
            record(it, answer: o.label, result: "已簽章，交給 executor 執行。")
            outbox.append(.answer(questionID: d.id, choiceID: o.id ?? o.label, text: nil, moneyConfirmed: true))
            askNext()
            return .executed
        }
        let snapshot = state
        commitPending()
        state.decisions.remove(at: i)
        record(it, answer: o.label, result: d.sessionID == nil ? "已交給 \(d.agent.name)。" : Engine.pendingResult)
        askNext()
        var effects: [Effect] = [.answer(questionID: d.id, choiceID: o.id ?? o.label, text: nil, moneyConfirmed: false)]
        if let sid = d.sessionID { effects.append(.resume(sessionID: sid)) }
        return .queued(hold(snapshot, effects: effects))
    }

    /// 文字題：打字就是答案（同樣有 5 秒撤回）。
    @discardableResult
    public mutating func answerText(_ id: String, text: String) -> UndoToken? {
        let t = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !t.isEmpty, let i = state.decisions.firstIndex(where: { $0.id == id }), let it = item(id: "d:\(id)") else { return nil }
        let snapshot = state
        commitPending()
        state.decisions.remove(at: i)
        record(it, answer: t, result: "已交給 \(it.agent.name)。")
        askNext()
        return hold(snapshot, effects: [.answer(questionID: id, choiceID: nil, text: t, moneyConfirmed: false)])
    }

    /// Mac 送來的快照：取代題目與紀錄；這次答過的題不加回來，別處答掉的題從畫面消失。
    public mutating func applySnapshot(decisions: [Decision], history: [HistoryDay], projects: [String] = []) {
        if !projects.isEmpty { state.projects = projects }
        state.decisions = decisions.filter { !state.answeredIDs.contains($0.id) }
        state.history = history
        state.loadedDays = min(state.loadedDays, history.count)
        askNext()
    }

    @discardableResult
    public mutating func sendReply(_ sessionID: String, text: String) -> UndoToken? {
        guard let s = state.sessions[sessionID], s.status == .reply, let it = item(id: "r:\(sessionID)"), !text.isEmpty else { return nil }
        let snapshot = state
        commitPending()
        state.sessions[sessionID]?.turns.append(.user(text))
        state.sessions[sessionID]?.status = .running("等你撤回時間結束")
        record(it, answer: text, result: Engine.pendingResult, question: OptionParser.body(s.lastAI))
        askNext()
        return hold(snapshot, effects: [.resume(sessionID: sessionID)])
    }

    /// 批准或拒絕指令：立即生效，沒有撤回（PRD §30.4）。
    public mutating func permit(_ sessionID: String, allow: Bool) {
        guard let s = state.sessions[sessionID], let p = s.permission, let it = item(id: "p:\(sessionID)") else { return }
        commitPending()
        record(it, answer: allow ? "允許一次" : "拒絕", result: allow ? "執行中…" : "AI：好，不執行。", question: "要執行 \(p.command)")
        state.sessions[sessionID]?.permission = nil
        if allow {
            state.sessions[sessionID]?.status = .running("執行 \(p.command)")
            state.sessions[sessionID]?.turns.append(.system("你允許了 \(p.command)"))
        } else {
            state.sessions[sessionID]?.status = .reply
            state.sessions[sessionID]?.turns.append(.ai("好，不執行。要我改成別的做法嗎？\nA. 好\nB. 不用"))
        }
        askNext()
    }

    /// Mac 拒收這題的答案：那則答案寫「沒有送到」，題目不再當成已答（下一份快照會把它帶回來）。
    public mutating func answerRejected(_ questionID: String, error: String) {
        state.answeredIDs.remove(questionID)
        let itemID = "d:\(questionID)"
        guard let idx = state.timeline.lastIndex(where: { $0.exchange?.itemID == itemID && $0.exchange?.answer != nil }),
              let hid = state.timeline[idx].exchange?.historyID,
              let v = state.visit.firstIndex(where: { $0.id == hid }) else { return }
        state.visit[v].result = "沒有送到：\(error)"
    }

    private mutating func record(_ it: Item, answer: String, result: String, question: String? = nil) {
        if case .decision(let d) = it.kind { state.answeredIDs.insert(d.id) }
        let entry = HistoryEntry(time: "剛剛", agent: it.agent, project: it.project, question: question ?? it.question,
                                 answer: answer, result: result, sessionID: it.sessionID)
        state.visit.append(entry)
        let snap = Snapshot(agent: it.agent, project: it.project, question: question ?? it.question)
        if let idx = state.timeline.lastIndex(where: { $0.exchange?.itemID == it.id && $0.exchange?.answer == nil }),
           case .exchange(var e) = state.timeline[idx] {
            e.answer = answer; e.snapshot = snap; e.historyID = entry.id
            state.timeline[idx] = .exchange(e)
        } else {
            // 從待辦清單直接回答別件：拿掉沒答的那件，補上這件
            if let idx = currentIndex { state.timeline.remove(at: idx) }
            state.timeline.append(.exchange(Exchange(id: nextID("x"), itemID: it.id, answer: answer, snapshot: snap, historyID: entry.id)))
        }
    }

    // MARK: 撤回

    private mutating func hold(_ snapshot: BoardState, effects: [Effect]) -> UndoToken {
        let t = UndoToken(raw: { seq += 1; return seq }())
        pending = (t, snapshot, effects)
        return t
    }

    public var pendingToken: UndoToken? { pending?.token }

    public mutating func undo(_ token: UndoToken) {
        guard let p = pending, p.token == token else { return }
        state = p.snapshot
        pending = nil
    }

    @discardableResult
    public mutating func commit(_ token: UndoToken) -> [Effect] {
        guard let p = pending, p.token == token else { return [] }
        pending = nil
        return p.effects
    }

    private mutating func commitPending() {
        if let p = pending { pending = nil; outbox.append(contentsOf: p.effects) }
    }

    public mutating func drainOutbox() -> [Effect] {
        defer { outbox = [] }
        return outbox
    }

    /// 這件答案還在撤回時間內嗎（答案底下寫「5 秒內可撤回」）。
    public func isUndoable(_ exchange: Exchange) -> Bool {
        guard pending != nil, let last = state.timeline.last(where: { $0.exchange?.answer != nil && $0.exchange?.skipped == false })?.exchange else { return false }
        return last.id == exchange.id
    }

    public func historyEntry(_ id: String?) -> HistoryEntry? {
        guard let id else { return nil }
        return state.visit.first { $0.id == id } ?? state.history.lazy.flatMap(\.entries).first { $0.id == id }
    }

    // MARK: 等下再說、先處理別件

    public mutating func skip() {
        guard let idx = currentIndex, case .exchange(var e) = state.timeline[idx] else { return }
        state.skipped.removeAll { $0 == e.itemID }
        state.skipped.append(e.itemID)
        e.answer = Engine.skipLabel
        if let it = item(id: e.itemID) { e.snapshot = Snapshot(agent: it.agent, project: it.project, question: it.question) }
        state.timeline[idx] = .exchange(e)
        askNext()
    }

    public mutating func jump(to itemID: String) {
        guard item(id: itemID) != nil else { return }
        if let idx = currentIndex, state.timeline[idx].exchange?.followUps.isEmpty == true { state.timeline.remove(at: idx) }
        else if let idx = currentIndex, case .exchange(var e) = state.timeline[idx] {
            e.answer = Engine.skipLabel
            state.timeline[idx] = .exchange(e)
        }
        state.skipped = items.map(\.id).filter { $0 != itemID }
        state.timeline.append(.exchange(Exchange(id: nextID("x"), itemID: itemID)))
    }

    // MARK: 追問（PRD §30.7）

    @discardableResult
    public mutating func followUp(_ kind: FollowUpKind) -> String {
        guard let idx = currentIndex, case .exchange(var e) = state.timeline[idx] else { return "" }
        let id = nextID("f")
        e.followUps.append(FollowUp(id: id, kind: kind, answer: nil, quote: nil, by: .relay, status: .thinking,
                                    signature: "軍師（\(state.relay.name)）"))
        state.timeline[idx] = .exchange(e)
        return id
    }

    public func followUp(_ id: String) -> FollowUp? {
        for entry in state.timeline { if let f = entry.exchange?.followUps.first(where: { $0.id == id }) { return f } }
        return nil
    }

    private mutating func updateFollowUp(_ id: String, _ change: (inout FollowUp) -> Void) {
        for (i, entry) in state.timeline.enumerated() {
            guard case .exchange(var e) = entry, let j = e.followUps.firstIndex(where: { $0.id == id }) else { continue }
            change(&e.followUps[j])
            state.timeline[i] = .exchange(e)
            return
        }
    }

    public mutating func resolveFollowUp(_ id: String, text: String, quote: [String]? = nil) {
        updateFollowUp(id) { $0.answer = text; $0.quote = quote; $0.status = .answered }
    }

    public mutating func failFollowUp(_ id: String, error: String) {
        updateFollowUp(id) { $0.answer = error; $0.status = .failed }
    }

    /// 這則追問是哪一題（送給 Mac 用）。
    public func questionID(ofFollowUp id: String) -> String? {
        guard let it = exchangeItem(ofFollowUp: id), case .decision(let d) = it.kind else { return nil }
        return d.id
    }

    /// Mac 送回來的中轉 AI 訊息（CONTRACT §17）。
    public mutating func apply(_ msg: [String: Any]) {
        let id = (msg["id"] as? String) ?? ""
        let ok = (msg["ok"] as? Bool) ?? false
        let text = (msg["text"] as? String) ?? ""
        let error = (msg["error"] as? String) ?? "沒有說原因"
        switch msg["type"] as? String {
        case "followup_answer":
            if ok { resolveFollowUp(id, text: text) } else { failFollowUp(id, error: error) }
        case "chat_reply":
            relayReply(ok ? text : "軍師沒有回應：\(error)")
        case "task_started":
            if ok { agentWorking(id) } else { agentFinished(id, text: "沒開成：\(error)") }
        case "task_done":
            agentFinished(id, text: ok ? text : "沒做完：\(text)")
        default:
            break
        }
    }

    private func exchangeItem(ofFollowUp id: String) -> Item? {
        for entry in state.timeline {
            if let e = entry.exchange, e.followUps.contains(where: { $0.id == id }) { return item(id: e.itemID) }
        }
        return nil
    }

    /// 「問 <專案> 本人」：只有中轉 AI 答完、而且這題有正在等的原對話時才有。
    public func canForward(_ id: String) -> Bool {
        guard state.forwardEnabled, let f = followUp(id), f.by == .relay, f.status == .answered, let it = exchangeItem(ofFollowUp: id) else { return false }
        if it.sessionID != nil { return true }
        if case .decision(let d) = it.kind { return d.hasConversation }
        return false
    }

    @discardableResult
    public mutating func forward(_ id: String) -> String {
        guard canForward(id), let f = followUp(id), let it = exchangeItem(ofFollowUp: id),
              let idx = state.timeline.lastIndex(where: { $0.exchange?.followUps.contains { $0.id == id } == true }),
              case .exchange(var e) = state.timeline[idx] else { return "" }
        let nid = nextID("f")
        e.followUps.append(FollowUp(id: nid, kind: f.kind, answer: nil, quote: nil, by: .agent, status: .thinking,
                                    signature: "\(it.project) 本人"))
        state.timeline[idx] = .exchange(e)
        return nid
    }

    public var typingTarget: TypingTarget {
        guard let it = currentItem else { return .relayChat }
        switch it.kind {
        case .decision(let d) where d.options.isEmpty && d.allowText: return .answerText(questionID: d.id)
        case .decision: return .followUp
        case .permission: return .disabled
        case .reply(let s): return .reply(sessionID: s.id)
        }
    }

    // MARK: 答完後跟中轉 AI 聊天

    public mutating func chat(_ text: String) {
        state.timeline.append(.chat(id: nextID("c"), .me(text)))
    }

    public mutating func relayThinking() {
        state.timeline.append(.chat(id: nextID("c"), .relayThinking))
    }

    public mutating func relayReply(_ text: String) {
        state.timeline.removeAll { $0.chat == .relayThinking }
        state.timeline.append(.chat(id: nextID("c"), .relay(text)))
    }

    @discardableResult
    public mutating func startTask(agent: Agent, project: String, text: String) -> String {
        let id = nextID("task-")
        state.sessions[id] = Session(id: id, name: project, agent: agent, status: .running("思考中"), turns: [.user(text)])
        state.visit.append(HistoryEntry(time: "剛剛", agent: agent, project: project, question: "你開了新任務", answer: text, result: "開始處理了。", sessionID: id))
        state.timeline.removeAll { $0.chat == .relayThinking }
        state.timeline.append(.chat(id: nextID("c"), .task(sessionID: id)))
        return id
    }

    // MARK: 原對話接手（服務層呼叫）

    public mutating func agentWorking(_ sessionID: String) {
        state.sessions[sessionID]?.status = .running("處理中")
    }

    public mutating func agentReplied(_ sessionID: String, tool: String?, reply: String) {
        guard state.sessions[sessionID] != nil else { return }
        if let tool { state.sessions[sessionID]?.turns.append(.tool(tool)) }
        state.sessions[sessionID]?.turns.append(.ai(reply))
        state.sessions[sessionID]?.status = .reply
        let first = reply.split(separator: "\n").first.map(String.init) ?? reply
        if let i = state.visit.lastIndex(where: { $0.sessionID == sessionID && ($0.result == Engine.pendingResult || $0.result == "執行中…") }) {
            state.visit[i].result = "AI：\(first)"
        }
        askNext()
    }

    /// 從手機開的任務跑完：記下最後的輸出，不變成待辦。
    public mutating func agentFinished(_ sessionID: String, text: String) {
        guard state.sessions[sessionID] != nil else { return }
        state.sessions[sessionID]?.turns.append(.ai(text))
        state.sessions[sessionID]?.status = .idle
        let first = text.split(separator: "\n").first.map(String.init) ?? text
        if let i = state.visit.lastIndex(where: { $0.sessionID == sessionID }) { state.visit[i].result = first }
        askNext()
    }

    // MARK: 設定

    public mutating func setRelay(_ agent: Agent) { state.relay = agent }
    public var headerTitle: String { "軍師 · \(state.relay.name)" }
}
