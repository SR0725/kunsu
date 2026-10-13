import Foundation
import CryptoKit
import Security

/// Secure Enclave 裡的 P-256 簽章金鑰：私鑰不可匯出，只能在這支手機上用（docs/design/mobile-relay.md）。
struct SecureEnclaveSigner: P256Signer, @unchecked Sendable {
    let key: SecureEnclave.P256.Signing.PrivateKey
    var publicKeyX963: Data { key.publicKey.x963Representation }
    func sign(_ data: Data) throws -> Data { try key.signature(for: data).rawRepresentation }
}

/// iOS 鑰匙圈（只限這台裝置、解鎖後可用）。
final class KeychainStore: SecretStore {
    private let service = "com.rayrealms.kunsu.mobile.identity"

    func read(_ account: String) -> Data? {
        let q: [String: Any] = [kSecClass as String: kSecClassGenericPassword, kSecAttrService as String: service,
                                kSecAttrAccount as String: account, kSecReturnData as String: true]
        var out: CFTypeRef?
        return SecItemCopyMatching(q as CFDictionary, &out) == errSecSuccess ? out as? Data : nil
    }

    func write(_ account: String, _ data: Data) -> Bool {
        let base: [String: Any] = [kSecClass as String: kSecClassGenericPassword, kSecAttrService as String: service,
                                   kSecAttrAccount as String: account]
        SecItemDelete(base as CFDictionary)
        var add = base
        add[kSecValueData as String] = data
        add[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        return SecItemAdd(add as CFDictionary, nil) == errSecSuccess && read(account) == data
    }
}

/// 鑰匙圈不能用時的退路：App 自己的 Application Support，檔案保護到第一次解鎖、不進備份。
final class ProtectedFileStore: SecretStore {
    private var dir: URL? {
        guard let base = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first else { return nil }
        let d = base.appendingPathComponent("identity", isDirectory: true)
        try? FileManager.default.createDirectory(at: d, withIntermediateDirectories: true)
        var v = URLResourceValues()
        v.isExcludedFromBackup = true
        var dd = d
        try? dd.setResourceValues(v)
        return d
    }

    func read(_ account: String) -> Data? {
        dir.flatMap { try? Data(contentsOf: $0.appendingPathComponent(account)) }
    }

    func write(_ account: String, _ data: Data) -> Bool {
        guard let url = dir?.appendingPathComponent(account) else { return false }
        return (try? data.write(to: url, options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])) != nil
    }
}

/// 手機身分。私鑰先放鑰匙圈、不行就放受保護的檔案（SecretChain）；簽章金鑰在 Secure Enclave（模擬器退回軟體金鑰）。
enum IdentityStore {
    private static let vault = SecretChain([KeychainStore(), ProtectedFileStore()])

    static func loadOrCreate() throws -> DeviceIdentity {
        let kx: Curve25519.KeyAgreement.PrivateKey
        if let raw = vault.read("kx"), let k = try? Curve25519.KeyAgreement.PrivateKey(rawRepresentation: raw) {
            kx = k
        } else {
            kx = Curve25519.KeyAgreement.PrivateKey()
            vault.write("kx", kx.rawRepresentation)
        }
        if SecureEnclave.isAvailable {
            if let blob = vault.read("sig-se"), let k = try? SecureEnclave.P256.Signing.PrivateKey(dataRepresentation: blob) {
                return DeviceIdentity(kx: kx, signer: SecureEnclaveSigner(key: k))
            }
            let k = try SecureEnclave.P256.Signing.PrivateKey()
            vault.write("sig-se", k.dataRepresentation)
            return DeviceIdentity(kx: kx, signer: SecureEnclaveSigner(key: k))
        }
        if let raw = vault.read("sig-sw"), let k = try? P256.Signing.PrivateKey(rawRepresentation: raw) {
            return DeviceIdentity(kx: kx, signer: SoftwareSigner(k))
        }
        let k = P256.Signing.PrivateKey()
        vault.write("sig-sw", k.rawRepresentation)
        return DeviceIdentity(kx: kx, signer: SoftwareSigner(k))
    }
}

/// 配對成功後記住的 Mac（都是公開資訊）。
struct Pairing: Codable, Equatable {
    var relay: String
    var mac: MacPublicKeys
    var pairedAt: Date

    private static let key = "pairing.v1"
    static func load() -> Pairing? {
        UserDefaults.standard.data(forKey: key).flatMap { try? JSONDecoder().decode(Pairing.self, from: $0) }
    }
    func save() { UserDefaults.standard.set(try? JSONEncoder().encode(self), forKey: Pairing.key) }
    static func clear() { UserDefaults.standard.removeObject(forKey: key) }
}
