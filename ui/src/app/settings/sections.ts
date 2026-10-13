// 設定頁的區段（BDD settings；DESIGN「Settings layout」）。
import type { IconKey } from '../status'

export type SettingsSectionId = 'general' | 'account' | 'mobile' | 'sleep' | 'force' | 'expiry' | 'dnd' | 'notch' | 'agents' | 'data' | 'about'

export const SETTINGS_SECTIONS: { id: SettingsSectionId; label: string; icon: IconKey; description: string }[] = [
  { id: 'general', label: '一般', icon: 'settings', description: '只套用在這台 Mac 的主視窗。' },
  { id: 'account', label: '帳號', icon: 'user', description: '登入 Kunsu 帳號，把這台 Mac 和你的手機串起來。內容只留在這台 Mac。' },
  { id: 'mobile', label: '手機', icon: 'smartphone', description: '配對你的 iPhone，在手機上回答；選中轉 AI。' },
  { id: 'sleep', label: '睡眠時間', icon: 'moon', description: '睡覺時不被打擾，題目一題都不會丟。' },
  { id: 'force', label: '強制詢問', icon: 'timer', description: '沒回答的題多久問你一次。' },
  { id: 'expiry', label: '到期與配額', icon: 'gauge', description: '題目能等多久、每個來源一天能送幾題、略過太多時怎麼辦。' },
  { id: 'dnd', label: '勿擾', icon: 'bell-off', description: '臨時暫停所有提醒，最長 24 小時。' },
  { id: 'notch', label: '瀏海', icon: 'map-pin', description: '貼在螢幕邊緣的黑色待決提醒。' },
  { id: 'agents', label: 'Agent 連線', icon: 'plug', description: '和 kunsu doctor 同一份檢查。' },
  { id: 'data', label: '資料與帳本', icon: 'database', description: '答案存在哪裡、帳本有沒有寫進去；每 5 秒更新。' },
  { id: 'about', label: '關於', icon: 'info', description: '版本與本機服務。' },
]
