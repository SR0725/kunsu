import SwiftUI

/// 浮在對話上的玻璃回覆面板：追問、答案按鈕、輸入列（DESIGN「Phone app」）。
struct Dock: View {
    @Environment(AppModel.self) private var model
    @State private var draft = ""
    @FocusState private var typing: Bool

    var body: some View {
        VStack(spacing: 10) {
            if let item = model.engine.currentItem {
                HStack(spacing: 6) {
                    QuietChip("為什麼？") { model.followUp(.why) }
                    QuietChip("前因後果？") { model.followUp(.context) }
                    Spacer()
                    QuietChip("等下再說") { model.skip() }
                }
                AnswerArea(item: item)
            } else {
                ScrollView(.horizontal) {
                    HStack(spacing: 6) {
                        ForEach(["大家在做什麼？", "開新任務", "今天做了什麼？"], id: \.self) { s in
                            QuietChip(s) { if s == "開新任務" { model.showNewTask = true } else { model.chat(s) } }
                        }
                    }
                }
                .scrollIndicators(.hidden)
            }
            inputRow
        }
        .padding(10)
        .glass(RoundedRectangle(cornerRadius: 30))
        .padding(.horizontal, 10)
        .padding(.bottom, 6)
    }

    private var placeholder: String {
        switch model.engine.typingTarget {
        case .reply: "或打字回覆 \(model.engine.currentItem?.project ?? "")…"
        case .answerText: "打字回答這題…"
        case .followUp: "打字問軍師這題的事…"
        case .disabled: "用上面的按鈕回答"
        case .relayChat: "跟軍師說…"
        }
    }

    private var inputRow: some View {
        let disabled = model.engine.typingTarget == .disabled
        let empty = draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        return HStack(spacing: 8) {
            Button { model.showNewTask = true } label: {
                Image(systemName: "plus").font(.system(size: 18, weight: .medium)).foregroundStyle(K.strong)
                    .frame(width: 44, height: 44)
                    .background(Circle().fill(K.glassFill))
                    .overlay(Circle().stroke(K.glassLine))
            }
            .accessibilityLabel("開新任務")
            TextField(placeholder, text: $draft, axis: .vertical)
                .lineLimit(1...5)
                .focused($typing)
                .disabled(disabled)
                .font(.callout)
                .foregroundStyle(K.strong)
                .padding(.horizontal, 16).padding(.vertical, 12)
                .background(RoundedRectangle(cornerRadius: 22).fill(K.glassFill))
                .overlay(RoundedRectangle(cornerRadius: 22).stroke(K.glassLine))
                .opacity(disabled ? 0.45 : 1)
                .submitLabel(.send)
                .onSubmit(submit)
            Button(action: submit) {
                Image(systemName: "arrow.up").font(.system(size: 18, weight: .bold)).foregroundStyle(K.onPrimary)
                    .frame(width: 44, height: 44)
                    .background(Circle().fill(K.primary))
            }
            .disabled(disabled || empty)
            .opacity(disabled || empty ? 0.4 : 1)
            .accessibilityLabel("送出")
        }
    }

    private func submit() {
        let t = draft
        draft = ""
        typing = false
        model.send(t)
    }
}

private struct QuietChip: View {
    let title: String
    let action: () -> Void
    init(_ title: String, action: @escaping () -> Void) { self.title = title; self.action = action }
    var body: some View {
        Button(action: action) {
            Text(title).font(.footnote.weight(.medium)).foregroundStyle(K.body)
                .padding(.horizontal, 12).frame(height: 32)
                .background(Capsule().fill(K.glassFill))
                .overlay(Capsule().stroke(K.glassLine))
        }
        .buttonStyle(.plain)
        .frame(minHeight: 44)
    }
}

/// 答案按鈕：短的兩欄、長的一排一個；超過畫面 38% 就在面板裡捲動，底部提示「往下還有選項」。
private struct AnswerArea: View {
    @Environment(AppModel.self) private var model
    let item: Item
    @State private var contentHeight: CGFloat = 0

    var body: some View {
        let buttons = model.engine.answers(for: item)
        let layout = model.engine.layout(for: item)
        let maxH = UIScreen.main.bounds.height * 0.38
        let overflow = contentHeight > maxH + 1
        ScrollView {
            grid(buttons, layout)
                .padding(.top, 8)
                .background(GeometryReader { g in Color.clear.preference(key: HeightKey.self, value: g.size.height) })
        }
        .scrollDisabled(!overflow)
        .frame(height: min(max(contentHeight, 1), maxH))
        .onPreferenceChange(HeightKey.self) { contentHeight = $0 }
        .overlay(alignment: .bottom) {
            if overflow {
                Text("往下還有選項").font(.caption).foregroundStyle(K.muted)
                    .frame(maxWidth: .infinity, minHeight: 40, alignment: .bottom)
                    .padding(.bottom, 2)
                    .background(LinearGradient(colors: [.clear, K.panel.opacity(0.9)], startPoint: .top, endPoint: .bottom))
                    .allowsHitTesting(false)
            }
        }
    }

    @ViewBuilder private func grid(_ buttons: [AnswerButton], _ layout: AnswerLayout) -> some View {
        if layout == .twoColumns {
            LazyVGrid(columns: [GridItem(.flexible(), spacing: 8), GridItem(.flexible(), spacing: 8)], spacing: 8) {
                ForEach(buttons) { b in AnswerTile(button: b, centered: true) }
            }
        } else {
            VStack(spacing: 8) { ForEach(buttons) { b in AnswerTile(button: b, centered: false) } }
        }
    }
}

private struct HeightKey: PreferenceKey {
    static let defaultValue: CGFloat = 0
    static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) { value = max(value, nextValue()) }
}

private struct AnswerTile: View {
    @Environment(AppModel.self) private var model
    let button: AnswerButton
    let centered: Bool

    var body: some View {
        Button { model.tap(button) } label: {
            VStack(alignment: centered ? .center : .leading, spacing: 2) {
                Text(button.label).font(.system(size: 16, weight: .semibold)).foregroundStyle(K.strong)
                    .multilineTextAlignment(centered ? .center : .leading)
                    .fixedSize(horizontal: false, vertical: true)
                if let d = button.detail {
                    Text(d).font(.footnote).foregroundStyle(K.muted).multilineTextAlignment(.leading)
                }
            }
            .frame(maxWidth: .infinity, minHeight: centered ? 60 : 52, alignment: centered ? .center : .leading)
            .padding(.horizontal, 16).padding(.vertical, 10)
            .background(RoundedRectangle(cornerRadius: 20).fill(K.glassFill))
            .overlay(RoundedRectangle(cornerRadius: 20).stroke(button.recommended ? K.strong : K.glassEdge, lineWidth: button.recommended ? 1.5 : 1))
            .overlay(alignment: centered ? .top : .topTrailing) {
                if button.recommended {
                    Text("AI 建議").font(.system(size: 11, weight: .semibold)).foregroundStyle(K.onPrimary)
                        .padding(.horizontal, 8).padding(.vertical, 1)
                        .background(Capsule().fill(K.primary))
                        .offset(x: centered ? 0 : -14, y: -8)
                }
            }
            .shadow(color: .black.opacity(0.05), radius: 6, y: 3)
        }
        .buttonStyle(PressScale())
        .accessibilityLabel(button.recommended ? "\(button.label)，AI 建議" : button.label)
    }
}

private struct PressScale: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label.scaleEffect(configuration.isPressed ? 0.98 : 1)
    }
}
