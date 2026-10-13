import SwiftUI
import Observation

enum Appearance: String, CaseIterable, Identifiable {
    case system, light, dark
    var id: String { rawValue }
    var label: String { self == .system ? "跟隨系統" : self == .light ? "淺色" : "深色" }
    var scheme: ColorScheme? { self == .system ? nil : self == .light ? .light : .dark }
}

/// 送出後 5 秒的撤回窗。
struct PendingSend: Equatable {
    let token: UndoToken
    let label: String
    let id = UUID()
}

/// App 的服務層：包住 KunsuCore 的 Engine，負責時間與 AI 回覆。
/// 現在是模擬（MockAgents）；接上 Mac 上的 Kunsu 時，只換掉這裡的模擬回覆與送出（PRD §30.7 階段）。
@MainActor
@Observable
final class AppModel {
    private(set) var engine: Engine
    var pendingSend: PendingSend?
    var moneySheet: (decisionID: String, option: Int)?
    var receipt: String?
    var showSettings = false
    var showNewTask = false
    var appearance: Appearance { didSet { UserDefaults.standard.set(appearance.rawValue, forKey: "appearance") } }
    var macOnline = true

    enum Mode { case demo, live }
    private(set) var mode: Mode = .demo
    /// 還沒配對、也沒選示範模式：顯示掃描畫面（AC-MP-09）。
    var needsPairing = false
    let account = AccountModel()
    /// 還沒登入、還沒配對、也沒選示範資料：先登入（AC-MU-04）。已經配對過的手機不強迫登入。
    var needsLogin: Bool { (needsPairing || wantsLogin) && !account.loggedIn }
    /// 已配對的手機從設定按「登入」。
    var wantsLogin = false
    var pairingPhase: PairingPhase = .idle
    var notice: String?
    private var link: LiveLink?
    /// 送給 Mac 的 id 加上這次啟動的前綴，重開 App 後不會撞到舊的。
    private let runID = String(UUID().uuidString.prefix(6))

    private var undoTask: Task<Void, Never>?

    init(state: BoardState = Fixtures.tonight()) {
        var s = state
        if let raw = UserDefaults.standard.string(forKey: "relay"), let a = Agent(rawValue: raw) { s.relay = a }
        engine = Engine(state: s)
        appearance = Appearance(rawValue: UserDefaults.standard.string(forKey: "appearance") ?? "") ?? .system
        if let identity = try? IdentityStore.loadOrCreate() {
            let pairing = Pairing.load()
            let link = LiveLink(identity: identity, pairing: pairing)
            self.link = link
            wire(link)
            // 測試用：啟動參數 -accountBase http://127.0.0.1:8787 指到本機的帳號服務
            let override = UserDefaults.standard.string(forKey: "accountBase")
            account.baseProvider = { [weak link] in override ?? link?.pairing.map { AccountAPI.base(relay: $0.relay) } ?? AccountAPI.defaultBase }
            if pairing != nil { goLive() } else { needsPairing = true }
        }
        // 截圖用：啟動參數 -demo YES 直接進示範資料
        if UserDefaults.standard.bool(forKey: "demo") { useDemo() }
    }

    // MARK: 接上 Mac（PRD §30.8）

    var pairing: Pairing? { link?.pairing }
    var deviceID: String { link?.identity.deviceID ?? "" }

    private func wire(_ link: LiveLink) {
        link.onConnection = { [weak self] c in self?.macOnline = c }
        link.onSnapshot = { [weak self] decisions, history, relay, projects in
            guard let self, self.mode == .live else { return }
            self.mutate { e in
                if let relay { e.setRelay(relay) }
                e.applySnapshot(decisions: decisions, history: history, projects: projects)
            }
        }
        link.onRelay = { [weak self] msg in
            guard let self, self.mode == .live else { return }
            var m = msg
            if let id = m["id"] as? String, id.hasPrefix("\(self.runID)-") { m["id"] = String(id.dropFirst(self.runID.count + 1)) }
            self.mutate { $0.apply(m) }
        }
        link.onAck = { [weak self] ok, error, questionID in
            guard let self, !ok else { return }
            let why = error ?? "沒有說原因"
            self.notice = "沒有送成功：\(why)"
            // AC-MA-13：那則答案寫「沒有送到」，題目回到待辦
            if let questionID {
                self.mutate { $0.answerRejected(questionID, error: why) }
                self.link?.send(["type": "sync"])
            }
        }
        link.onPairing = { [weak self] phase in
            guard let self else { return }
            self.pairingPhase = phase
            switch phase {
            case .idle where self.link?.pairing != nil:
                self.needsPairing = false
                self.goLive()
                UINotificationFeedbackGenerator().notificationOccurred(.success)
            case .revoked:
                self.mode = .demo
                self.needsPairing = true
            default: break
            }
        }
    }

    private func goLive() {
        var s = BoardState(decisions: [], sessions: [:], history: [], relay: engine.state.relay)
        s.forwardEnabled = true
        engine = Engine(state: s)
        mode = .live
        macOnline = false
        link?.connect()
    }

    /// 掃到或貼上的配對連結。
    func pair(with text: String) -> Bool {
        guard let l = PairingLink(text) else { return false }
        link?.startPairing(l)
        return true
    }

    func useDemo() {
        wantsLogin = false
        needsPairing = false
        mode = .demo
        engine = Engine(state: Fixtures.tonight())
        macOnline = true
    }

    /// 從設定按「登入」：回到登入畫面（配對留著，登入後回到對話）。
    func startLogin() {
        wantsLogin = true
    }

    func unpair() {
        link?.unpair()
        useDemo()
        needsPairing = true
        pairingPhase = .idle
    }

    // MARK: 回答

    func tap(_ button: AnswerButton) {
        switch button.action {
        case .decision(let id, let option): answerDecision(id, option: option)
        case .permission(let sid, let allow): permit(sid, allow: allow)
        case .reply(let sid, let text): reply(sid, text: text, label: button.label)
        }
    }

    func answerDecision(_ id: String, option: Int, confirmed: Bool = false) {
        let label = engine.state.decisions.first { $0.id == id }?.options[option].label ?? ""
        switch mutate({ $0.answerDecision(id, option: option, confirmed: confirmed) }) {
        case .needsConfirm?:
            moneySheet = (id, option)
        case .executed?:
            moneySheet = nil
            receipt = label
            haptic(.success)
        case .sent?:
            moneySheet = nil
            haptic(.success)
        case .queued(let t)?:
            holdUndo(t, label: "「\(label)」")
        case nil:
            break
        }
    }

    func reply(_ sid: String, text: String, label: String? = nil) {
        guard let t = mutate({ $0.sendReply(sid, text: text) }) else { return }
        holdUndo(t, label: "「\(label ?? text)」")
    }

    func permit(_ sid: String, allow: Bool) {
        mutate { $0.permit(sid, allow: allow) }
        haptic(allow ? .success : .warning)
        guard allow else { return }
        after(1.8) { m in
            m.mutate { $0.agentReplied(sid, tool: "執行完成 ✓", reply: "推上去了。要開 PR 嗎？\nA. 開 PR\nB. 先不用") }
        }
    }

    func skip() { mutate { $0.skip() } }
    func jump(_ id: String) { mutate { $0.jump(to: id) } }

    // MARK: 撤回

    private func holdUndo(_ token: UndoToken, label: String) {
        UIImpactFeedbackGenerator(style: .light).impactOccurred()
        undoTask?.cancel()
        withAnimation(.snappy) { pendingSend = PendingSend(token: token, label: label) }
        undoTask = Task { [weak self] in
            try? await Task.sleep(for: .seconds(5))
            guard !Task.isCancelled else { return }
            self?.commitPending()
        }
    }

    func undo() {
        guard let p = pendingSend else { return }
        undoTask?.cancel()
        UIImpactFeedbackGenerator(style: .rigid).impactOccurred()
        withAnimation(.snappy) {
            engine.undo(p.token)
            pendingSend = nil
        }
    }

    private func commitPending() {
        guard let p = pendingSend else { return }
        let effects = engine.commit(p.token)
        withAnimation(.snappy) { pendingSend = nil }
        run(effects)
    }

    // MARK: 追問（模擬中轉 AI 與原 AI）

    func followUp(_ kind: FollowUpKind) {
        guard let it = engine.currentItem else { return }
        let id = mutate { $0.followUp(kind) }
        if mode == .live {
            guard let qid = engine.questionID(ofFollowUp: id) else {
                mutate { $0.failFollowUp(id, error: "這一件不是 Kunsu 的題目，軍師答不了") }
                return
            }
            link?.send(RelayCommand.followUp(id: wire(id), questionID: qid, kind: kind))
            return
        }
        let (text, quote) = MockAgents.relayAnswer(kind, item: it, sessions: engine.state.sessions)
        after(1.0) { m in m.mutate { $0.resolveFollowUp(id, text: text, quote: quote) } }
    }

    func forward(_ id: String) {
        guard let it = engine.currentItem else { return }
        let asked = engine.followUp(id)?.question ?? ""
        let nid = mutate { $0.forward(id) }
        if mode == .live {
            guard let qid = engine.questionID(ofFollowUp: nid) else { return }
            link?.send(RelayCommand.forward(id: wire(nid), questionID: qid, text: asked))
            return
        }
        after(2.2) { m in m.mutate { $0.resolveFollowUp(nid, text: MockAgents.agentAnswer(item: it)) } }
    }

    private func wire(_ id: String) -> String { "\(runID)-\(id)" }

    // MARK: 聊天與新任務

    func send(_ text: String) {
        let t = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !t.isEmpty else { return }
        switch engine.typingTarget {
        case .answerText(let qid):
            if let token = mutate({ $0.answerText(qid, text: t) }) { holdUndo(token, label: "「\(t)」") }
        case .reply(let sid): reply(sid, text: t)
        case .followUp: followUp(.text(t))
        case .disabled: break
        case .relayChat: chat(t)
        }
    }

    func chat(_ text: String) {
        mutate { $0.chat(text); $0.relayThinking() }
        if mode == .live {
            link?.send(RelayCommand.chat(id: wire(UUID().uuidString), text: text))
            return
        }
        if text.contains("新任務") || text.hasPrefix("開") {
            after(0.8) { m in
                m.mutate { $0.relayReply("好，要給誰、在哪個專案？我幫你開。") }
                m.showNewTask = true
            }
            return
        }
        let answer = MockAgents.relayChat(text, state: engine.state)
        after(1.1) { m in m.mutate { $0.relayReply(answer) } }
    }

    func startTask(agent: Agent, project: String, text: String) {
        let sid = mutate { $0.startTask(agent: agent, project: project, text: text) }
        if mode == .live {
            link?.send(RelayCommand.startTask(id: wire(sid), agent: agent, project: project, text: text))
            return
        }
        after(3.0) { m in
            m.mutate { $0.agentReplied(sid, tool: "讀了 5 個檔", reply: "看完了。我打算先\(text)，大概 15 分鐘。要開始嗎？\nA. 開始\nB. 先給我計畫") }
        }
    }

    // MARK: 其他

    func reopen() {
        undoTask?.cancel()
        if let p = pendingSend { run(engine.commit(p.token)); pendingSend = nil }
        mutate { $0.openVisit() }
        if mode == .live { link?.send(["type": "sync"]) }
    }

    func releasePull(_ distance: Double) -> Bool { mutate { $0.releasePull(distance: distance) } }

    func setRelay(_ agent: Agent) {
        mutate { $0.setRelay(agent) }
        if mode == .live { link?.send(["type": "settings", "relay_agent": agent.rawValue]) }
        UserDefaults.standard.set(agent.rawValue, forKey: "relay")
    }

    private func run(_ effects: [Effect]) {
        for effect in effects {
            switch effect {
            case .answer(let qid, let choice, let text, let money):
                if mode == .live { link?.sendAnswer(questionID: qid, choiceID: choice, text: text, moneyConfirmed: money) }
            case .resume(let sid):
                guard mode == .demo else { continue }
                mutate { $0.agentWorking(sid) }
                after(2.4) { m in m.mutate { $0.agentReplied(sid, tool: "讀了 3 個檔 · 改了 1 個檔", reply: MockAgents.nextStep) } }
            }
        }
    }

    @discardableResult
    private func mutate<T>(_ change: (inout Engine) -> T) -> T {
        var result: T!
        withAnimation(.snappy) { result = change(&engine) }
        let extra = engine.drainOutbox()
        if !extra.isEmpty { run(extra) }
        return result
    }

    private func after(_ seconds: Double, _ work: @escaping @MainActor (AppModel) -> Void) {
        Task { [weak self] in
            try? await Task.sleep(for: .seconds(seconds))
            guard let self else { return }
            work(self)
        }
    }

    private func haptic(_ type: UINotificationFeedbackGenerator.FeedbackType) {
        UINotificationFeedbackGenerator().notificationOccurred(type)
    }
}

/// 模擬的 AI 回覆；接上 Mac 後由中轉 AI 與原對話真的回答。
enum MockAgents {
    static let nextStep = "好，這步處理完了。下一步要做什麼？\nA. 跑完整測試\nB. 先停在這裡"

    static func relayAnswer(_ kind: FollowUpKind, item: Item, sessions: [String: Session]) -> (String, [String]?) {
        switch kind {
        case .why:
            switch item.kind {
            case .decision(let d):
                let r = d.options.first { $0.recommended }
                return ("AI 建議「\(r?.label ?? "")」。\(r?.detail.map { $0 + "。" } ?? "")\(d.why)", nil)
            case .permission(let s): return (s.permission?.reason ?? "它要執行這個指令才能繼續。", nil)
            case .reply: return ("它剛做完上一步，下一步要你決定。", nil)
            }
        case .context:
            guard let sid = item.sessionID, let s = sessions[sid] else {
                return ("\(item.project)：這題是排程直接問你的，沒有對應的對話。", nil)
            }
            let lines = s.turns.suffix(4).map { t -> String in
                switch t {
                case .user(let x): "你：\(x)"
                case .ai(let x): "AI：\(x.split(separator: "\n").first.map(String.init) ?? x)"
                case .tool(let x): "⚙︎ \(x)"
                case .system(let x): x
                }
            }
            return ("\(item.project) · \(item.agent.name) 最近的對話：", lines)
        case .text(let q):
            return ("你問「\(q)」：我看了題目的脈絡，AI 的建議是照它的第一個選項做；如果你要更細的原因，可以直接問它本人。", nil)
        }
    }

    static func agentAnswer(item: Item) -> String {
        if case .decision(let d) = item.kind, d.id == "notch" {
            return "桌面版已經是 L 形吸附，手機和桌面不一致的話你拖的時候會猶豫。我會順便把 30% 這個距離做成設定。"
        }
        return "我會照你選的做，做完回報結果。"
    }

    static func relayChat(_ text: String, state: BoardState) -> String {
        let running = state.sessions.values.filter { if case .running = $0.status { true } else { false } }
        if text.contains("在做什麼") || text.contains("進度") {
            if running.isEmpty { return "現在沒有 AI 在跑。" }
            return running.map { s -> String in
                if case .running(let what) = s.status { return "\(s.name)（\(s.agent.name)）：\(what)" }
                return s.name
            }.joined(separator: "\n")
        }
        if text.contains("今天") {
            let n = state.visit.count + (state.history.first?.entries.count ?? 0)
            return "今晚你處理了 \(n) 件事。往下拉可以看每一件的答案和 AI 的回報。"
        }
        return "收到。我可以幫你看各個 AI 的進度、開新任務，或把話轉給某段對話。"
    }
}
