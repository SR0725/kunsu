import XCTest
@testable import KunsuCore

/// 手機身分要在重開 App 後不變（否則 Mac 會把它當成不認識的裝置）：鑰匙圈寫不進去時退回檔案。
final class KeyStoreTests: XCTestCase {
    final class Memory: SecretStore {
        var items: [String: Data] = [:]
        let accepts: Bool
        init(accepts: Bool) { self.accepts = accepts }
        func read(_ account: String) -> Data? { items[account] }
        func write(_ account: String, _ data: Data) -> Bool {
            guard accepts else { return false }
            items[account] = data
            return true
        }
    }

    func test_falls_back_when_the_first_store_refuses() {
        let keychain = Memory(accepts: false), file = Memory(accepts: true)
        let chain = SecretChain([keychain, file])
        XCTAssertTrue(chain.write("kx", Data([1, 2, 3])))
        XCTAssertNil(keychain.items["kx"])
        XCTAssertEqual(chain.read("kx"), Data([1, 2, 3]), "重開後讀得回來")
    }

    func test_prefers_the_first_store_and_reads_it_first() {
        let keychain = Memory(accepts: true), file = Memory(accepts: true)
        let chain = SecretChain([keychain, file])
        XCTAssertTrue(chain.write("kx", Data([9])))
        XCTAssertEqual(keychain.items["kx"], Data([9]))
        XCTAssertNil(file.items["kx"], "鑰匙圈能用就不寫檔")
        file.items["kx"] = Data([0])
        XCTAssertEqual(chain.read("kx"), Data([9]))
    }

    func test_nothing_writable_is_reported() {
        XCTAssertFalse(SecretChain([Memory(accepts: false)]).write("kx", Data([1])))
    }
}
