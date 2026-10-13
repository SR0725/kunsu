import XCTest
@testable import KunsuCore

/// AC-MA-15：手機上 AI 寫的文字照安全子集渲染 Markdown。
final class MarkdownTests: XCTestCase {
    private func plain(_ t: String) -> MDSpan { MDSpan(t) }

    func test_inline_styles() {
        XCTAssertEqual(Markdown.inline("先講**結論**，再說*原因*，~~舊的~~，跑 `cargo test`"), [
            plain("先講"), MDSpan("結論", bold: true), plain("，再說"), MDSpan("原因", italic: true), plain("，"),
            MDSpan("舊的", strike: true), plain("，跑 "), MDSpan("cargo test", code: true),
        ])
    }

    func test_bold_next_to_cjk_punctuation() {
        XCTAssertEqual(Markdown.inline("**要你決定：**用一句話"), [MDSpan("要你決定：", bold: true), plain("用一句話")])
    }

    func test_links_images_and_html_are_text_only() {
        XCTAssertEqual(Markdown.inline("看[這份報告](https://evil.example)與![圖表](x.png)<script>x</script>"),
                       [plain("看這份報告與圖表<script>x</script>")])
    }

    func test_snake_case_and_lone_markers_stay_literal() {
        XCTAssertEqual(Markdown.inline("改 relay_agent_setting 與 2 * 3 = 6"), [plain("改 relay_agent_setting 與 2 * 3 = 6")])
        XCTAssertEqual(Markdown.inline("沒關的 **粗體"), [plain("沒關的 **粗體")])
        XCTAssertEqual(Markdown.inline(#"跳脫 \*不是斜體\*"#), [plain("跳脫 *不是斜體*")])
        XCTAssertEqual(Markdown.inline("**甲****乙**"), [MDSpan("甲", bold: true), MDSpan("乙", bold: true)].reduce(into: [MDSpan]()) { r, s in
            if let l = r.last, l.bold == s.bold { r[r.count - 1].text += s.text } else { r.append(s) }
        })
    }

    func test_blocks() {
        let md = """
        ## 結論
        這題會**動錢**。
        第二行接著。

        - 加碼：每天多 100
        - 維持：
          - 什麼都不變
        1. 先看報表
        2) 再決定

        > 引用一段

        ```
        let x = 1
        ```
        """
        XCTAssertEqual(Markdown.blocks(md), [
            .heading(2, [plain("結論")]),
            .paragraph([plain("這題會"), MDSpan("動錢", bold: true), plain("。\n第二行接著。")]),
            .list([
                MDListItem(marker: "•", depth: 0, spans: [plain("加碼：每天多 100")]),
                MDListItem(marker: "•", depth: 0, spans: [plain("維持：")]),
                MDListItem(marker: "•", depth: 1, spans: [plain("什麼都不變")]),
                MDListItem(marker: "1.", depth: 0, spans: [plain("先看報表")]),
                MDListItem(marker: "2.", depth: 0, spans: [plain("再決定")]),
            ]),
            .quote([plain("引用一段")]),
            .code("let x = 1"),
        ])
    }

    func test_tables_render_as_monospaced_text_and_plain_text_is_one_paragraph() {
        XCTAssertEqual(Markdown.blocks("| a | b |\n|---|---|\n| 1 | 2 |"), [.code("| a | b |\n|---|---|\n| 1 | 2 |")])
        XCTAssertEqual(Markdown.blocks("沒有任何格式"), [.paragraph([plain("沒有任何格式")])])
        XCTAssertEqual(Markdown.blocks("  \n\n"), [])
    }
}
