import Foundation

/// 手機上 AI 寫的文字的 Markdown 安全子集（AC-MA-15；DESIGN「Markdown」）。
/// 只認排版：段落、標題、清單、引用、程式碼；行內粗體、斜體、刪除線、程式碼。
/// 連結只留文字、圖片只留替代文字、HTML 照原文——AI 的文字在手機上不能帶人去任何地方。

public struct MDSpan: Equatable, Sendable {
    public var text: String
    public var bold = false
    public var italic = false
    public var strike = false
    public var code = false
    public init(_ text: String, bold: Bool = false, italic: Bool = false, strike: Bool = false, code: Bool = false) {
        self.text = text; self.bold = bold; self.italic = italic; self.strike = strike; self.code = code
    }
    func sameStyle(_ o: MDSpan) -> Bool { bold == o.bold && italic == o.italic && strike == o.strike && code == o.code }
}

public struct MDListItem: Equatable, Sendable {
    public var marker: String
    public var depth: Int
    public var spans: [MDSpan]
    public init(marker: String, depth: Int, spans: [MDSpan]) { self.marker = marker; self.depth = depth; self.spans = spans }
}

public enum MDBlock: Equatable, Sendable {
    case paragraph([MDSpan])
    case heading(Int, [MDSpan])
    case list([MDListItem])
    case quote([MDSpan])
    case code(String)
}

public enum Markdown {
    // MARK: 區塊

    public static func blocks(_ source: String) -> [MDBlock] {
        let lines = source.replacingOccurrences(of: "\r\n", with: "\n").components(separatedBy: "\n")
        var out: [MDBlock] = []
        var para: [String] = []
        var quote: [String] = []
        var items: [MDListItem] = []
        var i = 0

        func flushPara() { if !para.isEmpty { out.append(.paragraph(inline(para.joined(separator: "\n")))); para = [] } }
        func flushQuote() { if !quote.isEmpty { out.append(.quote(inline(quote.joined(separator: "\n")))); quote = [] } }
        func flushList() { if !items.isEmpty { out.append(.list(items)); items = [] } }
        func flushAll() { flushPara(); flushQuote(); flushList() }

        while i < lines.count {
            let raw = lines[i]
            let line = raw.trimmingCharacters(in: .whitespaces)

            if line.isEmpty { flushAll(); i += 1; continue }

            // 程式碼區塊
            if line.hasPrefix("```") || line.hasPrefix("~~~") {
                flushAll()
                let fence = String(line.prefix(3))
                var body: [String] = []
                i += 1
                while i < lines.count, !lines[i].trimmingCharacters(in: .whitespaces).hasPrefix(fence) { body.append(lines[i]); i += 1 }
                out.append(.code(body.joined(separator: "\n")))
                i += 1
                continue
            }

            // 表格：等寬原文
            if line.hasPrefix("|") {
                flushAll()
                var rows: [String] = []
                while i < lines.count, lines[i].trimmingCharacters(in: .whitespaces).hasPrefix("|") {
                    rows.append(lines[i].trimmingCharacters(in: .whitespaces)); i += 1
                }
                out.append(.code(rows.joined(separator: "\n")))
                continue
            }

            // 標題
            if let h = heading(line) { flushAll(); out.append(.heading(h.0, inline(h.1))); i += 1; continue }

            // 引用
            if line.hasPrefix(">") {
                flushPara(); flushList()
                quote.append(String(line.dropFirst()).trimmingCharacters(in: .whitespaces))
                i += 1
                continue
            }

            // 清單
            if let item = listItem(raw) {
                flushPara(); flushQuote()
                items.append(item)
                i += 1
                continue
            }

            flushQuote(); flushList()
            para.append(line)
            i += 1
        }
        flushAll()
        return out
    }

    private static func heading(_ line: String) -> (Int, String)? {
        let hashes = line.prefix { $0 == "#" }.count
        guard (1...6).contains(hashes) else { return nil }
        let rest = line.dropFirst(hashes)
        guard rest.first == " " else { return nil }
        return (hashes, rest.trimmingCharacters(in: .whitespaces).trimmingCharacters(in: CharacterSet(charactersIn: "#")).trimmingCharacters(in: .whitespaces))
    }

    private static func listItem(_ raw: String) -> MDListItem? {
        let indent = raw.prefix { $0 == " " || $0 == "\t" }.reduce(0) { $0 + ($1 == "\t" ? 4 : 1) }
        let line = raw.trimmingCharacters(in: .whitespaces)
        let depth = min(indent / 2, 3)
        for m in ["- ", "* ", "+ "] where line.hasPrefix(m) {
            return MDListItem(marker: "•", depth: depth, spans: inline(String(line.dropFirst(2))))
        }
        let digits = line.prefix { $0.isASCII && $0.isNumber }
        guard !digits.isEmpty, digits.count <= 3 else { return nil }
        let after = line.dropFirst(digits.count)
        guard let p = after.first, p == "." || p == ")", after.dropFirst().first == " " else { return nil }
        return MDListItem(marker: "\(digits).", depth: depth, spans: inline(String(after.dropFirst(2))))
    }

    // MARK: 行內

    public static func inline(_ s: String) -> [MDSpan] {
        merge(parse(Array(s), base: MDSpan("")))
    }

    private static func parse(_ c: [Character], base: MDSpan) -> [MDSpan] {
        var out: [MDSpan] = []
        var buf = ""
        func emit(_ text: String, _ style: MDSpan) { if !text.isEmpty { var sp = style; sp.text = text; out.append(sp) } }
        func flush() { emit(buf, base); buf = "" }
        var i = 0
        while i < c.count {
            let ch = c[i]
            // 跳脫
            if ch == "\\", i + 1 < c.count, "\\`*_~[]()!#>-+.|".contains(c[i + 1]) { buf.append(c[i + 1]); i += 2; continue }
            // 行內程式碼
            if ch == "`", let end = find(c, "`", from: i + 1) {
                flush(); var st = base; st.code = true; emit(String(c[(i + 1)..<end]), st); i = end + 1; continue
            }
            // 圖片：只留替代文字；連結：只留文字
            if ch == "!", i + 1 < c.count, c[i + 1] == "[", let (text, next) = link(c, i + 1) { buf += text; i = next; continue }
            if ch == "[", let (text, next) = link(c, i) {
                flush(); out += parse(Array(text), base: base); i = next; continue
            }
            // 粗體／刪除線
            var handled = false
            for (marker, key) in [("**", "bold"), ("__", "bold"), ("~~", "strike")] {
                let m = Array(marker)
                guard i + 1 < c.count, c[i] == m[0], c[i + 1] == m[1] else { continue }
                if marker == "__", isWord(c, i - 1) { break }
                guard let end = findPair(c, m, from: i + 2), end > i + 2 else { break }
                flush()
                var st = base
                if key == "bold" { st.bold = true } else { st.strike = true }
                out += parse(Array(c[(i + 2)..<end]), base: st)
                i = end + 2
                handled = true
                break
            }
            if handled { continue }
            // 斜體
            if (ch == "*" || ch == "_"), i + 1 < c.count, c[i + 1] != " ", c[i + 1] != ch,
               !(ch == "_" && isWord(c, i - 1)),
               let end = findSingle(c, ch, from: i + 1), c[end - 1] != " ",
               !(ch == "_" && isWord(c, end + 1)) {
                flush(); var st = base; st.italic = true
                out += parse(Array(c[(i + 1)..<end]), base: st)
                i = end + 1
                continue
            }
            buf.append(ch)
            i += 1
        }
        flush()
        return out
    }

    private static func isWord(_ c: [Character], _ i: Int) -> Bool {
        guard i >= 0, i < c.count else { return false }
        return c[i].isLetter || c[i].isNumber
    }

    private static func find(_ c: [Character], _ ch: Character, from: Int) -> Int? {
        var j = from
        while j < c.count { if c[j] == ch { return j }; j += 1 }
        return nil
    }

    private static func findPair(_ c: [Character], _ m: [Character], from: Int) -> Int? {
        var j = from
        while j + 1 < c.count { if c[j] == m[0], c[j + 1] == m[1] { return j }; j += 1 }
        return nil
    }

    /// 單個 * 或 _ 的結尾（不是成對的 ** 的一半）。
    private static func findSingle(_ c: [Character], _ ch: Character, from: Int) -> Int? {
        var j = from
        while j < c.count {
            if c[j] == ch {
                if j + 1 < c.count, c[j + 1] == ch { j += 2; continue }
                return j
            }
            j += 1
        }
        return nil
    }

    /// `[文字](網址)`：回文字與下一個位置。
    private static func link(_ c: [Character], _ open: Int) -> (String, Int)? {
        guard let close = find(c, "]", from: open + 1), close + 1 < c.count, c[close + 1] == "(",
              let paren = find(c, ")", from: close + 2) else { return nil }
        return (String(c[(open + 1)..<close]), paren + 1)
    }

    private static func merge(_ spans: [MDSpan]) -> [MDSpan] {
        var out: [MDSpan] = []
        for s in spans where !s.text.isEmpty {
            if let last = out.last, last.sameStyle(s) { out[out.count - 1].text += s.text } else { out.append(s) }
        }
        return out
    }
}
