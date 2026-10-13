// 瀏海回到預設位置（BDD settings 場景 9、AC-ST-09 的 UI 這一半；BDD edge-notch 場景 3b／3e）。2026-10-09 QA 第 5 輪改成明確
// 事件：shell 刪檔後發 `kunsu://edge_position_reset`，網頁取消待寫的存檔、清掉記憶體位置、回到預設位置；`edge_stage_changed`
// 只重新貼合與重讀。以前以「同一台螢幕重讀時保存值從有變成沒有」猜重設（`savedPositionCleared`），會把「延後 400 ms 的寫檔
// 還沒發生／寫檔失敗」誤當成重設，瀏海跳回預設位置；拖完 400 ms 內按重設又被延後的寫檔蓋回去。
import { describe, expect, it } from 'vitest'
import {
  SAVED_MEMORY_INIT,
  defaultCenter,
  makeStage,
  restoreCenter,
  saveStillCurrent,
  savedAfterRead,
  savedAfterRelease,
  savedAfterReset,
  type SavedPosition,
} from '../edge/geometry'

describe('瀏海回到預設位置', () => {
  const dropped: SavedPosition = { edge: 'right', head: 0.4 }

  it('同一台螢幕重讀時保存值是 null（延後的寫檔還沒發生或寫檔失敗）不是重設：記憶體位置不變', () => {
    const read = savedAfterRead(SAVED_MEMORY_INIT, '#1', null)
    expect(read).toEqual({ display: '#1', saved: null, resetGen: 0 })
    // 放開：記憶體立即更新，寫檔延後 400 ms
    const released = savedAfterRelease(read, dropped)
    expect(released.saved).toBe(dropped)
    // 400 ms 內舞台重讀（睡眠喚醒、螢幕配置改變）：檔案裡還沒有，edge_info 回 saved: null
    expect(savedAfterRead(released, '#1', null)).toBe(released)
    // 寫檔失敗後重讀也一樣
    expect(savedAfterRead(released, '#1', undefined)).toBe(released)
  })

  it('換了螢幕才換成那台螢幕的保存值', () => {
    const onA = savedAfterRelease(savedAfterRead(SAVED_MEMORY_INIT, '#1', null), dropped)
    const onB = savedAfterRead(onA, '#2', { corner: 'tl' })
    expect(onB).toEqual({ display: '#2', saved: { corner: 'tl' }, resetGen: 0 })
    expect(savedAfterRead(onB, 'Studio Display', null)).toEqual({ display: 'Studio Display', saved: null, resetGen: 0 })
  })

  it('回到預設位置（edge_position_reset）：清掉記憶體位置、作廢排程中的寫檔；拖完 400 ms 內按也不被蓋回去', () => {
    const released = savedAfterRelease(savedAfterRead(SAVED_MEMORY_INIT, '#1', null), dropped)
    const gen = released.resetGen // 放開時排定寫檔，記下世代
    expect(saveStillCurrent(released, gen)).toBe(true)
    const reset = savedAfterReset(released)
    expect(reset.saved).toBeNull()
    expect(reset.display).toBe('#1')
    expect(saveStillCurrent(reset, gen), '重設前排定的寫檔不再寫').toBe(false)
    // 重設之後再拖、再放開：新的寫檔照常
    const again = savedAfterRelease(reset, { corner: 'br' })
    expect(saveStillCurrent(again, again.resetGen)).toBe(true)
    // 重設後同一台螢幕重讀（檔案已刪，saved: null）：維持清掉的狀態
    expect(savedAfterRead(reset, '#1', null).saved).toBeNull()
  })

  it('清掉之後重新定位就是預設位置（主螢幕左側、選單列下方）', () => {
    const stage = makeStage(1512, 982)
    const len = 180
    const moved = restoreCenter(stage, { edge: 'right', head: 0.4 }, len, 37)
    const reset = restoreCenter(stage, savedAfterReset({ display: '#1', saved: { edge: 'right', head: 0.4 }, resetGen: 0 }).saved, len, 37)
    expect(reset).toBe(defaultCenter(stage, len, 37))
    expect(reset).not.toBe(moved)
  })
})
