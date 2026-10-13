import SwiftUI

/// 軍師一問一答（DESIGN「Unified design language」Phone app）。
struct ChatScreen: View {
    @Environment(AppModel.self) private var model
    @State private var tracker = PullTracker()
    @State private var viewport: CGFloat = 0
    @State private var openFolds: Set<String> = []
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    private var engine: Engine { model.engine }

    var body: some View {
        ZStack(alignment: .top) {
            K.window.ignoresSafeArea()
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    history
                    // AC-MA-16：內容不滿一屏時，問候與這次的內容整塊垂直置中
                    Spacer(minLength: 0)
                    hero
                    timeline
                    if engine.current == nil { PendingRow() }
                    Spacer(minLength: 0)
                }
                .padding(.horizontal, K.gutter)
                .padding(.top, 64)
                .padding(.bottom, 12)
                .frame(maxWidth: .infinity, minHeight: viewport, alignment: .leading)
            }
            .defaultScrollAnchor(.bottom)
            .scrollDismissesKeyboard(.interactively)
            // AC-MA-02：只算超出頂端的距離；只有手指拉滿再放開才載入
            .onScrollGeometryChange(for: [Double].self) { g in
                [PullTracker.overscroll(offsetY: g.contentOffset.y, topInset: g.contentInsets.top),
                 g.containerSize.height] // 容器高度已經是狀態列與輸入區之間的可見範圍
            } action: { _, v in
                viewport = max(0, v[1])
                tracker.more = engine.hasMoreHistory
                if tracker.update(v[0]) { UIImpactFeedbackGenerator(style: .light).impactOccurred() }
            }
            .onScrollPhaseChange { old, new in
                if new == .interacting {
                    tracker.begin()
                } else if old == .interacting, tracker.end(more: engine.hasMoreHistory) {
                    withAnimation(reduceMotion ? nil : .easeOut(duration: 0.24)) { _ = model.releasePull(Pull.threshold) }
                }
            }
            .safeAreaInset(edge: .bottom, spacing: 0) { Dock() }

            // 狀態列與標頭後面：內容淡出，不跟時間、電量疊在一起。
            LinearGradient(colors: [K.window, K.window, K.window.opacity(0)], startPoint: .top, endPoint: .bottom)
                .frame(height: 40)
                .ignoresSafeArea(edges: .top)
                .allowsHitTesting(false)
            PullIndicator(distance: tracker.distance, more: engine.hasMoreHistory)
                .padding(.top, 70)
                .allowsHitTesting(false)
            HeaderPill()
                .padding(.horizontal, 12)
                .padding(.top, 4)
            if let n = model.notice {
                Text(n).font(.footnote).foregroundStyle(K.bad)
                    .padding(12).frame(maxWidth: .infinity, alignment: .leading)
                    .glass(RoundedRectangle(cornerRadius: 16))
                    .padding(.horizontal, 12).padding(.top, 62)
                    .onTapGesture { model.notice = nil }
            }
            if let p = model.pendingSend {
                UndoToast(send: p)
                    .padding(.horizontal, 12)
                    .padding(.top, 62)
                    .transition(.move(edge: .top).combined(with: .opacity))
            }
        }
    }

    // MARK: 歷史紀錄（拉滿才出現）

    @ViewBuilder private var history: some View {
        let days = engine.visibleHistory
        if !days.isEmpty {
            Group {
                ForEach(days.reversed()) { day in
                    DaySeparator(text: day.name)
                    ForEach(day.entries) { h in HistoryRow(entry: h) }
                }
                DaySeparator(text: "這次打開")
            }
            .transition(.opacity.combined(with: .move(edge: .top)))
        }
    }

    // MARK: 問候

    private var hero: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(Greeting.text(hour: Calendar.current.component(.hour, from: Date()), name: model.account.displayName)).font(.system(size: 30, weight: .semibold)).foregroundStyle(K.strong)
            Text(subtitle).font(.callout).foregroundStyle(K.muted)
        }
        .padding(.horizontal, 4)
        .padding(.top, 8)
    }

    private var subtitle: String {
        let n = engine.items.count
        if n > 0 { return "有 \(n) 件事在等你" }
        let running = engine.state.sessions.values.filter { if case .running = $0.status { true } else { false } }
        if !running.isEmpty { return "\(running.map(\.name).joined(separator: "、")) 還在跑，好了再叫你" }
        return "沒有事在等你，可以睡了"
    }

    // MARK: 這次的對話

    @ViewBuilder private var timeline: some View {
        ForEach(engine.state.timeline) { entry in
            switch entry {
            case .exchange(let e):
                if e.skipped {
                    Text("排到最後了").font(.footnote).foregroundStyle(K.muted).frame(maxWidth: .infinity)
                } else {
                    let live = e.id == engine.current?.id
                    QuestionCard(exchange: e, live: live, folded: !openFolds.contains(e.id)) {
                        if openFolds.contains(e.id) { openFolds.remove(e.id) } else { openFolds.insert(e.id) }
                    }
                    if live { PendingRow() }
                    if let a = e.answer { AnswerBubble(exchange: e, text: a) }
                }
            case .chat(_, let c):
                ChatLine(entry: c)
            }
        }
    }
}

// MARK: - 下拉

struct PullIndicator: View {
    let distance: Double
    let more: Bool
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        let phase = Pull.phase(distance: distance, more: more)
        let progress: Double = {
            switch phase {
            case .pulling(let p): p
            case .ready: 1
            default: 0
            }
        }()
        let ready = phase == .ready
        VStack(spacing: 6) {
            ZStack {
                // 從空開始，筆畫隨距離填滿；滿了變實心並出現箭頭
                Circle().stroke(K.border2, lineWidth: 2.5)
                Circle().trim(from: 0, to: progress).stroke(K.strong, style: StrokeStyle(lineWidth: 2.5, lineCap: .round))
                    .rotationEffect(.degrees(-90))
                Circle().fill(K.strong).opacity(ready ? 1 : 0)
                Image(systemName: "arrow.down").font(.system(size: 12, weight: .bold)).foregroundStyle(K.onPrimary)
                    .opacity(ready ? 1 : 0)
            }
            .frame(width: 26, height: 26)
            .scaleEffect(ready ? 1.08 : 0.7 + 0.3 * progress)
            Text(text(phase)).font(.caption.weight(ready ? .semibold : .regular)).foregroundStyle(ready ? K.strong : K.muted)
        }
        .opacity(min(1, max(0, (distance - 8) / 32)))
        .animation(reduceMotion ? nil : .easeOut(duration: 0.12), value: progress)
        .animation(reduceMotion ? nil : .spring(duration: 0.22, bounce: 0.3), value: ready)
        .accessibilityElement(children: .combine)
    }

    private func text(_ p: Pull.Phase) -> String { Pull.hint(p) }
}

// MARK: - 元件

private struct DaySeparator: View {
    let text: String
    var body: some View {
        HStack(spacing: 10) {
            Rectangle().fill(K.border).frame(height: 1)
            Text(text).font(.caption).foregroundStyle(K.muted).fixedSize()
            Rectangle().fill(K.border).frame(height: 1)
        }
        .padding(.vertical, 2)
    }
}

private struct HistoryRow: View {
    let entry: HistoryEntry
    var body: some View {
        VStack(alignment: .trailing, spacing: 10) {
            HStack(alignment: .top, spacing: 10) {
                AgentAvatar(agent: entry.agent)
                Text(entry.question).font(.system(size: 16, weight: .medium)).foregroundStyle(K.strong)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(16)
                    .background(RoundedRectangle(cornerRadius: 22).fill(K.panel))
                    .overlay(RoundedRectangle(cornerRadius: 22).stroke(K.border))
            }
            .opacity(0.62)
            MineBubble(text: entry.answer, status: "✓✓ \(entry.result)", tone: K.muted)
        }
    }
}

struct QuestionCard: View {
    @Environment(AppModel.self) private var model
    let exchange: Exchange
    let live: Bool
    let folded: Bool
    let toggleFold: () -> Void

    var body: some View {
        let item = live ? model.engine.item(id: exchange.itemID) : nil
        let agent = item?.agent ?? exchange.snapshot?.agent ?? .claude
        HStack(alignment: .top, spacing: 10) {
            AgentAvatar(agent: agent)
            VStack(alignment: .leading, spacing: 10) {
                if let item { content(item) } else {
                    Text(exchange.snapshot?.question ?? "").font(.system(size: 16, weight: .medium)).foregroundStyle(K.strong)
                }
                followUps
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(16)
            .background(RoundedRectangle(cornerRadius: 22).fill(K.panel))
            .overlay(RoundedRectangle(cornerRadius: 22).stroke(K.border))
            .shadow(color: .black.opacity(live ? 0.06 : 0), radius: 12, y: 6)
        }
        .opacity(live ? 1 : 0.62)
    }

    @ViewBuilder private func content(_ item: Item) -> some View {
        switch item.kind {
        case .decision(let d):
            Text(d.question).font(.system(size: 19, weight: .semibold)).foregroundStyle(K.strong).fixedSize(horizontal: false, vertical: true)
            MarkdownText(source: d.why)
            if let money = d.money { MoneyBlock(params: money) }
        case .permission(let s):
            Text("要執行這個指令嗎？").font(.system(size: 19, weight: .semibold)).foregroundStyle(K.strong)
            Text(s.permission?.command ?? "").font(.system(.callout, design: .monospaced)).foregroundStyle(K.strong)
                .padding(10).frame(maxWidth: .infinity, alignment: .leading)
                .background(RoundedRectangle(cornerRadius: 10).fill(K.hover))
            MarkdownText(source: s.permission?.reason ?? "")
        case .reply(let s):
            MarkdownText(source: OptionParser.body(s.lastAI), font: .system(size: 17, weight: .medium), color: K.strong)
        }
    }

    @ViewBuilder private var followUps: some View {
        let list = exchange.followUps
        if !list.isEmpty {
            if live || !folded {
                ForEach(list) { f in FollowUpView(followUp: f, canForward: live && model.engine.canForward(f.id) && !list.contains { $0.by == .agent }) }
            }
            if !live {
                Button(action: toggleFold) {
                    Text("追問 \(list.count) 次 \(folded ? "▾" : "▴")").font(.footnote).foregroundStyle(K.muted)
                }
                .buttonStyle(.plain)
            }
        }
    }
}

struct MoneyBlock: View {
    let params: [MoneyParam]
    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("所見即所簽").font(.caption.weight(.semibold)).foregroundStyle(K.strong)
            ForEach(params, id: \.label) { p in
                HStack(alignment: .firstTextBaseline) {
                    Text(p.label).foregroundStyle(K.muted)
                    Spacer()
                    Text(p.value).foregroundStyle(K.strong).monospacedDigit().multilineTextAlignment(.trailing)
                }
                .font(.subheadline)
            }
        }
        .padding(12)
        .background(RoundedRectangle(cornerRadius: 12).fill(K.subtle))
        .overlay(RoundedRectangle(cornerRadius: 12).stroke(K.border2))
    }
}

private struct FollowUpView: View {
    @Environment(AppModel.self) private var model
    let followUp: FollowUp
    let canForward: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Divider().overlay(K.border)
            Text("你問：\(followUp.question)").font(.caption).foregroundStyle(K.muted)
            HStack(spacing: 6) {
                if followUp.by == .relay { RelayAvatar(relay: model.engine.state.relay, size: 16) }
                Text(followUp.signature).font(.caption.weight(.semibold)).foregroundStyle(K.body)
            }
            if followUp.status == .thinking {
                Text(followUp.by == .agent ? "已轉問 \(followUp.signature.replacingOccurrences(of: " 本人", with: ""))，它回答前這題會留著…" : "正在想…")
                    .font(.subheadline).foregroundStyle(K.muted)
            } else {
                if let a = followUp.answer {
                    if followUp.status == .failed {
                        Text("沒問到：\(a)").font(.subheadline).foregroundStyle(K.muted).fixedSize(horizontal: false, vertical: true)
                    } else {
                        MarkdownText(source: a)
                    }
                }
                if let q = followUp.quote {
                    VStack(alignment: .leading, spacing: 3) { ForEach(q, id: \.self) { Text($0) } }
                        .font(.footnote).foregroundStyle(K.body)
                        .padding(.leading, 10)
                        .overlay(alignment: .leading) { Rectangle().fill(K.border2).frame(width: 2) }
                }
                if canForward, let project = model.engine.currentItem?.project {
                    Button("問 \(project) 本人") { model.forward(followUp.id) }
                        .font(.footnote.weight(.semibold)).foregroundStyle(K.accent)
                        .frame(minHeight: 32)
                }
            }
        }
    }
}

struct MineBubble: View {
    let text: String
    let status: String
    let tone: Color
    var body: some View {
        VStack(alignment: .trailing, spacing: 4) {
            Text(text).font(.callout).foregroundStyle(K.onPrimary)
                .padding(.horizontal, 15).padding(.vertical, 10)
                .background(UnevenRoundedRectangle(topLeadingRadius: 20, bottomLeadingRadius: 20, bottomTrailingRadius: 6, topTrailingRadius: 20).fill(K.primary))
            if !status.isEmpty { Text(status).font(.caption).foregroundStyle(tone).multilineTextAlignment(.trailing) }
        }
        .frame(maxWidth: .infinity, alignment: .trailing)
        .padding(.leading, 48)
    }
}

private struct AnswerBubble: View {
    @Environment(AppModel.self) private var model
    let exchange: Exchange
    let text: String

    var body: some View {
        let (status, tone) = statusLine
        MineBubble(text: text, status: status, tone: tone)
    }

    private var statusLine: (String, Color) {
        let e = model.engine
        if model.pendingSend != nil && e.isUndoable(exchange) { return ("5 秒內可撤回", K.warn) }
        guard let h = e.historyEntry(exchange.historyID) else { return ("", K.muted) }
        if h.result == Engine.pendingResult || h.result == "執行中…" { return ("✓ 已轉給 \(h.project) · 處理中…", K.accent) }
        return ("✓✓ \(h.result)", K.muted)
    }
}

/// 現在這件以外還在等你的事：頭像疊在一起＋一行字，點開可以挑一件先處理。
struct PendingRow: View {
    @Environment(AppModel.self) private var model
    @State private var open = false

    var body: some View {
        let rest = model.engine.waiting
        if !rest.isEmpty {
            VStack(alignment: .leading, spacing: 8) {
                Button { withAnimation(.snappy) { open.toggle() } } label: {
                    HStack(spacing: 10) {
                        HStack(spacing: -9) {
                            ForEach(Array(rest.prefix(5).enumerated()), id: \.offset) { i, it in
                                SourceMark(agent: it.agent, size: 16)
                                    .frame(width: 28, height: 28)
                                    .background(Circle().fill(K.panel))
                                    .overlay(Circle().stroke(K.window, lineWidth: 2))
                                    .zIndex(Double(10 - i))
                            }
                        }
                        Text("尚有 \(rest.count) 個待辦事項要處理").font(.subheadline).foregroundStyle(K.body)
                        Text(open ? "▴" : "▾").foregroundStyle(K.muted)
                    }
                    .frame(minHeight: 40)
                }
                .buttonStyle(.plain)
                .accessibilityHint("點一下看清單，挑一件先處理")
                if open {
                    VStack(spacing: 0) {
                        ForEach(rest) { it in
                            Button { open = false; model.jump(it.id) } label: {
                                HStack(spacing: 10) {
                                    SourceMark(agent: it.agent, size: 16)
                                    (Text(it.project).foregroundStyle(K.strong).fontWeight(.semibold) + Text("　\(it.question.split(separator: "\n").first.map(String.init) ?? "")").foregroundStyle(K.body))
                                        .lineLimit(1)
                                    Spacer(minLength: 4)
                                    Text("先處理").font(.caption).foregroundStyle(K.accent)
                                }
                                .font(.footnote)
                                .padding(.horizontal, 14).frame(minHeight: 44)
                            }
                            .buttonStyle(.plain)
                            if it.id != rest.last?.id { Divider().overlay(K.border) }
                        }
                    }
                    .background(RoundedRectangle(cornerRadius: 16).fill(K.panel))
                    .overlay(RoundedRectangle(cornerRadius: 16).stroke(K.border))
                }
            }
        }
    }
}

private struct ChatLine: View {
    @Environment(AppModel.self) private var model
    let entry: ChatEntry

    var body: some View {
        switch entry {
        case .me(let t):
            MineBubble(text: t, status: "", tone: K.muted)
        case .relay(let t):
            relay(MarkdownText(source: t, font: .callout, color: K.strong))
        case .relayThinking:
            relay(Text("正在想…").font(.callout).foregroundStyle(K.muted))
        case .task(let sid):
            if let s = model.engine.state.sessions[sid] { TaskCard(session: s) }
        }
    }

    private func relay(_ content: some View) -> some View {
        HStack(alignment: .top, spacing: 10) {
            RelayAvatar(relay: model.engine.state.relay)
            content.frame(maxWidth: .infinity, alignment: .leading).padding(.top, 4)
        }
    }
}

private struct TaskCard: View {
    let session: Session
    var body: some View {
        HStack(spacing: 10) {
            SourceMark(agent: session.agent, size: 20)
            VStack(alignment: .leading, spacing: 2) {
                Text("新任務 · \(session.name)").font(.subheadline.weight(.semibold)).foregroundStyle(K.strong)
                Text(status).font(.caption).foregroundStyle(K.muted)
                if session.status == .idle, case .ai(let last)? = session.turns.last {
                    MarkdownText(source: last)
                }
            }
            Spacer()
        }
        .padding(14)
        .background(RoundedRectangle(cornerRadius: 16).fill(K.panel))
        .overlay(RoundedRectangle(cornerRadius: 16).stroke(K.border))
        .padding(.leading, 40)
    }

    private var status: String {
        switch session.status {
        case .running(let w): "執行中 · \(w)"
        case .reply: "等你回話"
        case .permission: "等你批准"
        case .idle: "做完了"
        }
    }
}

// MARK: - 標頭與撤回

private struct HeaderPill: View {
    @Environment(AppModel.self) private var model
    var body: some View {
        Button { model.showSettings = true } label: {
            HStack(spacing: 10) {
                Image(uiImage: UIImage(named: "kmark") ?? UIImage()).resizable().frame(width: 26, height: 26)
                    .clipShape(RoundedRectangle(cornerRadius: 7))
                Text(model.engine.headerTitle).font(.subheadline.weight(.semibold)).foregroundStyle(K.strong)
                Spacer()
                HStack(spacing: 6) {
                    Circle().fill(model.macOnline ? K.strong : K.warn).frame(width: 6, height: 6)
                    Text(model.macOnline ? "Mac 已連線" : "Mac 離線").font(.caption).foregroundStyle(K.muted)
                }
            }
            .padding(.horizontal, 12)
            .frame(height: 48)
            .glass(Capsule())
        }
        .buttonStyle(.plain)
        .accessibilityLabel("\(model.engine.headerTitle)，設定")
    }
}

private struct UndoToast: View {
    @Environment(AppModel.self) private var model
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    let send: PendingSend
    @State private var progress: CGFloat = 1

    var body: some View {
        HStack(spacing: 10) {
            VStack(alignment: .leading, spacing: 2) {
                Text("已送出\(send.label)").font(.subheadline.weight(.semibold)).foregroundStyle(K.strong).lineLimit(1)
                Text("5 秒內可撤回，撤回前不會送到 Mac").font(.caption).foregroundStyle(K.muted)
            }
            Spacer(minLength: 8)
            Button("撤回") { model.undo() }
                .font(.subheadline.weight(.semibold)).foregroundStyle(K.strong)
                .padding(.horizontal, 14).frame(minHeight: 40)
                .background(Capsule().fill(K.hover))
        }
        .padding(.leading, 16).padding(.trailing, 10).padding(.vertical, 10)
        .overlay(alignment: .bottomLeading) {
            GeometryReader { g in
                Capsule().fill(K.strong.opacity(0.35)).frame(width: max(0, g.size.width - 32) * progress, height: 2)
                    .offset(x: 16, y: g.size.height - 6)
            }
        }
        .glass(RoundedRectangle(cornerRadius: 20))
        .id(send.id)
        .onAppear { if !reduceMotion { withAnimation(.linear(duration: 5)) { progress = 0 } } }
    }
}
