import Foundation
import UIKit

enum PairingPhase: Equatable {
    case idle
    case waiting
    case rejected(String)
    case revoked
}

/// 接上 Mac 的那一層（PRD §30.8；docs/design/mobile-relay.md）：配對、封包加解密、把 Kunsu 的題目轉成 App 的格式。
@MainActor
final class LiveLink {
    let identity: DeviceIdentity
    private let relay = RelayClient()
    private(set) var pairing: Pairing?
    private var pendingLink: PairingLink?
    private var lastSeq: UInt64 = 0
    private var drafts: [String: (Int?, String?)] = [:]

    var onSnapshot: (([Decision], [HistoryDay], Agent?, [String]) -> Void)?
    /// 中轉 AI 的回覆：followup_answer、chat_reply、task_started、task_done。
    var onRelay: (([String: Any]) -> Void)?
    /// (成功, 錯誤, 哪一題的答案)。
    var onAck: ((Bool, String?, String?) -> Void)?
    var onPairing: ((PairingPhase) -> Void)?
    var onConnection: ((Bool) -> Void)?

    init(identity: DeviceIdentity, pairing: Pairing?) {
        self.identity = identity
        self.pairing = pairing
        relay.onText = { [weak self] in self?.handle($0) }
        relay.onState = { [weak self] connected in
            guard let self else { return }
            self.onConnection?(connected)
            if connected, self.pairing != nil { self.send(["type": "sync"]) }
        }
    }

    private func url(relay: String, mac: String) -> URL? {
        URL(string: "\(relay)/connect?role=device&mac=\(mac)&dev=\(identity.deviceID)")
    }

    func connect() {
        guard let p = pairing, let u = url(relay: p.relay, mac: p.mac.id) else { return }
        relay.connect(u)
    }

    func disconnect() { relay.disconnect() }

    // MARK: 配對

    func startPairing(_ link: PairingLink) {
        pendingLink = link
        guard let u = url(relay: link.relay, mac: link.mac.id) else { return }
        relay.connect(u)
        do {
            let wire = try identity.pairRequest(mac: link.mac, code: link.code, deviceName: UIDevice.current.name,
                                                ts: Int64(Date().timeIntervalSince1970))
            let body = try JSONEncoder().encode(wire)
            let frame = "{\"body\":\(String(data: body, encoding: .utf8)!)}"
            relay.send(frame)
            onPairing?(.waiting)
        } catch {
            onPairing?(.rejected("配對請求產生失敗：\(error.localizedDescription)"))
        }
    }

    func unpair() {
        Pairing.clear()
        pairing = nil
        relay.disconnect()
    }

    // MARK: 送

    func send(_ command: [String: Any]) {
        guard let p = pairing, let plain = try? JSONSerialization.data(withJSONObject: command) else { return }
        let ms = UInt64(max(0, Date().timeIntervalSince1970 * 1000))
        lastSeq = max(lastSeq + 1, ms)
        guard let env = try? identity.seal(mac: p.mac, seq: lastSeq, ts: Int64(Date().timeIntervalSince1970), plaintext: plain),
              let body = try? JSONEncoder().encode(env) else { return }
        relay.send("{\"body\":\(String(data: body, encoding: .utf8)!)}")
    }

    func sendAnswer(questionID: String, choiceID: String?, text: String?, moneyConfirmed: Bool) {
        var cmd: [String: Any] = ["type": "answer", "question_id": questionID, "money_confirmed": moneyConfirmed]
        if let choiceID { cmd["choice_id"] = choiceID }
        if let text { cmd["text"] = text }
        if let (rev, hash) = drafts[questionID] {
            if let rev { cmd["revision"] = rev }
            if let hash { cmd["content_hash"] = hash }
        }
        send(cmd)
    }

    // MARK: 收

    private func handle(_ text: String) {
        guard let data = text.data(using: .utf8), let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { return }
        if let t = obj["t"] as? String {
            switch t {
            case "pair_accepted":
                guard let link = pendingLink, let acc = try? JSONDecoder().decode(PairAccepted.self, from: data),
                      identity.verify(acc, mac: link.mac) else { return }
                let p = Pairing(relay: link.relay, mac: link.mac, pairedAt: Date())
                p.save()
                pairing = p
                pendingLink = nil
                onPairing?(.idle)
                send(["type": "sync"])
            case "pair_rejected":
                onPairing?(.rejected((obj["reason"] as? String) ?? "配對被拒絕"))
            case "revoked":
                unpair()
                onPairing?(.revoked)
            default:
                break
            }
            return
        }
        guard let p = pairing, let env = try? JSONDecoder().decode(Envelope.self, from: data),
              let plain = try? identity.open(fromMac: p.mac, env),
              let msg = try? JSONSerialization.jsonObject(with: plain) as? [String: Any] else { return }
        switch msg["type"] as? String {
        case "snapshot":
            let questions = (msg["questions"] as? [[String: Any]]) ?? []
            let decisions = questions.compactMap(Self.decision)
            drafts = Dictionary(uniqueKeysWithValues: questions.compactMap { q -> (String, (Int?, String?))? in
                guard let id = q["id"] as? String else { return nil }
                return (id, (q["revision"] as? Int, q["content_hash"] as? String))
            })
            let history = Self.history((msg["history"] as? [[String: Any]]) ?? [])
            onSnapshot?(decisions, history, (msg["relay_agent"] as? String).flatMap(Agent.init(rawValue:)), (msg["projects"] as? [String]) ?? [])
        case "ack":
            onAck?((msg["ok"] as? Bool) ?? false, msg["error"] as? String,
                   msg["for"] as? String == "answer" ? msg["question_id"] as? String : nil)
        case "followup_answer", "chat_reply", "task_started", "task_done":
            onRelay?(msg)
        default:
            break
        }
    }

    // MARK: Kunsu 題目 → App

    private static let clock: DateFormatter = {
        let f = DateFormatter()
        f.dateFormat = "HH:mm"
        return f
    }()

    static func decision(_ q: [String: Any]) -> Decision? {
        guard let id = q["id"] as? String, let question = q["question"] as? String else { return nil }
        let kind = (q["kind"] as? String) ?? "choice"
        if kind == "batch" { return nil } // 題組請在電腦上逐項處理
        var options = ((q["options"] as? [[String: Any]]) ?? []).map {
            Option(($0["label"] as? String) ?? "", detail: $0["detail"] as? String, recommended: ($0["recommended"] as? Bool) ?? false,
                   id: $0["id"] as? String)
        }
        if options.isEmpty && kind == "confirm" { options = [Option("確認", id: "confirm")] }
        let money = (q["money"] as? [[String: Any]])?.map { MoneyParam(($0["label"] as? String) ?? "", ($0["value"] as? String) ?? "") }
        let asked = (q["asked_at"] as? Double).map { Date(timeIntervalSince1970: $0) } ?? Date()
        var d = Decision(
            id: id,
            agent: Agent(rawValue: (q["agent"] as? String) ?? "") ?? .claude,
            project: (q["project"] as? String) ?? "Kunsu",
            sessionID: nil,
            question: question,
            why: (q["why"] as? String) ?? "",
            options: options,
            money: money,
            time: clock.string(from: asked),
            allowText: options.isEmpty && ((q["allow_text"] as? Bool) ?? false),
            context: (q["context"] as? [String]) ?? [],
            revision: q["revision"] as? Int,
            contentHash: q["content_hash"] as? String
        )
        d.hasConversation = (q["has_conversation"] as? Bool) ?? false
        return d
    }

    static func history(_ rows: [[String: Any]]) -> [HistoryDay] {
        var days: [HistoryDay] = []
        for r in rows {
            let entry = HistoryEntry(
                id: (r["id"] as? String) ?? UUID().uuidString,
                time: (r["time"] as? String) ?? "",
                agent: Agent(rawValue: (r["agent"] as? String) ?? "") ?? .claude,
                project: (r["project"] as? String) ?? "",
                question: (r["question"] as? String) ?? "",
                answer: (r["answer"] as? String) ?? "",
                result: (r["result"] as? String) ?? ""
            )
            let day = (r["day"] as? String) ?? "稍早"
            if let i = days.firstIndex(where: { $0.name == day }) { days[i].entries.append(entry) } else { days.append(HistoryDay(day, [entry])) }
        }
        // Mac 送來的是新到舊；畫面上同一天由舊到新往下排
        return days.map { HistoryDay($0.name, $0.entries.reversed()) }
    }
}
