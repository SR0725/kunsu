// 瀏海來源圖示中心回報（BDD overlay-presentation AC-OP-02）：題目自動到期、從選單列開啟時沒有點擊座標，原生層用最後回報
// 的圖示中心當蓋板的轉場起點，不再固定在左上角。
// 瀏海本身（ui/src/edge/）由主線維護；這裡在瀏海視窗外層包住 ipc：每次瀏海回報命中區（位置或長度改變）後，量一次
// 來源圖示（`.edge-stage [data-agent-mark]`）的中心，有移動才回報。
import type { KunsuIpc } from './ipc'

export interface NotchAnchor {
  x: number
  y: number
  size: number
}

/** 圖示外框（舞台本地 CSS px，舞台貼滿視窗所以等於 viewport 座標）→ 中心與大小。 */
export function anchorFromRect(rect: { left: number; top: number; width: number; height: number }): NotchAnchor {
  // 大小取整數：向量外框可能是 17.1 之類的小數，取整數讓「有沒有移動」的判斷穩定。
  return { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2, size: Math.round(Math.max(rect.width, rect.height)) }
}

/** 變動超過 0.5 px（或大小改變）才算移動；第一次一定送。 */
export function anchorMoved(previous: NotchAnchor | null, next: NotchAnchor): boolean {
  if (!previous) return true
  return Math.abs(previous.x - next.x) > 0.5 || Math.abs(previous.y - next.y) > 0.5 || previous.size !== next.size
}

/** 在瀏海視窗裡量來源圖示的中心；瀏海沒畫出來時回 null。 */
export function readNotchMark(root: ParentNode = document): NotchAnchor | null {
  const mark = root.querySelector('.edge-stage [data-agent-mark]')
  if (!mark) return null
  const rect = mark.getBoundingClientRect()
  if (rect.width <= 0 || rect.height <= 0) return null
  return anchorFromRect(rect)
}

/**
 * 包住瀏海用的 ipc：`edgeHit` 照常送出，接著排一次量測（`schedule`，正式環境是 150 ms 後，DOM 已更新；同一段時間內的
 * 多次命中區更新只量一次），圖示有移動才 `edgeAnchor`。拖曳中不量（起點只在靜止時有用），放開後的最後一次更新會量到。
 */
export function withNotchAnchorReport(
  ipc: KunsuIpc,
  read: () => NotchAnchor | null = readNotchMark,
  schedule: (task: () => void) => void = (task) => void window.setTimeout(task, 150),
): KunsuIpc {
  let last: NotchAnchor | null = null
  let queued = false
  const measure = () => {
    queued = false
    const anchor = read()
    if (!anchor || !anchorMoved(last, anchor)) return
    last = anchor
    void ipc.edgeAnchor(anchor).catch((error) => console.error('瀏海圖示位置回報失敗', error))
  }
  return {
    ...ipc,
    edgeHit: async (shapes, dragging) => {
      const sent = ipc.edgeHit(shapes, dragging)
      if (!queued && !dragging) {
        queued = true
        schedule(measure)
      }
      await sent
    },
  }
}
