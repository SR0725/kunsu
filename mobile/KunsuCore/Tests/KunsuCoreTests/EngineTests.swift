import XCTest
@testable import KunsuCore

/// BDD docs/bdd/mobile-app.md AC-MA-01～11。
final class EngineTests: XCTestCase {
    private func engine() -> Engine { Engine(state: Fixtures.tonight()) }

    // AC-MA-01
    func test_AC_MA_01_opening_shows_no_history() {
        let e = engine()
        XCTAssertEqual(e.state.loadedDays, 0)
        XCTAssertTrue(e.visibleHistory.isEmpty)
        XCTAssertNotNil(e.current)
    }

    // AC-MA-02
    func test_AC_MA_02_pull_loads_a_day_only_when_full() {
        var e = engine()
        XCTAssertEqual(Pull.phase(distance: 40, more: true), .pulling(progress: 40 / Pull.threshold))
        XCTAssertEqual(Pull.hint(.pulling(progress: 0.4)), "向下拖出歷史紀錄")
        XCTAssertEqual(Pull.hint(.idle), "向下拖出歷史紀錄")
        XCTAssertEqual(Pull.hint(.ready), "放開看歷史紀錄")
        XCTAssertEqual(Pull.hint(.noMore), "沒有更早的紀錄了")
        XCTAssertFalse(e.releasePull(distance: Pull.threshold - 1))
        XCTAssertEqual(e.state.loadedDays, 0)
        XCTAssertTrue(e.releasePull(distance: Pull.threshold))
        XCTAssertEqual(e.state.loadedDays, 1)
        XCTAssertFalse(e.visibleHistory.isEmpty)
        while e.hasMoreHistory { _ = e.releasePull(distance: Pull.threshold) }
        XCTAssertEqual(Pull.phase(distance: 200, more: e.hasMoreHistory), .noMore)
        XCTAssertFalse(e.releasePull(distance: Pull.threshold))
    }

    // AC-MA-03
    func test_AC_MA_03_order_is_decision_then_permission_then_reply_and_skip_goes_last() {
        var e = engine()
        XCTAssertEqual(e.items.map(\.kindName), ["decision", "decision", "decision", "permission", "reply"])
        let first = e.current!.itemID
        e.skip()
        XCTAssertEqual(e.items.last?.id, first)
        XCTAssertNotEqual(e.current?.itemID, first)
    }

    // AC-MA-04
    func test_AC_MA_04_answer_layout() {
        XCTAssertEqual(AnswerLayout.of(labels: ["允許一次", "拒絕"], hasDetail: false), .twoColumns)
        XCTAssertEqual(AnswerLayout.of(labels: ["commit 並安裝", "先給我看 diff"], hasDetail: false), .twoColumns)
        XCTAssertEqual(AnswerLayout.of(labels: ["吸附成 L 形", "滑回直邊"], hasDetail: true), .stacked)
        XCTAssertEqual(AnswerLayout.of(labels: ["我寫了 8 年程式，最後發現最難的不是技術", "B"], hasDetail: false), .stacked)
        XCTAssertEqual(AnswerLayout.of(labels: ["a", "b", "c", "d", "e"], hasDetail: false), .stacked)
        XCTAssertEqual(AnswerLayout.of(labels: ["繼續"], hasDetail: false), .stacked)
    }

    // AC-MA-05
    func test_AC_MA_05_option_extraction() {
        let opts = OptionParser.parse("測試全綠了。要我 commit 嗎？\nA. commit 並安裝\nB. 先給我看 diff")
        XCTAssertEqual(opts.map(\.key), ["A", "B"])
        XCTAssertEqual(opts.map(\.label), ["commit 並安裝", "先給我看 diff"])
        XCTAssertEqual(OptionParser.body("要嗎？\nA. 好\nB. 不要"), "要嗎？")
        var e = engine()
        e.state.sessions["kunsu"]!.turns.append(.ai("改好了，要繼續嗎？"))
        XCTAssertEqual(e.answers(for: e.item(id: "r:kunsu")!).map(\.label), ["繼續"])
    }

    // AC-MA-06
    func test_AC_MA_06_undo_restores_everything() {
        var e = engine()
        let before = e.state
        let token = e.answerDecision("notch", option: 0)
        guard case .queued(let t)? = token else { return XCTFail("non-money answer should be undoable") }
        XCTAssertEqual(e.items.count, before.decisions.count + 1)
        XCTAssertEqual(e.state.timeline.compactMap(\.exchange).first?.answer, "吸附成 L 形")
        e.undo(t)
        XCTAssertEqual(e.state, before)
    }

    // AC-MA-07
    func test_AC_MA_07_money_recommended_option_needs_confirm_and_no_face_id() {
        var e = engine()
        XCTAssertEqual(e.answerDecision("budget", option: 0), .needsConfirm)
        XCTAssertNotNil(e.state.decisions.first { $0.id == "budget" })
        XCTAssertEqual(e.answerDecision("budget", option: 0, confirmed: true), .executed)
        XCTAssertNil(e.state.decisions.first { $0.id == "budget" })
        // 不執行的選項也要先核對（Mac 只收經過核對畫面的錢題答案）；送出後錢不會動
        var f = engine()
        XCTAssertEqual(f.answerDecision("budget", option: 1), .needsConfirm)
        XCTAssertEqual(f.answerDecision("budget", option: 1, confirmed: true), .sent)
    }

    // AC-MA-08
    func test_AC_MA_08_follow_up_never_answers_and_forward_needs_a_session() {
        var e = engine()
        let id = e.current!.itemID
        let fu = e.followUp(.why)
        XCTAssertEqual(e.current?.itemID, id)
        XCTAssertNil(e.current?.answer)
        e.resolveFollowUp(fu, text: "因為桌面一致")
        XCTAssertTrue(e.canForward(fu))
        let forwarded = e.forward(fu)
        XCTAssertEqual(e.followUp(forwarded)?.by, .agent)
        XCTAssertEqual(e.followUp(forwarded)?.status, .thinking)
        // 沒有原對話的題（電子報排程）不能問本人
        e.jump(to: "d:hook")
        let lone = e.followUp(.why)
        e.resolveFollowUp(lone, text: "x")
        XCTAssertFalse(e.canForward(lone))
    }

    // AC-MA-09
    func test_AC_MA_09_typing_target_by_kind() {
        var e = engine()
        XCTAssertEqual(e.typingTarget, .followUp)
        e.jump(to: "p:threads")
        XCTAssertEqual(e.typingTarget, .disabled)
        e.jump(to: "r:kunsu")
        XCTAssertEqual(e.typingTarget, .reply(sessionID: "kunsu"))
    }

    // AC-MA-10
    func test_AC_MA_10_chat_mode_when_nothing_waits_and_tasks_make_cards() {
        var e = Engine(state: Fixtures.quiet())
        XCTAssertNil(e.current)
        XCTAssertEqual(e.typingTarget, .relayChat)
        e.chat("開新任務")
        XCTAssertEqual(e.state.timeline.last?.chat, .me("開新任務"))
        let sid = e.startTask(agent: .codex, project: "kunsu", text: "整理 README")
        XCTAssertEqual(e.state.timeline.last?.chat, .task(sessionID: sid))
        XCTAssertEqual(e.state.sessions[sid]?.status, .running("思考中"))
    }

    // AC-MA-11
    func test_AC_MA_11_relay_setting_names_header_and_follow_ups() {
        var e = engine()
        e.setRelay(.codex)
        XCTAssertEqual(e.headerTitle, "軍師 · Codex")
        let fu = e.followUp(.why)
        XCTAssertEqual(e.followUp(fu)?.signature, "軍師（Codex）")
    }

    // 回答後下一件接上、待辦數減一（場景 3）
    func test_answer_moves_on_and_pending_count_drops() {
        var e = engine()
        let n = e.items.count
        guard case .queued(let t)? = e.answerDecision("notch", option: 0) else { return XCTFail() }
        e.commit(t)
        XCTAssertEqual(e.items.count, n - 1)
        XCTAssertEqual(e.current?.itemID, "d:hook")
    }

    // 重新打開：這次的答案進入今晚紀錄，畫面再次乾淨
    func test_reopen_moves_answers_to_history_and_starts_clean() {
        var e = engine()
        guard case .queued(let t)? = e.answerDecision("notch", option: 0) else { return XCTFail() }
        e.commit(t)
        e.openVisit()
        XCTAssertEqual(e.state.loadedDays, 0)
        XCTAssertEqual(e.state.timeline.compactMap(\.exchange).count, 1)
        XCTAssertTrue(e.state.history[0].entries.contains { $0.answer == "吸附成 L 形" })
    }
}
