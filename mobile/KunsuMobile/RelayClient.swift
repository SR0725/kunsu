import Foundation

/// 連中轉站的 WebSocket：只送收 JSON 文字訊框；斷線退避重連。內容的加解密在 LiveLink。
/// 連線狀態交給 KunsuCore 的 Reconnect（AC-MA-17）：每條連線有自己的編號，被取代的舊連線送來的失敗與心跳一律不算。
@MainActor
final class RelayClient {
    private var task: URLSessionWebSocketTask?
    private var url: URL?
    private var state = Reconnect()
    var onText: ((String) -> Void)?
    var onState: ((Bool) -> Void)?
    private(set) var connected = false

    func connect(_ url: URL) {
        self.url = url
        open()
    }

    func disconnect() {
        state.close()
        task?.cancel(with: .goingAway, reason: nil)
        task = nil
        setConnected(false)
    }

    func send(_ text: String) {
        task?.send(.string(text)) { _ in }
    }

    private func setConnected(_ c: Bool) {
        guard connected != c else { return }
        connected = c
        onState?(c)
    }

    private func apply(_ action: Reconnect.Action) {
        switch action {
        case .ignore:
            break
        case .online:
            setConnected(true)
        case .offline(let wait):
            setConnected(false)
            let id = currentID
            Task { @MainActor [weak self] in
                try? await Task.sleep(for: .seconds(wait))
                // 等待期間被別的連線取代或刻意關掉就不重連
                guard let self, self.currentID == id, self.url != nil, !self.closedOnPurpose else { return }
                self.open()
            }
        }
    }

    private var currentID = 0
    private var closedOnPurpose: Bool { task == nil }

    private func open() {
        guard let url else { return }
        let old = task
        let id = state.open()
        currentID = id
        let t = URLSession.shared.webSocketTask(with: url)
        task = t
        old?.cancel(with: .goingAway, reason: nil) // 舊連線取消後的失敗會被 Reconnect 忽略
        t.resume()
        receive(t, id)
        ping(t, id)
    }

    private func ping(_ t: URLSessionWebSocketTask, _ id: Int) {
        t.sendPing { [weak self] error in
            Task { @MainActor in
                guard let self else { return }
                if error == nil {
                    self.apply(self.state.alive(id))
                    try? await Task.sleep(for: .seconds(20))
                    if self.currentID == id { self.ping(t, id) }
                } else {
                    self.apply(self.state.failed(id))
                }
            }
        }
    }

    private func receive(_ t: URLSessionWebSocketTask, _ id: Int) {
        t.receive { [weak self] result in
            Task { @MainActor in
                guard let self else { return }
                switch result {
                case .success(.string(let s)):
                    if self.currentID == id { self.onText?(s) }
                    self.receive(t, id)
                case .success(.data(let d)):
                    if self.currentID == id, let s = String(data: d, encoding: .utf8) { self.onText?(s) }
                    self.receive(t, id)
                case .success:
                    self.receive(t, id)
                case .failure:
                    self.apply(self.state.failed(id))
                }
            }
        }
    }
}
