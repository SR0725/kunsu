import SwiftUI

/// 動錢：核對後送出（所見即所簽），沒有 Face ID（2026-10-12）。點哪個選項進來，就是使用者明確選的那個。
struct MoneySheet: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    let decisionID: String
    @State var selected: Int

    var body: some View {
        if let d = model.engine.state.decisions.first(where: { $0.id == decisionID }) {
            VStack(spacing: 14) {
                Text("核對後送出").font(.title2.weight(.semibold)).foregroundStyle(K.strong).padding(.top, 20)
                Text("\(d.project) · \(d.agent.name)").font(.subheadline).foregroundStyle(K.muted)
                Text(d.question).font(.headline).foregroundStyle(K.strong).multilineTextAlignment(.center)
                if let m = d.money { MoneyBlock(params: m) }
                VStack(spacing: 0) {
                    ForEach(Array(d.options.enumerated()), id: \.offset) { i, o in
                        Button { selected = i } label: {
                            HStack {
                                Text(o.label).foregroundStyle(selected == i ? K.accent : K.strong)
                                Spacer()
                                if selected == i { Image(systemName: "checkmark").foregroundStyle(K.accent) }
                            }
                            .padding(.horizontal, 16).frame(minHeight: 48)
                            .background(selected == i ? K.accentFill : .clear)
                        }
                        .buttonStyle(.plain)
                        .accessibilityAddTraits(selected == i ? .isSelected : [])
                        if i < d.options.count - 1 { Divider().overlay(K.border) }
                    }
                }
                .background(RoundedRectangle(cornerRadius: 16).fill(K.panel))
                .clipShape(RoundedRectangle(cornerRadius: 16))
                .overlay(RoundedRectangle(cornerRadius: 16).stroke(K.border))
                Spacer(minLength: 0)
                Button(d.options[selected].recommended ? "批准並執行" : "送出決定") {
                    model.answerDecision(decisionID, option: selected, confirmed: true)
                    if !d.options[selected].recommended { dismiss() }
                }
                .buttonStyle(PrimaryPill())
                Button("先不要") { dismiss() }.foregroundStyle(K.muted).frame(minHeight: 44)
            }
            .padding(.horizontal, 20)
            .padding(.bottom, 12)
            .presentationDetents([.large])
            .presentationBackground(K.window)
        }
    }
}

struct ReceiptSheet: View {
    @Environment(\.dismiss) private var dismiss
    let label: String
    var body: some View {
        VStack(spacing: 16) {
            Image(systemName: "checkmark.seal.fill").font(.system(size: 40)).foregroundStyle(K.ok).padding(.top, 28)
            Text("已批准").font(.title2.weight(.semibold)).foregroundStyle(K.strong)
            Text(label).foregroundStyle(K.body)
            VStack(alignment: .leading, spacing: 10) {
                Label("Kunsu 已收到正式答案", systemImage: "checkmark")
                Label("已簽章，交給 executor 執行", systemImage: "signature")
                Label("已交給等待中的原對話", systemImage: "arrow.turn.down.right")
            }
            .font(.subheadline).foregroundStyle(K.body)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(16)
            .background(RoundedRectangle(cornerRadius: 16).fill(K.panel))
            Spacer()
            Button("完成") { dismiss() }.buttonStyle(PrimaryPill())
        }
        .padding(20)
        .presentationDetents([.medium])
        .presentationBackground(K.window)
    }
}

struct SettingsSheet: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        @Bindable var model = model
        NavigationStack {
            Form {
                Section {
                    Picker("中轉 AI", selection: Binding(get: { model.engine.state.relay }, set: { model.setRelay($0) })) {
                        ForEach(Agent.allCases, id: \.self) { a in Text(a.name).tag(a) }
                    }
                    .pickerStyle(.segmented)
                } header: { Text("中轉 AI") } footer: {
                    Text("軍師用這個 AI 回答你的追問、陪你聊天和開任務。跟電腦版「設定」是同一個值。它不能替你回答決定題或批准動錢。")
                }
                Section("外觀") {
                    Picker("外觀", selection: $model.appearance) {
                        ForEach(Appearance.allCases) { a in Text(a.label).tag(a) }
                    }
                    .pickerStyle(.segmented)
                }
                AccountSection()
                Section {
                    if let p = model.pairing {
                        LabeledContent("Mac", value: model.macOnline ? "已連線" : "連不到中轉站")
                        LabeledContent("Mac 識別", value: p.mac.id).font(.system(.footnote, design: .monospaced))
                        LabeledContent("這支手機", value: model.deviceID).font(.system(.footnote, design: .monospaced))
                        Button("解除配對", role: .destructive) { model.unpair(); dismiss() }
                    } else {
                        LabeledContent("目前", value: "示範資料")
                        Button("配對這台 Mac") { model.needsPairing = true; dismiss() }
                    }
                    #if DEBUG
                    Button("模擬重新打開 App") { model.reopen(); dismiss() }
                    #endif
                } header: { Text("連線") } footer: {
                    Text(model.pairing == nil ? "示範資料不會送到任何地方。" : "內容經中轉站端對端加密，中轉站看不到。要換手機時在電腦的「設定 › 手機」撤銷舊的。")
                }
            }
            .navigationTitle("設定")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .confirmationAction) { Button("完成") { dismiss() } } }
        }
    }
}

struct NewTaskSheet: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    @State private var agent: Agent = .claude
    @State private var project = "kunsu"
    @State private var text = ""

    var body: some View {
        NavigationStack {
            Form {
                Section("給誰") {
                    Picker("AI", selection: $agent) { ForEach(Agent.allCases, id: \.self) { Text($0.name).tag($0) } }
                        .pickerStyle(.segmented)
                }
                Section("專案") {
                    ForEach(model.engine.state.projects, id: \.self) { p in
                        Button { project = p } label: {
                            HStack {
                                Text(p).foregroundStyle(K.strong)
                                Spacer()
                                if project == p { Image(systemName: "checkmark").foregroundStyle(K.accent) }
                            }
                        }
                    }
                }
                Section { TextField("要做什麼？", text: $text, axis: .vertical).lineLimit(3...8) } header: { Text("要做什麼") } footer: {
                    Text("權限：Auto（一般動作自己做，高風險才回來問你）")
                }
            }
            .navigationTitle("開新任務")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("取消") { dismiss() } }
                ToolbarItem(placement: .confirmationAction) {
                    Button("開始") { model.startTask(agent: agent, project: project, text: text); dismiss() }
                        .disabled(text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                }
            }
        }
        .onAppear {
            agent = model.engine.state.relay
            if let first = model.engine.state.projects.first, !model.engine.state.projects.contains(project) { project = first }
        }
    }
}

/// 設定裡的帳號：email、登出、刪除帳號（App Store 5.1.1(v)；AC-MU-04）。
private struct AccountSection: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    @State private var confirmDelete = false
    @State private var deleting = false

    var body: some View {
        let account = model.account
        Section {
            if let email = account.email, account.loggedIn {
                LabeledContent("Email", value: email)
                if !account.macs.isEmpty {
                    LabeledContent("你的 Mac", value: account.macs.joined(separator: "、"))
                }
                Button("登出") { account.logout(); dismiss() }
                Button(deleting ? "刪除中…" : "刪除帳號", role: .destructive) { confirmDelete = true }
                    .disabled(deleting)
                if let e = account.deleteError { Text(e).font(.caption).foregroundStyle(K.bad) }
            } else {
                Button("登入") { model.startLogin(); dismiss() }
            }
        } header: { Text("帳號") } footer: {
            Text("雲端只存你的 email 和裝置清單；題目、答案與對話只在你的 Mac。刪除帳號會清掉雲端的帳號與裝置清單，Mac 上的資料不受影響。")
        }
        .task { await account.refresh() }
        .confirmationDialog("刪除帳號？", isPresented: $confirmDelete, titleVisibility: .visible) {
            Button("刪除帳號", role: .destructive) {
                deleting = true
                Task {
                    if await account.deleteAccount() { model.unpair(); dismiss() }
                    deleting = false
                }
            }
            Button("取消", role: .cancel) {}
        } message: {
            Text("雲端的帳號與裝置清單會刪除，這支手機也會解除配對。這個動作不能復原。")
        }
    }
}
