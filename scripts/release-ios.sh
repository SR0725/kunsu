#!/usr/bin/env bash
# iPhone App 打包並上傳到 App Store Connect（TestFlight／送審用）。
# 簽章用 Xcode 已登入的帳號（自動管理）；有 KUNSU_ASC_KEY／KUNSU_ASC_KEY_ID／KUNSU_ASC_ISSUER 時改用 API 金鑰。
# Apple Developer team 用 KUNSU_TEAM_ID（預設沿用 mobile/project.yml 的 DEVELOPMENT_TEAM）。
# 用法：scripts/release-ios.sh [build 號碼，預設用時間戳]
set -euo pipefail
cd "$(dirname "$0")/../mobile"
BUILD="${1:-$(date +%Y%m%d%H%M)}"
TEAM_ID="${KUNSU_TEAM_ID:-J9Z63W9C42}"
OUT=../dist/ios
rm -rf "$OUT" && mkdir -p "$OUT"

xcodegen generate >/dev/null
AUTH=()
if [[ -n "${KUNSU_ASC_KEY:-}" && -n "${KUNSU_ASC_KEY_ID:-}" && -n "${KUNSU_ASC_ISSUER:-}" ]]; then
  AUTH=(-authenticationKeyPath "$KUNSU_ASC_KEY" -authenticationKeyID "$KUNSU_ASC_KEY_ID" -authenticationKeyIssuerID "$KUNSU_ASC_ISSUER")
fi

xcodebuild -project KunsuMobile.xcodeproj -scheme KunsuMobile -configuration Release \
  -destination 'generic/platform=iOS' -archivePath "$OUT/Kunsu.xcarchive" \
  CURRENT_PROJECT_VERSION="$BUILD" DEVELOPMENT_TEAM="$TEAM_ID" -allowProvisioningUpdates ${AUTH[@]+"${AUTH[@]}"} archive | tail -3

cat > "$OUT/export.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>method</key><string>app-store-connect</string>
  <key>destination</key><string>${KUNSU_IOS_DESTINATION:-upload}</string>
  <key>teamID</key><string>${TEAM_ID}</string>
  <key>signingStyle</key><string>automatic</string>
  <key>manageAppVersionAndBuildNumber</key><false/>
</dict></plist>
PLIST

xcodebuild -exportArchive -archivePath "$OUT/Kunsu.xcarchive" -exportOptionsPlist "$OUT/export.plist" \
  -exportPath "$OUT" -allowProvisioningUpdates ${AUTH[@]+"${AUTH[@]}"} | tail -5
echo "✅ build $BUILD 完成（${KUNSU_IOS_DESTINATION:-upload}）"
