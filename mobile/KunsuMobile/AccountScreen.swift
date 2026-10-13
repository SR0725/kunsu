import SwiftUI
import Observation

/// 手機的帳號（PRD §31.2；BDD multi-user AC-MU-04）。session 跟手機私鑰一樣放鑰匙圈，不行就放受保護的檔案。
@MainActor
@Observable
final class AccountModel {
    private(set) var email: String?
    private(set) var session: String?
    var flow: LoginFlow = .idle()
    private(set) var macs: [String] = []
    /// 帳號的顯示名稱（問候用，AC-MA-18）。
    private(set) var displayName: String? = UserDefaults.standard.string(forKey: "account.name")
    var deleteError: String?
    private let vault = SecretChain([KeychainStore(), ProtectedFileStore()])
    private var task: Task<Void, Never>?
    var baseProvider: () -> String = { AccountAPI.defaultBase }

    init() {
        session = vault.read("session").flatMap { String(data: $0, encoding: .utf8) }.flatMap { $0.isEmpty ? nil : $0 }
        email = UserDefaults.standard.string(forKey: "account.email")
    }

    var loggedIn: Bool { session != nil }

    func submit(_ raw: String) {
        flow = flow.submit(raw, now: Date())
        guard case .sending(let email, _) = flow else { return }
        task?.cancel()
        task = Task { [weak self] in
            guard let self else { return }
            do {
                let (data, res) = try await URLSession.shared.data(for: AccountAPI.startLogin(base: self.baseProvider(), email: email))
                let poll = (try? JSONSerialization.jsonObject(with: data) as? [String: Any])?["poll"] as? String
                guard (res as? HTTPURLResponse)?.statusCode == 202, let poll else { self.flow = self.flow.failed("寄不出登入信，請稍後再試"); return }
                self.flow = self.flow.sent(poll: poll)
                await self.waitForClick(email: email, poll: poll)
            } catch {
                self.flow = self.flow.failed("連不上帳號服務")
            }
        }
    }

    private func waitForClick(email: String, poll: String) async {
        while !Task.isCancelled, case .waiting = flow {
            try? await Task.sleep(for: .seconds(2))
            guard let (data, res) = try? await URLSession.shared.data(for: AccountAPI.poll(base: baseProvider(), poll: poll)) else { continue }
            flow = flow.polled(AccountAPI.pollResult(status: (res as? HTTPURLResponse)?.statusCode ?? 0, body: data), now: Date())
        }
        if case .done(let s) = flow {
            vault.write("session", Data(s.utf8))
            UserDefaults.standard.set(email, forKey: "account.email")
            self.session = s
            self.email = email
            flow = .idle()
            await refresh()
        }
    }

    func cancel() {
        task?.cancel()
        flow = .idle()
    }

    /// 讀帳號裡的 Mac 清單（只是顯示；配對仍要掃 QR 並在 Mac 上按允許）。
    func refresh() async {
        guard let session, let (data, res) = try? await URLSession.shared.data(for: AccountAPI.account(base: baseProvider(), session: session)) else { return }
        if (res as? HTTPURLResponse)?.statusCode == 401 { clearLocal(); return }
        let json = (try? JSONSerialization.jsonObject(with: data) as? [String: Any]) ?? [:]
        self.macs = ((json["macs"] as? [[String: Any]]) ?? []).compactMap { $0["name"] as? String }
        if let name = json["display_name"] as? String {
            displayName = name
            UserDefaults.standard.set(name, forKey: "account.name")
        }
    }

    func logout() { clearLocal() }

    /// 刪除帳號（App Store 5.1.1(v)）：雲端的帳號與裝置清單刪除；這支手機上的配對也一併清掉。
    func deleteAccount() async -> Bool {
        guard let session else { return false }
        deleteError = nil
        guard let (_, res) = try? await URLSession.shared.data(for: AccountAPI.deleteAccount(base: baseProvider(), session: session)),
              let code = (res as? HTTPURLResponse)?.statusCode, code == 204 || code == 401 else {
            deleteError = "刪除失敗，請確認網路後再試一次"
            return false
        }
        clearLocal()
        return true
    }

    private func clearLocal() {
        vault.write("session", Data())
        UserDefaults.standard.removeObject(forKey: "account.email")
        UserDefaults.standard.removeObject(forKey: "account.name")
        displayName = nil
        session = nil
        email = nil
        macs = []
    }
}

/// 第一次打開：先登入（AC-MU-04）。審查與試用可以先看示範資料。
struct LoginScreen: View {
    @Environment(AppModel.self) private var model
    @State private var email = ""
    @FocusState private var focused: Bool

    var body: some View {
        let account = model.account
        ZStack {
            K.window.ignoresSafeArea()
            ScrollView {
                VStack(alignment: .leading, spacing: 18) {
                    Image(uiImage: UIImage(named: "kmark") ?? UIImage()).resizable().frame(width: 52, height: 52)
                        .clipShape(RoundedRectangle(cornerRadius: 14))
                        .padding(.top, 24)
                    Text("登入 Kunsu").font(.system(size: 30, weight: .semibold)).foregroundStyle(K.strong)
                    Text("AI 卡住要你決定時，在手機上就能回答。題目、答案與對話只留在你的 Mac；雲端只有你的 email 和裝置清單。")
                        .font(.callout).foregroundStyle(K.body)

                    switch account.flow {
                    case .waiting(let e, _, _):
                        VStack(alignment: .leading, spacing: 10) {
                            HStack(spacing: 12) {
                                ProgressView()
                                Text("已寄登入連結到 \(e)").font(.subheadline.weight(.semibold)).foregroundStyle(K.strong)
                            }
                            Text("在這支手機或電腦上點信裡的連結，這裡就會自動登入（15 分鐘內有效）。").font(.footnote).foregroundStyle(K.muted)
                            Button("改用別的 email") { account.cancel() }.font(.footnote).frame(minHeight: 44)
                        }
                        .padding(16)
                        .background(RoundedRectangle(cornerRadius: 16).fill(K.panel))
                    default:
                        VStack(alignment: .leading, spacing: 8) {
                            TextField(text: $email, prompt: Text(verbatim: "you@example.com").foregroundStyle(K.faint)) { Text("Email") }
                                .keyboardType(.emailAddress).textContentType(.emailAddress)
                                .textInputAutocapitalization(.never).autocorrectionDisabled()
                                .focused($focused)
                                .padding(14)
                                .background(RoundedRectangle(cornerRadius: 14).fill(K.panel))
                                .overlay(RoundedRectangle(cornerRadius: 14).stroke(errorText == nil ? K.border : K.bad))
                                .submitLabel(.send)
                                .onSubmit { account.submit(email) }
                            if let e = errorText { Text(e).font(.caption).foregroundStyle(K.bad) }
                            Button {
                                account.submit(email)
                            } label: {
                                Text(isSending ? "寄送中…" : "用 email 登入").frame(maxWidth: .infinity)
                            }
                            .buttonStyle(PrimaryPill())
                            .disabled(email.trimmingCharacters(in: .whitespaces).isEmpty || isSending)
                        }
                    }

                    Button("先用示範資料看看") { model.useDemo() }
                        .font(.footnote).foregroundStyle(K.muted).frame(minHeight: 44)
                }
                .padding(.horizontal, 20)
            }
        }
    }

    private var errorText: String? {
        if case .idle(let e) = model.account.flow { return e }
        return nil
    }

    private var isSending: Bool {
        if case .sending = model.account.flow { return true }
        return false
    }
}
