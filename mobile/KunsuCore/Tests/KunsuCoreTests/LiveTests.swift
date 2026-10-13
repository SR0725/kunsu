import XCTest
@testable import KunsuCore

/// 接上 Mac 之後（PRD §30.8）：答案變成送給 Mac 的效果；Mac 的快照取代題目但不吃掉剛答的。
final class LiveTests: XCTestCase {
    private func live() -> Engine {
        var s = Fixtures.tonight()
        s.sessions = [:]
        s.decisions = s.decisions.map { d in
            var d = d
            d.sessionID = nil
            d.options = d.options.enumerated().map { i, o in var o = o; o.id = "opt\(i)"; return o }
            return d
        }
        s.forwardEnabled = false
        return Engine(state: s)
    }

    func test_committing_an_answer_sends_it_to_the_mac() {
        var e = live()
        guard case .queued(let t)? = e.answerDecision("notch", option: 1) else { return XCTFail() }
        XCTAssertEqual(e.commit(t), [.answer(questionID: "notch", choiceID: "opt1", text: nil, moneyConfirmed: false)])
    }

    func test_money_is_sent_immediately_with_confirmation() {
        var e = live()
        XCTAssertEqual(e.answerDecision("budget", option: 0, confirmed: true), .executed)
        XCTAssertEqual(e.drainOutbox(), [.answer(questionID: "budget", choiceID: "opt0", text: nil, moneyConfirmed: true)])
    }

    func test_undo_never_sends() {
        var e = live()
        guard case .queued(let t)? = e.answerDecision("notch", option: 0) else { return XCTFail() }
        e.undo(t)
        XCTAssertEqual(e.drainOutbox(), [])
        XCTAssertEqual(e.commit(t), [])
    }

    func test_snapshot_replaces_questions_but_keeps_what_was_just_answered_out() {
        var e = live()
        _ = e.answerDecision("notch", option: 0)
        var fresh = e.state.decisions
        fresh.append(Decision(id: "new", agent: .codex, project: "kunsu", sessionID: nil, question: "新題", why: "", options: [Option("好", id: "y")], time: "剛剛"))
        let all = [Fixtures.tonight().decisions[0]] + fresh
        e.applySnapshot(decisions: all, history: [HistoryDay("今天", [])])
        XCTAssertFalse(e.items.contains { $0.id == "d:notch" }, "剛答、還在撤回窗的題不被快照加回來")
        XCTAssertTrue(e.items.contains { $0.id == "d:new" })
        XCTAssertEqual(e.state.history.first?.name, "今天")
    }

    func test_question_answered_elsewhere_disappears_from_current() {
        var e = live()
        XCTAssertEqual(e.current?.itemID, "d:notch")
        e.applySnapshot(decisions: e.state.decisions.filter { $0.id != "notch" }, history: [])
        XCTAssertEqual(e.current?.itemID, "d:hook")
    }

    func test_text_questions_are_answered_by_typing() {
        var e = live()
        e.applySnapshot(decisions: [Decision(id: "t", agent: .claude, project: "blog", sessionID: nil, question: "標題要寫什麼？", why: "", options: [], time: "剛剛", allowText: true)], history: [])
        XCTAssertEqual(e.typingTarget, .answerText(questionID: "t"))
        guard let t = e.answerText("t", text: "就叫一人公司") else { return XCTFail() }
        XCTAssertEqual(e.commit(t), [.answer(questionID: "t", choiceID: nil, text: "就叫一人公司", moneyConfirmed: false)])
    }

    func test_forward_is_off_until_the_relay_ai_is_connected() {
        var e = live()
        let f = e.followUp(.why)
        e.resolveFollowUp(f, text: "x")
        XCTAssertFalse(e.canForward(f))
    }

    /// 動錢題的每個選項（包含「維持」）都要先過核對畫面；Mac 只收 money_confirmed 的錢題答案。
    func test_every_option_on_a_money_question_goes_through_the_confirm_sheet() {
        var e = live()
        XCTAssertEqual(e.answerDecision("budget", option: 1), .needsConfirm)
        XCTAssertEqual(e.drainOutbox(), [])
        XCTAssertEqual(e.answerDecision("budget", option: 1, confirmed: true), .sent)
        XCTAssertEqual(e.drainOutbox(), [.answer(questionID: "budget", choiceID: "opt1", text: nil, moneyConfirmed: true)])
    }

    /// Mac 拒收答案：那則答案寫「沒有送到」，題目回到待辦（下一份快照會把它帶回來）。
    func test_rejected_answer_is_shown_and_the_question_comes_back() {
        var e = live()
        guard case .queued(let t)? = e.answerDecision("notch", option: 0) else { return XCTFail() }
        _ = e.commit(t)
        let before = e.state.decisions
        e.answerRejected("notch", error: "這題已經在別處回答")
        XCTAssertEqual(e.state.visit.last?.result, "沒有送到：這題已經在別處回答")
        XCTAssertFalse(e.state.answeredIDs.contains("notch"))
        e.applySnapshot(decisions: Fixtures.tonight().decisions.map { d in var d = d; d.sessionID = nil; return d }, history: [])
        XCTAssertTrue(e.items.contains { $0.id == "d:notch" })
        _ = before
    }
}
