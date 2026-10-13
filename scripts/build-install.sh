#!/usr/bin/env bash
# Kunsu build + install（CONTRACT §0 launchd / BDD survival 場景 8–9）。
#
# 流程：ui build → tauri build → 備份 /Applications/Kunsu.app（.bak）→ 拷貝新
# bundle → 寫 launchd plist（**指 /Applications/Kunsu.app，絕不指 repo build
# 路徑**——task-engine zombie plist 的直接死因）→ bootout/bootstrap → doctor
# 冒煙（非互動層）→ 失敗自動回滾 .bak + 重新 bootstrap。
#
# 本腳本只由使用者（或取得明確同意的 agent）在終端執行。

set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP_NAME="Kunsu.app"
APP_DST="/Applications/${APP_NAME}"
APP_BAK="/Applications/${APP_NAME}.bak"
PLIST_LABEL="me.ray.kunsu"
PLIST_PATH="${HOME}/Library/LaunchAgents/${PLIST_LABEL}.plist"
KUNSU_DIR="${HOME}/.kunsu"
GUI_DOMAIN="gui/$(id -u)"

log() { printf '\n\033[1;36m[build-install]\033[0m %s\n' "$*"; }
fail() { printf '\n\033[1;31m[build-install] ❌ %s\033[0m\n' "$*" >&2; exit 1; }

# ---------------------------------------------------------------------------
# 0. 前置檢查
# ---------------------------------------------------------------------------
command -v pnpm >/dev/null || fail "缺 pnpm"
command -v cargo >/dev/null || fail "缺 cargo"
if ! cargo tauri --version >/dev/null 2>&1; then
  fail "缺 tauri-cli。先跑：cargo install tauri-cli --locked"
fi

# ---------------------------------------------------------------------------
# 1. build（ui → tauri bundle → kunsu CLI）
# ---------------------------------------------------------------------------
log "ui pnpm build"
(cd "${REPO}/ui" && pnpm install --frozen-lockfile && pnpm build)

log "cargo tauri build（.app bundle）"
(cd "${REPO}/src-tauri" && cargo tauri build)

APP_SRC="$(find "${REPO}/target/release/bundle/macos" -maxdepth 1 -name "${APP_NAME}" | head -1)"
[ -n "${APP_SRC}" ] || fail "找不到 build 產物 ${APP_NAME}（target/release/bundle/macos）"

# 穩定簽章身分。ad-hoc 簽章（--sign -）的 designated requirement 是整個程式的 cdhash：每次重建都不同，
# macOS 把它當成新 app，「桌面／完全取用磁碟」與 Keychain 授權全部失效、檔案呼叫卡在權限對話框
# （2026-08-01 假死事故與 2026-10-09 帳本卡住的根因）。只釘 identifier 不夠。
# 鑰匙圈裡有「Kunsu Local Signing」程式碼簽署憑證（自簽、一次性由使用者建立）就用它簽：requirement 變成
# identifier＋憑證，跨版本不變，授權一次就好。沒有就退回 ad-hoc 並大聲提醒。
SIGN_IDENTITY="${KUNSU_SIGN_IDENTITY:-Kunsu Local Signing}"
if security find-certificate -c "${SIGN_IDENTITY}" >/dev/null 2>&1; then
  log "codesign（固定憑證「${SIGN_IDENTITY}」，授權跨版本延續）"
  codesign --force --deep --sign "${SIGN_IDENTITY}" --identifier me.ray.kunsu "${APP_SRC}" >/dev/null 2>&1 || \
    fail "codesign 失敗：找得到憑證「${SIGN_IDENTITY}」但簽不了（確認它是「程式碼簽署」類型、私鑰在登入鑰匙圈）"
else
  log "codesign（ad-hoc identifier me.ray.kunsu）"
  codesign --force --deep --sign - --identifier me.ray.kunsu "${APP_SRC}" >/dev/null 2>&1 || \
    log "⚠️ codesign 失敗（不阻斷安裝）"
  log "⚠️ 沒有固定簽章憑證「${SIGN_IDENTITY}」：這次安裝後 macOS 會把 Kunsu 當成新 app，要重新允許「桌面」存取，\
否則帳本寫入會卡住。一次性修法：鑰匙圈存取 → 憑證輔助程式 → 製作憑證，名稱「${SIGN_IDENTITY}」、身分類型「自簽根憑證」、\
憑證類型「程式碼簽署」，之後重跑本腳本。"
fi

log "cargo build --release -p kunsu-cli（kunsu binary）"
(cd "${REPO}" && cargo build --release -p kunsu-cli)
KUNSU_BIN="${REPO}/target/release/kunsu"
mkdir -p "${KUNSU_DIR}/bin"
install -m 755 "${KUNSU_BIN}" "${KUNSU_DIR}/bin/kunsu"
log "kunsu CLI 已裝到 ${KUNSU_DIR}/bin/kunsu（建議 PATH 加入 ~/.kunsu/bin）"

# ---------------------------------------------------------------------------
# 2. policy / playbooks symlink（CONTRACT §0）
# ---------------------------------------------------------------------------
mkdir -p "${KUNSU_DIR}"
# ⚠️ 2026-08-01 事故：這裡原本把 policy.toml / playbooks symlink 到 Desktop repo，
# 而 macOS 對 Desktop 的檔案存取授權在 app 重簽名後失效 → GUI 啟動時 open() 無限期卡住
# （無錯誤、無 log、port 永不 bind）。改為**實體複製到 ~/.kunsu**（不碰受保護目錄）。
# 代價：改 repo config 後要重跑本腳本（或手動 cp）才生效——換到啟動永不假死。
# 只換 policy.toml；設定頁的個人值 ${KUNSU_DIR}/policy.local.toml 永不碰（CONTRACT §15：重裝後設定仍在）。
rm -f "${KUNSU_DIR}/policy.toml"
cp "${REPO}/config/policy.toml" "${KUNSU_DIR}/policy.toml"
rm -rf "${KUNSU_DIR}/playbooks"
cp -R "${REPO}/playbooks" "${KUNSU_DIR}/playbooks"
log "policy/playbooks 已複製到 ${KUNSU_DIR}（實體檔，非 symlink——避開 Desktop 授權假死）"

# ---------------------------------------------------------------------------
# 3. 停舊實例 → 備份 → 拷貝新 bundle
# ---------------------------------------------------------------------------
log "launchctl bootout（舊實例若在跑先停）"
launchctl bootout "${GUI_DOMAIN}" "${PLIST_PATH}" 2>/dev/null || true
# Gracefully stop only this installed executable; no UI scripting.
  pkill -TERM -f '^/Applications/Kunsu[.]app/Contents/MacOS/kunsu-app($| )' 2>/dev/null || true
# 等 4777 真正釋放（舊 app 退出可能 >1s；埠沒讓出新 app 會觸發雙開防護直接退出——
# 2026-08-01 首次重裝就是這個競態害 doctor 全紅誤判回滾）
for i in $(seq 1 40); do
  lsof -nP -iTCP:4777 -sTCP:LISTEN >/dev/null 2>&1 || break
  sleep 0.5
done
if lsof -nP -iTCP:4777 -sTCP:LISTEN >/dev/null 2>&1; then
  fail "舊實例 20 秒仍未釋放 4777，中止（請手動退出 Kunsu 再重跑）"
fi

ROLLBACK_READY=0
if [ -d "${APP_DST}" ]; then
  log "備份既有 ${APP_DST} → ${APP_BAK}"
  if [ -e "${APP_BAK}" ]; then
    mv "${APP_BAK}" "${APP_BAK}.$(date +%Y%m%d-%H%M%S)"
  fi
  mv "${APP_DST}" "${APP_BAK}"
  ROLLBACK_READY=1
fi

log "拷貝 ${APP_SRC} → ${APP_DST}"
cp -R "${APP_SRC}" "${APP_DST}"

# ---------------------------------------------------------------------------
# 4. launchd plist（絕不指 repo build 路徑）
# ---------------------------------------------------------------------------
log "寫 ${PLIST_PATH}（ProgramArguments = open -a ${APP_DST}）"
mkdir -p "$(dirname "${PLIST_PATH}")"
cat > "${PLIST_PATH}" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key><string>${PLIST_LABEL}</string>
    <key>ProgramArguments</key>
    <array>
        <string>/usr/bin/open</string>
        <string>-a</string>
        <string>${APP_DST}</string>
    </array>
    <key>RunAtLoad</key><true/>
    <key>KeepAlive</key><false/>
    <key>StandardOutPath</key><string>/tmp/${PLIST_LABEL}.out.log</string>
    <key>StandardErrorPath</key><string>/tmp/${PLIST_LABEL}.err.log</string>
</dict>
</plist>
PLIST

rollback() {
  printf '\n\033[1;31m[build-install] ⛔ doctor 失敗——自動回滾\033[0m\n' >&2
  launchctl bootout "${GUI_DOMAIN}" "${PLIST_PATH}" 2>/dev/null || true
  # Gracefully stop only this installed executable; no UI scripting.
  pkill -TERM -f '^/Applications/Kunsu[.]app/Contents/MacOS/kunsu-app($| )' 2>/dev/null || true
  sleep 1
  if [ "${ROLLBACK_READY}" = "1" ] && [ -d "${APP_BAK}" ]; then
    rm -rf "${APP_DST}"
    mv "${APP_BAK}" "${APP_DST}"
    launchctl bootstrap "${GUI_DOMAIN}" "${PLIST_PATH}" || true
    launchctl kickstart "${GUI_DOMAIN}/${PLIST_LABEL}" 2>/dev/null || true
    # 與主路徑同樣輪詢健康最多 30 秒（冷啟動含 webview 常 >3s；固定 sleep 會誤判「回滾後仍失敗」）
    for i in $(seq 1 60); do
      curl -s --max-time 1 http://127.0.0.1:4777/health | grep -q uptime_s && break
      sleep 0.5
    done
    if "${KUNSU_DIR}/bin/kunsu" doctor --install-gate </dev/null; then
      echo "[build-install] 回滾完成：舊版 Kunsu 已恢復且 doctor 通過" >&2
    else
      echo "[build-install] ⚠️ 回滾後 doctor 仍失敗——Kunsu 可能整體不健康，需要人工介入" >&2
    fi
  else
    echo "[build-install] 沒有 .bak 可回滾（首次安裝失敗）——已停用 launchd，需要人工介入" >&2
  fi
  exit 1
}

# ---------------------------------------------------------------------------
# 5. bootstrap → doctor 冒煙（非互動層）→ 失敗回滾
# ---------------------------------------------------------------------------
log "launchctl bootstrap"
launchctl bootstrap "${GUI_DOMAIN}" "${PLIST_PATH}" || fail "launchctl bootstrap 失敗"
# macOS 有時 bootstrap 後不跑 RunAtLoad（runs=0），主動踢一次；App 已在跑時 open -a 沒有副作用
launchctl kickstart "${GUI_DOMAIN}/${PLIST_LABEL}" 2>/dev/null || true
# 輪詢健康最多 30 秒（冷啟動含 webview 可能 >3s），起來了才進 doctor
for i in $(seq 1 60); do
  curl -s --max-time 1 http://127.0.0.1:4777/health | grep -q uptime_s && break
  sleep 0.5
done

log "kunsu doctor（非互動層冒煙）"
if ! "${KUNSU_DIR}/bin/kunsu" doctor --install-gate </dev/null; then
  rollback
fi

log "✅ 安裝完成：${APP_DST}（launchd ${PLIST_LABEL} 已載入）"
if [ "${ROLLBACK_READY}" = "1" ]; then
  log "上一版備份保留在 ${APP_BAK}（下次安裝先加時間戳保存）"
fi
log "提醒：MCP 註冊是一次性人工步驟（doctor 會印出確切指令）；首次安裝 doctor 若只有 MCP 註冊項失敗，先照指令註冊再重跑 doctor"
