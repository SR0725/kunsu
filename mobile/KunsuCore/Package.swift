// swift-tools-version: 5.9
import PackageDescription

// 手機 App 的純邏輯（BDD mobile-app AC-MA-01～11）。不依賴 UIKit，在 Mac 上用 `swift test` 驗證。
let package = Package(
    name: "KunsuCore",
    platforms: [.iOS(.v17), .macOS(.v14)],
    products: [.library(name: "KunsuCore", targets: ["KunsuCore"])],
    targets: [
        .target(name: "KunsuCore"),
        .testTarget(name: "KunsuCoreTests", dependencies: ["KunsuCore"]),
    ]
)
