import Foundation
import CryptoKit

// 手機端的配對與中轉協定（docs/design/mobile-relay.md）：跟 Rust 的 kunsu-mobile 同一套位元格式，
// 互通由 docs/design/mobile-relay-vector.json 驗證（AC-MP-04）。

public enum B64 {
    public static func encode(_ d: Data) -> String {
        d.base64EncodedString().replacingOccurrences(of: "+", with: "-").replacingOccurrences(of: "/", with: "_")
            .replacingOccurrences(of: "=", with: "")
    }
    public static func decode(_ s: String) -> Data? {
        var t = s.replacingOccurrences(of: "-", with: "+").replacingOccurrences(of: "_", with: "/")
        while t.count % 4 != 0 { t += "=" }
        return Data(base64Encoded: t)
    }
}

/// sha256 → base32 小寫前 16 字：`mac_id`、`device_id`。
public enum ShortID {
    private static let alphabet = Array("abcdefghijklmnopqrstuvwxyz234567")
    public static func of(_ key: Data) -> String {
        let digest = Array(SHA256.hash(data: key))
        var out = ""
        var buffer = 0, bits = 0
        for byte in digest {
            buffer = (buffer << 8) | Int(byte)
            bits += 8
            while bits >= 5 {
                out.append(alphabet[(buffer >> (bits - 5)) & 31])
                bits -= 5
                if out.count == 16 { return out }
            }
        }
        return out
    }
}

public struct MacPublicKeys: Equatable, Codable, Sendable {
    public var id: String
    public var kx: Data
    public var sig: Data
    public init(id: String, kx: Data, sig: Data) { self.id = id; self.kx = kx; self.sig = sig }
}

/// 裝置簽章：正式在 Secure Enclave（私鑰不可匯出）；測試與模擬器用軟體金鑰。
public protocol P256Signer: Sendable {
    var publicKeyX963: Data { get }
    func sign(_ data: Data) throws -> Data
}

public struct SoftwareSigner: P256Signer {
    public let key: P256.Signing.PrivateKey
    public init(_ key: P256.Signing.PrivateKey) { self.key = key }
    public var publicKeyX963: Data { key.publicKey.x963Representation }
    public func sign(_ data: Data) throws -> Data { try key.signature(for: data).rawRepresentation }
}

public struct PairRequestWire: Codable, Equatable, Sendable {
    public var t: String
    public var eph: String
    public var nonce: String
    public var ct: String
}

public struct PairAccepted: Codable, Equatable, Sendable {
    public var t: String
    public var mac: String
    public var dev: String
    public var ts: Int64
    public var sig: String
}

public struct Envelope: Codable, Equatable, Sendable {
    public var v: Int
    public var dev: String
    public var seq: UInt64
    public var ts: Int64
    public var nonce: String
    public var ct: String
    public var sig: String

    public static func signingInput(_ e: Envelope) -> String { "1|\(e.dev)|\(e.seq)|\(e.ts)|\(e.nonce)|\(e.ct)" }
    static func aad(dev: String, seq: UInt64, ts: Int64) -> Data { Data("1|\(dev)|\(seq)|\(ts)".utf8) }
}

public enum RelayCryptoError: Error, Equatable {
    case badKey, badSignature, cannotOpen, notForMe
}

private func hkdf(_ shared: SharedSecret, salt: Data, info: Data) -> SymmetricKey {
    shared.hkdfDerivedSymmetricKey(using: SHA256.self, salt: salt, sharedInfo: info, outputByteCount: 32)
}

private func chachaSeal(_ key: SymmetricKey, nonce: Data, aad: Data, _ msg: Data) throws -> Data {
    let box = try ChaChaPoly.seal(msg, using: key, nonce: ChaChaPoly.Nonce(data: nonce), authenticating: aad)
    return box.ciphertext + box.tag
}

private func chachaOpen(_ key: SymmetricKey, nonce: Data, aad: Data, _ ct: Data) throws -> Data {
    guard ct.count >= 16 else { throw RelayCryptoError.cannotOpen }
    let box = try ChaChaPoly.SealedBox(nonce: ChaChaPoly.Nonce(data: nonce), ciphertext: ct.dropLast(16), tag: ct.suffix(16))
    return try ChaChaPoly.open(box, using: key, authenticating: aad)
}

public func randomBytes(_ n: Int) -> Data {
    var d = Data(count: n)
    _ = d.withUnsafeMutableBytes { SecRandomCopyBytes(kSecRandomDefault, n, $0.baseAddress!) }
    return d
}

/// 這支手機的身分：X25519（交換）＋ P-256（簽章）。
public struct DeviceIdentity: Sendable {
    public let kx: Curve25519.KeyAgreement.PrivateKey
    public let signer: any P256Signer
    public var deviceID: String { ShortID.of(signer.publicKeyX963) }

    public init(kx: Curve25519.KeyAgreement.PrivateKey, signer: any P256Signer) { self.kx = kx; self.signer = signer }

    private struct PairBody: Codable { let code: String; let device_name: String; let kx: String; let sig: String; let ts: Int64 }

    /// 配對請求：臨時 X25519 對 Mac 公鑰做 ECIES。
    public func pairRequest(mac: MacPublicKeys, code: String, deviceName: String, ts: Int64,
                            ephemeral: Curve25519.KeyAgreement.PrivateKey = .init(), nonce: Data = randomBytes(12)) throws -> PairRequestWire {
        let macKX = try Curve25519.KeyAgreement.PublicKey(rawRepresentation: mac.kx)
        let shared = try ephemeral.sharedSecretFromKeyAgreement(with: macKX)
        let key = hkdf(shared, salt: Data("kunsu-pair-v1".utf8), info: Data(mac.id.utf8))
        let enc = JSONEncoder()
        enc.outputFormatting = [.sortedKeys, .withoutEscapingSlashes]
        let body = try enc.encode(PairBody(code: code, device_name: deviceName, kx: B64.encode(kx.publicKey.rawRepresentation),
                                           sig: B64.encode(signer.publicKeyX963), ts: ts))
        let ct = try chachaSeal(key, nonce: nonce, aad: Data("pair_request|\(mac.id)".utf8), body)
        return PairRequestWire(t: "pair_request", eph: B64.encode(ephemeral.publicKey.rawRepresentation), nonce: B64.encode(nonce), ct: B64.encode(ct))
    }

    public func verify(_ a: PairAccepted, mac: MacPublicKeys) -> Bool {
        guard a.dev == deviceID, a.mac == mac.id, let sig = B64.decode(a.sig),
              let key = try? Curve25519.Signing.PublicKey(rawRepresentation: mac.sig) else { return false }
        return key.isValidSignature(sig, for: Data("pair_accepted|\(a.mac)|\(a.dev)|\(a.ts)".utf8))
    }

    private func sessionKey(_ mac: MacPublicKeys, info: String) throws -> SymmetricKey {
        let shared = try kx.sharedSecretFromKeyAgreement(with: Curve25519.KeyAgreement.PublicKey(rawRepresentation: mac.kx))
        return hkdf(shared, salt: Data(deviceID.utf8), info: Data(info.utf8))
    }

    /// 手機 → Mac 封包：加密並以裝置私鑰簽章。
    public func seal(mac: MacPublicKeys, seq: UInt64, ts: Int64, plaintext: Data, nonce: Data = randomBytes(12)) throws -> Envelope {
        let key = try sessionKey(mac, info: "kunsu-relay-v1|d2m")
        let ct = try chachaSeal(key, nonce: nonce, aad: Envelope.aad(dev: deviceID, seq: seq, ts: ts), plaintext)
        var env = Envelope(v: 1, dev: deviceID, seq: seq, ts: ts, nonce: B64.encode(nonce), ct: B64.encode(ct), sig: "")
        env.sig = B64.encode(try signer.sign(Data(Envelope.signingInput(env).utf8)))
        return env
    }

    /// Mac → 手機封包：驗 Mac 的 Ed25519 簽章後解開。
    public func open(fromMac mac: MacPublicKeys, _ env: Envelope) throws -> Data {
        guard env.dev == deviceID else { throw RelayCryptoError.notForMe }
        guard let sig = B64.decode(env.sig), let key = try? Curve25519.Signing.PublicKey(rawRepresentation: mac.sig),
              key.isValidSignature(sig, for: Data(Envelope.signingInput(env).utf8)) else { throw RelayCryptoError.badSignature }
        guard let nonce = B64.decode(env.nonce), let ct = B64.decode(env.ct) else { throw RelayCryptoError.cannotOpen }
        do {
            return try chachaOpen(try sessionKey(mac, info: "kunsu-relay-v1|m2d"), nonce: nonce, aad: Envelope.aad(dev: env.dev, seq: env.seq, ts: env.ts), ct)
        } catch { throw RelayCryptoError.cannotOpen }
    }
}

/// 主視窗 QR 的內容：`kunsu-pair:v1?relay=…&mac=…&code=…&kx=…&sig=…`。mac_id 跟公鑰對不上就拒收。
public struct PairingLink: Equatable, Sendable {
    public let relay: String
    public let code: String
    public let mac: MacPublicKeys

    public init?(_ s: String) {
        let t = s.trimmingCharacters(in: .whitespacesAndNewlines)
        guard t.hasPrefix("kunsu-pair:v1?") else { return nil }
        var q: [String: String] = [:]
        for kv in t.dropFirst("kunsu-pair:v1?".count).split(separator: "&") {
            let parts = kv.split(separator: "=", maxSplits: 1).map(String.init)
            if parts.count == 2 { q[parts[0]] = parts[1].removingPercentEncoding ?? parts[1] }
        }
        guard let relay = q["relay"], relay.hasPrefix("wss://") || relay.hasPrefix("ws://"),
              let mac = q["mac"], let code = q["code"], let kx = q["kx"].flatMap(B64.decode), let sig = q["sig"].flatMap(B64.decode),
              kx.count == 32, sig.count == 32, ShortID.of(sig) == mac else { return nil }
        self.relay = relay
        self.code = code
        self.mac = MacPublicKeys(id: mac, kx: kx, sig: sig)
    }
}

/// 存手機私鑰的地方（鑰匙圈、檔案）。寫入失敗要回 false，不能悄悄吞掉。
public protocol SecretStore: AnyObject {
    func read(_ account: String) -> Data?
    func write(_ account: String, _ data: Data) -> Bool
}

/// 依序嘗試：讀第一個有的；寫第一個成功的。鑰匙圈寫不進去（例如沒簽章的模擬器）就退回檔案，
/// 否則每次重開 App 都會換一個身分，Mac 會把它當成不認識的裝置。
public final class SecretChain {
    private let stores: [SecretStore]
    public init(_ stores: [SecretStore]) { self.stores = stores }
    public func read(_ account: String) -> Data? {
        for s in stores { if let d = s.read(account) { return d } }
        return nil
    }
    @discardableResult
    public func write(_ account: String, _ data: Data) -> Bool {
        stores.contains { $0.write(account, data) }
    }
}

/// 中轉站連線的狀態與重連（AC-MA-17）：每次開新連線都有自己的編號；只有目前這一條的心跳與失敗算數，
/// 被取代或取消的舊連線送來的結果一律忽略——否則取消舊連線的那個失敗會把新連線標成離線、再觸發重連，無限閃爍。
public struct Reconnect: Equatable, Sendable {
    public enum Action: Equatable, Sendable {
        case ignore
        case online
        case offline(retryAfter: Double)
    }

    private var current = 0
    private var isOnline = false
    private var closed = true
    private var currentFailed = false
    private var backoff: Double = 1

    public init() {}

    /// 開一條新連線（取代舊的）。
    public mutating func open() -> Int {
        current += 1
        closed = false
        currentFailed = false
        isOnline = false // 新連線要自己確認過才算（畫面不會因此先變離線：只有 failed 才回 .offline）
        return current
    }

    /// 這條連線的心跳成功。
    public mutating func alive(_ id: Int) -> Action {
        guard id == current, !closed, !currentFailed, !isOnline else { return .ignore }
        isOnline = true
        backoff = 1
        return .online
    }

    /// 這條連線失敗（收訊失敗、心跳失敗）。
    public mutating func failed(_ id: Int) -> Action {
        guard id == current, !closed, !currentFailed else { return .ignore }
        currentFailed = true
        isOnline = false
        let wait = backoff
        backoff = min(backoff * 2, 30)
        return .offline(retryAfter: wait)
    }

    /// 刻意關掉（解除配對、換連線對象）：之後什麼都不重連。
    public mutating func close() {
        closed = true
        isOnline = false
    }
}
