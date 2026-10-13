import XCTest
import CryptoKit
@testable import KunsuCore

/// AC-MP-04：Swift 與 Rust 用同一份測試向量（docs/design/mobile-relay-vector.json）互通。
final class RelayCryptoTests: XCTestCase {
    private func vector() throws -> [String: Any] {
        let url = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("docs/design/mobile-relay-vector.json")
        return try JSONSerialization.jsonObject(with: Data(contentsOf: url)) as! [String: Any]
    }

    private func identity(_ v: [String: Any]) throws -> DeviceIdentity {
        let seeds = v["seeds"] as! [String: String]
        return try DeviceIdentity(
            kx: Curve25519.KeyAgreement.PrivateKey(rawRepresentation: B64.decode(seeds["dev_kx"]!)!),
            signer: SoftwareSigner(P256.Signing.PrivateKey(rawRepresentation: B64.decode(seeds["dev_sig"]!)!))
        )
    }

    private func macKeys(_ v: [String: Any]) -> MacPublicKeys {
        MacPublicKeys(id: v["mac_id"] as! String, kx: B64.decode(v["mac_kx_pub"] as! String)!, sig: B64.decode(v["mac_sig_pub"] as! String)!)
    }

    func test_ids_match_rust() throws {
        let v = try vector()
        let me = try identity(v)
        XCTAssertEqual(me.deviceID, v["device_id"] as? String)
        XCTAssertEqual(B64.encode(me.signer.publicKeyX963), v["dev_sig_pub"] as? String)
        XCTAssertEqual(ShortID.of(macKeys(v).sig), v["mac_id"] as? String)
    }

    func test_pair_request_bytes_match_rust() throws {
        let v = try vector()
        let me = try identity(v)
        let pair = v["pair"] as! [String: Any]
        let seeds = v["seeds"] as! [String: String]
        let wire = try me.pairRequest(
            mac: macKeys(v), code: pair["code"] as! String, deviceName: pair["device_name"] as! String,
            ts: pair["ts"] as! Int64,
            ephemeral: Curve25519.KeyAgreement.PrivateKey(rawRepresentation: B64.decode(seeds["eph"]!)!),
            nonce: B64.decode(pair["nonce"] as! String)!
        )
        let rust = pair["wire"] as! [String: String]
        XCTAssertEqual(wire.eph, rust["eph"])
        XCTAssertEqual(wire.ct, rust["ct"])
    }

    func test_device_envelope_matches_rust_ciphertext_and_rust_signature_verifies() throws {
        let v = try vector()
        let me = try identity(v)
        let d2m = v["d2m"] as! [String: Any]
        let rust = try JSONDecoder().decode(Envelope.self, from: JSONSerialization.data(withJSONObject: d2m["envelope"]!))
        let mine = try me.seal(mac: macKeys(v), seq: rust.seq, ts: rust.ts,
                               plaintext: Data((d2m["plaintext"] as! String).utf8), nonce: B64.decode(rust.nonce)!)
        XCTAssertEqual(mine.ct, rust.ct)
        // ECDSA 每次簽章不同：互相驗得過就是互通
        let pub = try P256.Signing.PublicKey(x963Representation: me.signer.publicKeyX963)
        let sig = try P256.Signing.ECDSASignature(rawRepresentation: B64.decode(rust.sig)!)
        XCTAssertTrue(pub.isValidSignature(sig, for: Data(Envelope.signingInput(rust).utf8)))
        let mySig = try P256.Signing.ECDSASignature(rawRepresentation: B64.decode(mine.sig)!)
        XCTAssertTrue(pub.isValidSignature(mySig, for: Data(Envelope.signingInput(mine).utf8)))
    }

    func test_open_mac_envelope_and_reject_forgery() throws {
        let v = try vector()
        let me = try identity(v)
        let m2d = v["m2d"] as! [String: Any]
        var env = try JSONDecoder().decode(Envelope.self, from: JSONSerialization.data(withJSONObject: m2d["envelope"]!))
        let plain = try me.open(fromMac: macKeys(v), env)
        XCTAssertEqual(String(data: plain, encoding: .utf8), m2d["plaintext"] as? String)
        env.seq += 1
        XCTAssertThrowsError(try me.open(fromMac: macKeys(v), env))
    }

    func test_pair_accepted_signature() throws {
        let v = try vector()
        let me = try identity(v)
        let acc = try JSONDecoder().decode(PairAccepted.self, from: JSONSerialization.data(withJSONObject: v["pair_accepted"]!))
        XCTAssertTrue(me.verify(acc, mac: macKeys(v)))
        var forged = acc
        forged.ts += 1
        XCTAssertFalse(me.verify(forged, mac: macKeys(v)))
    }

    func test_qr_payload_parses() throws {
        let v = try vector()
        let s = "kunsu-pair:v1?relay=wss%3A%2F%2Fkunsu-relay.zeabur.app%2Fv1&mac=\(v["mac_id"]!)&code=ABCD2345&kx=\(v["mac_kx_pub"]!)&sig=\(v["mac_sig_pub"]!)"
        let p = try XCTUnwrap(PairingLink(s))
        XCTAssertEqual(p.relay, "wss://kunsu-relay.zeabur.app/v1")
        XCTAssertEqual(p.code, "ABCD2345")
        XCTAssertEqual(p.mac.id, v["mac_id"] as? String)
        XCTAssertNil(PairingLink("https://evil.example"))
        // mac_id 跟公鑰對不上的 QR 拒收
        XCTAssertNil(PairingLink(s.replacingOccurrences(of: "mac=\(v["mac_id"]!)", with: "mac=aaaaaaaaaaaaaaaa")))
    }
}
