import XCTest
@testable import KunsuCore

/// 手機的中轉 AI、追問、問本人、聊天、開新任務（BDD mobile-relay-ai AC-RA-09）。
final class RelayAITests: XCTestCase {
    private func live(conversation: Bool = true) -> Engine {
        var s = Fixtures.tonight()
        s.sessions = [:]
        s.decisions = s.decisions.map { d in
            var d = d
            d.sessionID = nil
            d.hasConversation = conversation
            return d
        }
        s.forwardEnabled = true
        return Engine(state: s)
    }

    func test_followup_becomes_a_command_and_the_reply_lands_on_it() {
        var e = live()
        let qid = e.currentItem!.id.replacingOccurrences(of: "d:", with: "")
        let fid = e.followUp(.why)
        XCTAssertEqual(RelayCommand.followUp(id: fid, questionID: qid, kind: .why),
                       ["type": "followup", "id": fid, "question_id": qid, "kind": "why"])
        XCTAssertEqual(RelayCommand.followUp(id: fid, questionID: qid, kind: .text("含稅嗎？")),
                       ["type": "followup", "id": fid, "question_id": qid, "kind": "text", "text": "含稅嗎？"])
        e.apply(["type": "followup_answer", "id": fid, "ok": true, "by": "relay", "text": "因為轉換掉了一半。"])
        XCTAssertEqual(e.followUp(fid)?.answer, "因為轉換掉了一半。")
        XCTAssertEqual(e.followUp(fid)?.status, .answered)
    }

    func test_relay_error_is_shown_not_spinning_forever() {
        var e = live()
        let fid = e.followUp(.context)
        e.apply(["type": "followup_answer", "id": fid, "ok": false, "error": "軍師沒有回應（逾時 180 秒）"])
        XCTAssertEqual(e.followUp(fid)?.status, .failed)
        XCTAssertEqual(e.followUp(fid)?.answer, "軍師沒有回應（逾時 180 秒）")
    }

    func test_ask_the_agent_itself_only_when_the_question_has_a_conversation() {
        var e = live(conversation: true)
        let fid = e.followUp(.why)
        e.apply(["type": "followup_answer", "id": fid, "ok": true, "text": "x"])
        XCTAssertTrue(e.canForward(fid))
        let qid = e.questionID(ofFollowUp: fid)!
        let nid = e.forward(fid)
        XCTAssertEqual(RelayCommand.forward(id: nid, questionID: qid, text: "為什麼？"),
                       ["type": "forward", "id": nid, "question_id": qid, "text": "為什麼？"])
        e.apply(["type": "followup_answer", "id": nid, "ok": true, "by": "agent", "text": "含稅。"])
        XCTAssertEqual(e.followUp(nid)?.answer, "含稅。")
        XCTAssertEqual(e.followUp(nid)?.by, .agent)

        var e2 = live(conversation: false)
        let f2 = e2.followUp(.why)
        e2.apply(["type": "followup_answer", "id": f2, "ok": true, "text": "x"])
        XCTAssertFalse(e2.canForward(f2), "沒有原對話就沒有「問本人」")
    }

    func test_chat_reply_and_error() {
        var e = live()
        e.chat("大家在做什麼？")
        e.relayThinking()
        XCTAssertEqual(RelayCommand.chat(id: "c1", text: "大家在做什麼？"), ["type": "chat", "id": "c1", "text": "大家在做什麼？"])
        e.apply(["type": "chat_reply", "id": "c1", "ok": true, "text": "kunsu 在等你回一題。"])
        XCTAssertEqual(e.state.timeline.last?.chat, .relay("kunsu 在等你回一題。"))
        e.relayThinking()
        e.apply(["type": "chat_reply", "id": "c2", "ok": false, "error": "軍師沒有回應（逾時 180 秒）"])
        XCTAssertEqual(e.state.timeline.last?.chat, .relay("軍師沒有回應：軍師沒有回應（逾時 180 秒）"))
        XCTAssertFalse(e.state.timeline.contains { $0.chat == .relayThinking })
    }

    func test_new_task_lifecycle() {
        var e = live()
        let sid = e.startTask(agent: .codex, project: "kunsu", text: "補 README")
        XCTAssertEqual(RelayCommand.startTask(id: sid, agent: .codex, project: "kunsu", text: "補 README"),
                       ["type": "start_task", "id": sid, "agent": "codex", "project": "kunsu", "text": "補 README"])
        e.apply(["type": "task_started", "id": sid, "ok": true])
        e.apply(["type": "task_done", "id": sid, "ok": true, "text": "改了 2 個檔。"])
        XCTAssertEqual(e.state.sessions[sid]?.turns.last, .ai("改了 2 個檔。"))
        XCTAssertEqual(e.state.sessions[sid]?.status, .idle, "跑完不會變成等你回話（手機回覆對話是下一階段）")
        XCTAssertFalse(e.items.contains { $0.sessionID == sid })

        let bad = e.startTask(agent: .claude, project: "nope", text: "x")
        e.apply(["type": "task_started", "id": bad, "ok": false, "error": "Mac 上找不到這個專案"])
        XCTAssertEqual(e.state.sessions[bad]?.turns.last, .ai("沒開成：Mac 上找不到這個專案"))
    }

    func test_snapshot_brings_the_macs_projects() {
        var e = live()
        e.applySnapshot(decisions: [], history: [], projects: ["demo-shop", "kunsu"])
        XCTAssertEqual(e.state.projects, ["demo-shop", "kunsu"])
        e.applySnapshot(decisions: [], history: [], projects: [])
        XCTAssertEqual(e.state.projects, ["demo-shop", "kunsu"], "空的不蓋掉")
    }
}
