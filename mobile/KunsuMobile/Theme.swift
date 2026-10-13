import SwiftUI
import UIKit

// DESIGN.md「Unified design language」：主視窗的 token，淺色／深色跟著系統或設定。

extension UIColor {
    convenience init(hex: UInt32, alpha: CGFloat = 1) {
        self.init(red: CGFloat((hex >> 16) & 0xFF) / 255, green: CGFloat((hex >> 8) & 0xFF) / 255,
                  blue: CGFloat(hex & 0xFF) / 255, alpha: alpha)
    }
}

extension Color {
    /// 品牌色（來源標記）：兩種主題都一樣。
    init(hex: UInt32) { self.init(uiColor: UIColor(hex: hex)) }
}

private func adaptive(_ light: UInt32, _ dark: UInt32, lightAlpha: CGFloat = 1, darkAlpha: CGFloat = 1) -> Color {
    Color(UIColor { $0.userInterfaceStyle == .dark ? UIColor(hex: dark, alpha: darkAlpha) : UIColor(hex: light, alpha: lightAlpha) })
}

enum K {
    static let window = adaptive(0xF5F5F5, 0x0A0A0A)
    static let panel = adaptive(0xFFFFFF, 0x111111)
    static let subtle = adaptive(0xFAFAFA, 0x161616)
    static let hover = adaptive(0xF0F0F0, 0x1C1C1C)
    static let strong = adaptive(0x171717, 0xFAFAFA)
    static let body = adaptive(0x404040, 0xD4D4D4)
    static let muted = adaptive(0x6B6B6B, 0xA3A3A3)
    static let faint = adaptive(0xA3A3A3, 0x737373)
    static let border = adaptive(0xE5E5E5, 0x262626)
    static let border2 = adaptive(0xD4D4D4, 0x333333)
    static let primary = adaptive(0x171717, 0xFAFAFA)
    static let onPrimary = adaptive(0xFFFFFF, 0x0A0A0A)
    static let accent = adaptive(0x2563EB, 0x60A5FA)
    static let accentFill = adaptive(0xEFF4FF, 0x60A5FA, darkAlpha: 0.12)
    static let warn = adaptive(0xB54708, 0xFCD34D)
    static let ok = adaptive(0x067647, 0x6EE7A0)
    static let bad = adaptive(0xB42318, 0xFCA5A5)
    static let glassEdge = adaptive(0xFFFFFF, 0xFFFFFF, lightAlpha: 0.85, darkAlpha: 0.14)
    static let glassLine = adaptive(0x000000, 0xFFFFFF, lightAlpha: 0.07, darkAlpha: 0.06)
    static let glassFill = adaptive(0xFFFFFF, 0x262628, lightAlpha: 0.72, darkAlpha: 0.72)

    static let gutter: CGFloat = 16
}

/// 液態玻璃：只用在浮在內容上面、現在要你處理的東西（DESIGN「Glass means floating above content」）。
/// iOS 26 用系統的 Liquid Glass；更早的系統用材質＋亮邊＋陰影近似。
struct Glass<S: InsettableShape>: ViewModifier {
    let shape: S
    var strong = false

    func body(content: Content) -> some View {
        if #available(iOS 26.0, *) {
            content.glassEffect(.regular, in: shape)
        } else {
            content
                .background(shape.fill(strong ? AnyShapeStyle(K.glassFill) : AnyShapeStyle(.ultraThinMaterial)))
                .overlay(shape.strokeBorder(K.glassEdge, lineWidth: 1))
                .shadow(color: .black.opacity(0.10), radius: 18, y: 8)
        }
    }
}

extension View {
    func glass<S: InsettableShape>(_ shape: S, strong: Bool = false) -> some View { modifier(Glass(shape: shape, strong: strong)) }
}

struct PrimaryPill: ButtonStyle {
    var enabled = true
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.system(.headline))
            .foregroundStyle(K.onPrimary)
            .frame(maxWidth: .infinity, minHeight: 50)
            .background(Capsule().fill(K.primary.opacity(enabled ? 1 : 0.35)))
            .scaleEffect(configuration.isPressed ? 0.98 : 1)
    }
}

/// 只有 M L H V C Z 絕對指令的 SVG path（Claude 星芒正好只用這些）。
struct SVGPath: Shape {
    let d: String
    let viewBox: CGFloat

    func path(in rect: CGRect) -> Path {
        var p = Path()
        let s = min(rect.width, rect.height) / viewBox
        var tokens: [String] = []
        var cur = ""
        for ch in d {
            if ch.isLetter {
                if !cur.isEmpty { tokens.append(cur); cur = "" }
                tokens.append(String(ch))
            } else if ch == " " || ch == "," {
                if !cur.isEmpty { tokens.append(cur); cur = "" }
            } else if ch == "-" && !cur.isEmpty && cur.last != "e" {
                tokens.append(cur); cur = "-"
            } else {
                cur.append(ch)
            }
        }
        if !cur.isEmpty { tokens.append(cur) }
        var i = 0
        var cmd = "M"
        var pt = CGPoint.zero
        func num() -> CGFloat { defer { i += 1 }; return CGFloat(Double(tokens[i]) ?? 0) * s }
        while i < tokens.count {
            if let c = tokens[i].first, c.isLetter { cmd = String(c); i += 1; if cmd == "Z" { p.closeSubpath(); continue } }
            switch cmd {
            case "M": pt = CGPoint(x: num(), y: num()); p.move(to: pt); cmd = "L"
            case "L": pt = CGPoint(x: num(), y: num()); p.addLine(to: pt)
            case "H": pt.x = num(); p.addLine(to: pt)
            case "V": pt.y = num(); p.addLine(to: pt)
            case "C":
                let c1 = CGPoint(x: num(), y: num()); let c2 = CGPoint(x: num(), y: num())
                pt = CGPoint(x: num(), y: num()); p.addCurve(to: pt, control1: c1, control2: c2)
            default: i += 1
            }
        }
        return p.offsetBy(dx: rect.minX, dy: rect.minY)
    }
}
