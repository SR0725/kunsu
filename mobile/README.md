# Kunsu 手機 App（KunsuMobile）

PRD §30、DESIGN.md「Unified design language」、docs/design/mobile-relay.md。

- `KunsuCore/`：純邏輯（一問一答狀態機、配對與中轉加密），`swift test` 在 Mac 上跑。
- `KunsuMobile/`：SwiftUI 畫面、Secure Enclave 身分、中轉站連線（LiveLink）。

```bash
cd mobile && xcodegen generate
# 模擬器
xcodebuild -project KunsuMobile.xcodeproj -target KunsuMobile -sdk iphonesimulator -configuration Debug \
  ARCHS=arm64 ONLY_ACTIVE_ARCH=YES SYMROOT=$PWD/build/sym CODE_SIGNING_ALLOWED=NO build
# 實機（用 project.yml 的 DEVELOPMENT_TEAM；請換成你自己的 Apple Developer team）
xcodebuild -project KunsuMobile.xcodeproj -target KunsuMobile -sdk iphoneos -configuration Debug \
  -allowProvisioningUpdates SYMROOT=$PWD/build/dev build
xcrun devicectl device install app --device <iPhone id> build/dev/Debug-iphoneos/KunsuMobile.app
```

用 target 而不用 scheme：本機 Xcode 26.3 沒有 iOS 26.2 平台元件，scheme 的 destination 會失敗；也因此沒有 asset catalog（沒有 App
icon），K 圖示直接讀 `Resources/kmark.png`。
