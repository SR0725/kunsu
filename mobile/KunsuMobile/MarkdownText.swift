import SwiftUI

/// AI 寫的文字（AC-MA-15；DESIGN「Markdown」）：原生畫出 Markdown 安全子集，不用網頁。
struct MarkdownText: View {
    let source: String
    var font: Font = .subheadline
    var color: Color = K.body

    var body: some View {
        let blocks = Markdown.blocks(source)
        VStack(alignment: .leading, spacing: 8) {
            ForEach(Array(blocks.enumerated()), id: \.offset) { _, b in block(b) }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    @ViewBuilder private func block(_ b: MDBlock) -> some View {
        switch b {
        case .paragraph(let spans):
            styled(spans).font(font).foregroundStyle(color)
        case .heading(_, let spans):
            // 標題不比題目大
            styled(spans).font(font.weight(.semibold)).foregroundStyle(K.strong)
        case .list(let items):
            VStack(alignment: .leading, spacing: 4) {
                ForEach(Array(items.enumerated()), id: \.offset) { _, it in
                    HStack(alignment: .firstTextBaseline, spacing: 6) {
                        Text(it.marker).font(font).foregroundStyle(K.muted).monospacedDigit()
                        styled(it.spans).font(font).foregroundStyle(color)
                    }
                    .padding(.leading, CGFloat(it.depth) * 16)
                }
            }
        case .quote(let spans):
            styled(spans).font(font).foregroundStyle(K.muted)
                .padding(.leading, 10)
                .overlay(alignment: .leading) { Rectangle().fill(K.border2).frame(width: 2) }
        case .code(let text):
            ScrollView(.horizontal) {
                Text(text).font(.system(.footnote, design: .monospaced)).foregroundStyle(K.strong)
                    .fixedSize(horizontal: true, vertical: false)
                    .padding(10)
            }
            .scrollIndicators(.hidden)
            .background(RoundedRectangle(cornerRadius: 10).fill(K.hover))
        }
    }

    private func styled(_ spans: [MDSpan]) -> Text {
        var out = AttributedString()
        for s in spans {
            var a = AttributedString(s.text)
            var intent: InlinePresentationIntent = []
            if s.bold { intent.insert(.stronglyEmphasized) }
            if s.italic { intent.insert(.emphasized) }
            if s.code { intent.insert(.code) }
            if !intent.isEmpty { a.inlinePresentationIntent = intent }
            if s.strike { a.strikethroughStyle = .single }
            if s.code { a.backgroundColor = K.hover }
            out += a
        }
        return Text(out)
    }
}
