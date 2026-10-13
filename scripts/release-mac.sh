#!/usr/bin/env bash
# 公開版 Mac App（PRD §31.4）：Developer ID 簽章（hardened runtime）→ DMG → Apple 公證 → staple。
# 產物：dist/Kunsu-<版本>.dmg。公證需要 App Store Connect API 金鑰：
#   KUNSU_NOTARY_KEY=~/.appstoreconnect/private_keys/AuthKey_XXXX.p8 KUNSU_NOTARY_KEY_ID=XXXX KUNSU_NOTARY_ISSUER=<issuer uuid>
# 沒有就只簽章與打包，印出提醒（不能拿去公開發佈）。
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# 鑰匙圈裡可能有兩份同名的 Developer ID 憑證：用登入鑰匙圈那份的雜湊，避免「ambiguous」
IDENTITY="${KUNSU_DEVELOPER_ID:-$(security find-identity -v -p codesigning ~/Library/Keychains/login.keychain-db | grep "Developer ID Application" | head -1 | awk '{print $2}')}"
[ -n "${IDENTITY}" ] || { echo "找不到 Developer ID Application 憑證" >&2; exit 1; }
VERSION="$(python3 -c "import json;print(json.load(open('${REPO}/src-tauri/tauri.conf.json'))['version'])")"
OUT="${REPO}/dist"
DMG="${OUT}/Kunsu-${VERSION}.dmg"

log() { printf '\n\033[1;36m[release-mac]\033[0m %s\n' "$*"; }
fail() { printf '\n\033[1;31m[release-mac] ❌ %s\033[0m\n' "$*" >&2; exit 1; }

log "ui build"
(cd "${REPO}/ui" && pnpm install --frozen-lockfile && pnpm build)
log "cargo tauri build（內含 kunsu CLI、policy、playbooks、外掛）"
(cd "${REPO}/src-tauri" && cargo tauri build)
APP="${REPO}/target/release/bundle/macos/Kunsu.app"
[ -d "${APP}" ] || fail "找不到 ${APP}"

log "Developer ID 簽章（由內而外、hardened runtime）"
find "${APP}/Contents/Resources" -type f \( -perm -u+x -o -name "*.dylib" \) -print0 | while IFS= read -r -d '' f; do
  if file "$f" | grep -q "Mach-O"; then
    codesign --force --options runtime --timestamp --sign "${IDENTITY}" "$f"
  fi
done
codesign --force --options runtime --timestamp --sign "${IDENTITY}" "${APP}/Contents/MacOS/kunsu-app"
codesign --force --options runtime --timestamp --sign "${IDENTITY}" --identifier me.ray.kunsu "${APP}"
codesign --verify --deep --strict --verbose=2 "${APP}" 2>&1 | tail -2

log "DMG"
mkdir -p "${OUT}"
STAGE="$(mktemp -d)"
cp -R "${APP}" "${STAGE}/"
ln -s /Applications "${STAGE}/Applications"
rm -f "${DMG}"
hdiutil create -volname "Kunsu" -srcfolder "${STAGE}" -ov -format UDZO "${DMG}" >/dev/null
rm -rf "${STAGE}"
codesign --force --timestamp --sign "${IDENTITY}" "${DMG}"

if [ -n "${KUNSU_NOTARY_KEY:-}" ] && [ -n "${KUNSU_NOTARY_KEY_ID:-}" ] && [ -n "${KUNSU_NOTARY_ISSUER:-}" ]; then
  log "Apple 公證（可能要幾分鐘）"
  xcrun notarytool submit "${DMG}" --key "${KUNSU_NOTARY_KEY}" --key-id "${KUNSU_NOTARY_KEY_ID}" --issuer "${KUNSU_NOTARY_ISSUER}" --wait
  xcrun stapler staple "${DMG}"
  spctl --assess --type open --context context:primary-signature --verbose=2 "${DMG}" 2>&1 | tail -1
  log "✅ ${DMG}（已公證，可以公開下載）"
else
  log "⚠️ 沒有公證金鑰：${DMG} 只有簽章，別人下載會被 Gatekeeper 擋。設定 KUNSU_NOTARY_* 後重跑。"
fi
