import SwiftUI
import AVFoundation

/// 第一次打開：掃電腦上的 QR code 配對（AC-MP-09）。模擬器沒有相機時可以貼上配對連結。
struct PairingScreen: View {
    @Environment(AppModel.self) private var model
    @State private var pasted = ""
    @State private var invalid = false
    @State private var cameraDenied = false

    var body: some View {
        ZStack {
            K.window.ignoresSafeArea()
            ScrollView {
                VStack(alignment: .leading, spacing: 18) {
                    Image(uiImage: UIImage(named: "kmark") ?? UIImage()).resizable().frame(width: 52, height: 52)
                        .clipShape(RoundedRectangle(cornerRadius: 14))
                        .padding(.top, 24)
                    Text("配對這台 Mac").font(.system(size: 30, weight: .semibold)).foregroundStyle(K.strong)
                    Text("在電腦上打開軍師主視窗的「設定 › 手機」，按「配對新手機」，用這裡掃出現的 QR code。掃完之後在電腦上按「允許」就完成了。")
                        .font(.callout).foregroundStyle(K.body)

                    phaseView

                    if model.pairingPhase != .waiting {
                        scanner
                        VStack(alignment: .leading, spacing: 8) {
                            Text("或貼上配對連結").font(.footnote.weight(.semibold)).foregroundStyle(K.muted)
                            HStack(spacing: 8) {
                                TextField("kunsu-pair:v1?…", text: $pasted)
                                    .textInputAutocapitalization(.never).autocorrectionDisabled()
                                    .font(.system(.footnote, design: .monospaced))
                                    .padding(12)
                                    .background(RoundedRectangle(cornerRadius: 14).fill(K.panel))
                                    .overlay(RoundedRectangle(cornerRadius: 14).stroke(invalid ? K.bad : K.border))
                                Button("配對") { invalid = !model.pair(with: pasted) }
                                    .buttonStyle(.borderedProminent).tint(K.primary)
                                    .disabled(pasted.isEmpty)
                            }
                            if invalid { Text("這不是軍師的配對連結").font(.caption).foregroundStyle(K.bad) }
                        }
                    }

                    Button("先用示範資料看看") { model.useDemo() }
                        .font(.footnote).foregroundStyle(K.muted).frame(minHeight: 44)
                }
                .padding(.horizontal, 20)
            }
        }
    }

    @ViewBuilder private var phaseView: some View {
        switch model.pairingPhase {
        case .waiting:
            HStack(spacing: 12) {
                ProgressView()
                VStack(alignment: .leading, spacing: 2) {
                    Text("已送出，等你在電腦上按「允許」").font(.subheadline.weight(.semibold)).foregroundStyle(K.strong)
                    Text("電腦的軍師主視窗會跳出這支手機要配對。").font(.caption).foregroundStyle(K.muted)
                }
            }
            .padding(16).frame(maxWidth: .infinity, alignment: .leading)
            .background(RoundedRectangle(cornerRadius: 16).fill(K.panel))
        case .rejected(let reason):
            Label("沒有配對成功：\(reason)。請在電腦上重新產生 QR code 再掃一次。", systemImage: "xmark.circle")
                .font(.subheadline).foregroundStyle(K.bad)
        case .revoked:
            Label("這支手機已在電腦上被撤銷，請重新配對。", systemImage: "exclamationmark.triangle")
                .font(.subheadline).foregroundStyle(K.warn)
        case .idle:
            EmptyView()
        }
    }

    @ViewBuilder private var scanner: some View {
        if QRScanner.cameraAvailable && !cameraDenied {
            QRScanner { code in
                if model.pairingPhase != .waiting { invalid = !model.pair(with: code) }
            } onDenied: { cameraDenied = true }
            .frame(height: 300)
            .clipShape(RoundedRectangle(cornerRadius: 22))
            .overlay(RoundedRectangle(cornerRadius: 22).stroke(K.border))
            .overlay(alignment: .bottom) {
                Text("把 QR code 放進框裡").font(.caption.weight(.semibold)).foregroundStyle(.white)
                    .padding(.horizontal, 12).padding(.vertical, 6).background(Capsule().fill(.black.opacity(0.5))).padding(12)
            }
        } else {
            Label(cameraDenied ? "相機權限被關掉了：到「設定 › 軍師」打開相機，或貼上配對連結。" : "這台裝置沒有相機，請貼上配對連結。",
                  systemImage: "camera")
                .font(.footnote).foregroundStyle(K.muted)
        }
    }
}

/// AVFoundation 的 QR 掃描。
struct QRScanner: UIViewRepresentable {
    static var cameraAvailable: Bool { AVCaptureDevice.default(for: .video) != nil }
    let onCode: (String) -> Void
    let onDenied: () -> Void

    func makeCoordinator() -> Coordinator { Coordinator(onCode: onCode) }

    func makeUIView(context: Context) -> PreviewView {
        let view = PreviewView()
        let session = AVCaptureSession()
        view.previewLayer.session = session
        view.previewLayer.videoGravity = .resizeAspectFill
        context.coordinator.session = session
        AVCaptureDevice.requestAccess(for: .video) { granted in
            guard granted else { DispatchQueue.main.async { onDenied() }; return }
            guard let device = AVCaptureDevice.default(for: .video), let input = try? AVCaptureDeviceInput(device: device),
                  session.canAddInput(input) else { return }
            session.addInput(input)
            let output = AVCaptureMetadataOutput()
            guard session.canAddOutput(output) else { return }
            session.addOutput(output)
            output.setMetadataObjectsDelegate(context.coordinator, queue: .main)
            output.metadataObjectTypes = [.qr]
            DispatchQueue.global(qos: .userInitiated).async { session.startRunning() }
        }
        return view
    }

    func updateUIView(_ uiView: PreviewView, context: Context) {}

    static func dismantleUIView(_ uiView: PreviewView, coordinator: Coordinator) {
        coordinator.session?.stopRunning()
    }

    final class Coordinator: NSObject, AVCaptureMetadataOutputObjectsDelegate {
        let onCode: (String) -> Void
        var session: AVCaptureSession?
        private var last: String?
        init(onCode: @escaping (String) -> Void) { self.onCode = onCode }
        func metadataOutput(_ output: AVCaptureMetadataOutput, didOutput objects: [AVMetadataObject], from connection: AVCaptureConnection) {
            guard let s = (objects.first as? AVMetadataMachineReadableCodeObject)?.stringValue, s != last else { return }
            last = s
            UINotificationFeedbackGenerator().notificationOccurred(.success)
            onCode(s)
        }
    }

    final class PreviewView: UIView {
        override class var layerClass: AnyClass { AVCaptureVideoPreviewLayer.self }
        var previewLayer: AVCaptureVideoPreviewLayer { layer as! AVCaptureVideoPreviewLayer }
    }
}
