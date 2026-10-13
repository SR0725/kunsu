import SwiftUI

@main
struct KunsuMobileApp: App {
    @State private var model = AppModel()

    var body: some Scene {
        WindowGroup {
            RootView()
                .environment(model)
                .preferredColorScheme(model.appearance.scheme)
                .tint(K.strong)
        }
    }
}

struct RootView: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        @Bindable var model = model
        Group {
            if model.needsLogin { LoginScreen() } else if model.needsPairing { PairingScreen() } else { ChatScreen() }
        }
            .sheet(isPresented: Binding(get: { model.moneySheet != nil }, set: { if !$0 { model.moneySheet = nil } })) {
                if let m = model.moneySheet { MoneySheet(decisionID: m.decisionID, selected: m.option) }
            }
            .sheet(isPresented: Binding(get: { model.receipt != nil }, set: { if !$0 { model.receipt = nil } })) {
                ReceiptSheet(label: model.receipt ?? "")
            }
            .sheet(isPresented: $model.showSettings) { SettingsSheet() }
            .sheet(isPresented: $model.showNewTask) { NewTaskSheet() }
    }
}
