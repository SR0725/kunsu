// Mock adapter：pnpm dev 免 Tauri 全功能開發。
// 假題取 2026-07-31 真實案例風格：TW-3 暫停黃燈動錢題、Kongko 批次審圖題、
// 紅燈多選項策略題、T1 素材過審、T2 董事會題、一張已答收據。
// 行為盡量鏡射 server 語義（15 分鐘循環、紅線升紅、發佈≠啟用等），
// 但真相永遠在 kunsu-server——本檔只為 UI 開發。

import type { BatchItem, Channel, Question } from './types'
import { isTerminal } from './types'
import type { EditOptionResult, ExecEvent, KunsuIpc, OverlayTransition, Receipt } from './ipc'
import { checkRedline, parseMoneyChange, recalcLine } from './redline'
import type { DndStatus } from './dnd'
import { INACTIVE_DND } from './dnd'
import { makeSettingsMock, scenarioFromSearch } from './app/settings/settingsMock'

// ---------------------------------------------------------------- 假資料

const TODAY = '2026-07-31'
const now = () => new Date().toISOString()
const hoursFromNow = (h: number) => new Date(Date.now() + h * 3_600_000).toISOString()

const baseSource = {
  agent: 'claude' as const,
  cwd: '/Users/you/notes',
  routine_id: 'mkt-daily-marketing-os',
}

const MOCK_CANVAS = [
  "import { LineChart, Line, XAxis, YAxis, CartesianGrid, Tooltip } from 'recharts'",
  "import { Grid, Stat, Panel, ChartFrame, chart, series } from 'kunsu'",
  '',
  'const days = ["10/02", "10/03", "10/04", "10/05", "10/06", "10/07", "10/08"]',
  'const roas = { A: [2.6, 2.4, 2.3, 2.1, 1.9, 1.7, 1.6], B: [1.9, 2.1, 2.2, 2.3, 2.4, 2.4, 2.4], C: [1.4, 1.3, 1.2, 1.2, 1.1, 1.1, 1.1] }',
  'const data = days.map((d, i) => ({ day: d, A: roas.A[i], B: roas.B[i], C: roas.C[i] }))',
  '',
  'export default function Canvas() {',
  '  return (',
  '    <div className="flex flex-col gap-3">',
  '      <Grid min={140}>',
  '        <Stat label="素材 B ROAS" value="2.4" delta={0.26} spark={roas.B} />',
  '        <Stat label="素材 A ROAS" value="1.6" delta={-0.38} spark={roas.A} />',
  '        <Stat label="B 頻次" value="1.4" good="down" delta={-0.05} note="低於 2 還有空間" />',
  '      </Grid>',
  '      <Panel title="7 天 ROAS" note="Meta API，10/02–10/08">',
  '        <ChartFrame height={200}>',
  '          <LineChart data={data} margin={chart.margin}>',
  '            <CartesianGrid {...chart.grid} />',
  '            <XAxis dataKey="day" {...chart.axis} />',
  '            <YAxis {...chart.yAxis} domain={[0, 3]} />',
  '            <Tooltip {...chart.tooltip} />',
  '            {["A", "B", "C"].map((k, i) => <Line key={k} dataKey={k} name={"素材 " + k} stroke={series[i]} {...chart.line} />)}',
  '          </LineChart>',
  '        </ChartFrame>',
  '      </Panel>',
  '    </div>',
  '  )',
  '}',
].join('\n')

const MOCK_THUMB =
  'data:image/svg+xml;base64,' +
  btoa(
    '<svg xmlns="http://www.w3.org/2000/svg" width="480" height="270"><defs><linearGradient id="g" x1="0" x2="1"><stop offset="0" stop-color="#1f3b63"/><stop offset="1" stop-color="#3987e5"/></linearGradient></defs><rect width="480" height="270" fill="url(#g)"/><text x="32" y="150" font-size="36" fill="#fff" font-family="sans-serif">Creative B</text></svg>',
  )

const MOCK_EVIDENCE: Record<string, { canvas?: { kind: 'tsx' | 'jsx' | 'html'; source: string }; assets: Record<string, string> }> = {
  'pa-creative-b-scale-1008': { canvas: { kind: 'tsx', source: MOCK_CANVAS }, assets: { './assets/b.svg': MOCK_THUMB } },
}

/** 題組示範（PRD §25；資料取自 docs/prototypes/batch-review.html 的「回信 10 封」）。 */
const MAIL_ITEMS: [string, string, string, string][] = [
  ['陳同學', '課程影片第 5 章打不開', '老師好，我在 10/6 買了 Demo 課，今天看到第 5 章「SwiftUI 動畫」時影片一直轉圈圈，換了 Chrome 和 Safari 都一樣，手機也不行。想請問是不是我的帳號有問題？訂單編號 #A1029。', '陳同學你好，謝謝回報！第 5 章的影片昨晚更新後有部分學員打不開，我們已經修好了，請重新整理頁面再試一次。如果還是不行，回信告訴我你用的裝置，我直接幫你處理。'],
  ['某品牌窗口（品牌合作）', '想邀請合作開箱影片', '您好，我們是某咖啡品牌的行銷，看到您在 Threads 分享工作日常，想邀請您開箱我們的冷萃新品，提供產品與稿酬，形式可討論。', '您好，謝謝邀請！目前我的內容主軸是 AI 與開發，咖啡開箱可能跟受眾比較不貼。如果之後有跟「工作效率」相關的合作，很歡迎再找我。'],
  ['王先生', '需要開立公司發票', '你好，我們公司幫 3 位同仁報名了 Sample Pro 課，需要開立三聯式發票，統編 1234XXXX，抬頭為某某科技有限公司。', '王先生你好，收到！三聯式發票會在 3 個工作天內寄到報名時的信箱，抬頭與統編我會照你提供的資料開立。有其他需要再跟我說。'],
  ['Podcast 製作人', '邀請上節目聊一人公司', '嗨，我們的節目「某某創業聊」在聊獨立工作者，想邀你來錄一集聊一人公司怎麼用 AI，錄音約 60 分鐘，可以遠端。', '你好，謝謝邀請，這個題目我很有興趣！可以的話我們約下週二或週四下午錄，60 分鐘遠端沒問題。可以先給我大綱嗎？'],
  ['張同學', '可以退費嗎？', '老師好，我上週買了 Sample Pro 課，但最近工作太忙可能沒時間看，請問還可以退費嗎？我只看了第一章。', '張同學你好，課程購買 7 天內、觀看進度在 20% 以內都可以全額退費，你符合條件。我這邊先幫你處理，款項約 5–7 個工作天退回原付款方式。'],
  ['合作夥伴 Kevin', '上次的聯名企劃進度？', '上次聊的聯名直播企劃，想確認你那邊 11 月的時程，我們這邊希望 11/15 前定案。', 'Kevin 你好，抱歉晚回！11 月我可以，11/15 前定案沒問題。這週五前我把我這邊的大綱寄給你。'],
  ['某大學 系學會', '邀請演講：AI 與職涯', '您好，我們是某大學資工系學會，12 月想辦一場「AI 與職涯」講座，想邀請您分享約 90 分鐘，有車馬費。', '你好，謝謝邀請！12 月上旬我時間比較滿，中下旬可以安排，90 分鐘沒問題。可以先告訴我場地、人數和車馬費金額嗎？'],
  ['李同學', '作業的 API key 要放哪？', '老師好，第 3 章作業要串 API，請問 key 要放在哪裡比較安全？直接寫在程式碼裡可以嗎？', '李同學好問題！不要寫在程式碼裡，放在 .env 檔並加進 .gitignore，部署時改用平台的環境變數設定。第 3 章的附件有範例，可以照著做。'],
  ['某軟體廠商', '升級企業方案優惠', '親愛的客戶，您的團隊方案將於 10/31 到期，現在升級企業方案享 8 折優惠，名額有限。', ''], // AI 建議不處理：空草稿，不准佔位文字（2026-10-09 作者）
  ['趙小姐（舊學員）', '想問進階課什麼時候開', '老師，上完 Demo 課很有收穫，請問之後會開進階課嗎？想學 SwiftData 和上架流程。', '趙小姐你好，謝謝支持！進階課預計明年第一季開，會包含 SwiftData 和上架流程。開放報名時我會先寄信通知舊學員，舊學員有早鳥價。'],
]

const MOCK_BATCH_ITEMS: BatchItem[] = MAIL_ITEMS.map(([title, subtitle, body, draft], i) => ({
  id: `i${i + 1}`,
  title,
  subtitle: `${['課程','Facebook','Instagram','Threads','LINE'][i % 5]}｜${subtitle}`,
  context_md: body,
  proposal: draft,
  editable: true,
  default: i === 8 ? 'skip' : 'approve',
  ...(i === 8 ? { default_reason: '促銷信；方案到期前我會另外提醒你要不要續約' } : {}),
  content_hash: `sha256:mock-mail-i${i + 1}`,
}))

function seedQuestions(): Question[] {
  const questions: Question[] = [
    // 排程督導（PRD §21）：每天寫日記（昨天已回應＋AI 回饋、前天沒回應）、每週目標檢討（Codex）
    {
      id: 'rt-diary-1007',
      title: '每日日記 check-in',
      question_line: '今天最重要的三件事？',
      body_md: '',
      kind: 'text',
      tier: 't1',
      requested_tier: 't1',
      evidence: [],
      options: [],
      allow_free_text: true,
      default_action: { on_expire: 'noop_log' },
      wake_on_answer: { mode: 'continue' },
      source: { ...baseSource, routine_id: 'diary', routine_name: '每天寫日記', trigger: 'scheduled', routine_kind: 'claude_scheduled_task', expected_every_hours: 24, host: 'claude_desktop', conversation_id: 'mock-diary', host_session_id: 'local_mock-diary' },
      state: 'answered',
      snooze_count: 0,
      forced_today_used: false,
      first_proposed: TODAY,
      last_seen: hoursFromNow(-26),
      created_at: hoursFromNow(-26),
      next_transition_at: hoursFromNow(-26),
      answer: {
        answer_id: 'mock-diary-a',
        free_text: '1. 把 Kunsu 的瀏海做完\n2. 回覆投資人\n3. 22:30 前睡\n參考：https://example.com/a-very-long-link-without-any-spaces-' + 'x'.repeat(160),
        channel: 'ui',
        answered_at: hoursFromNow(-25.5),
        delivery: { state: 'resumed', updated_at: hoursFromNow(-25.4), conversation_link: 'claude://code/continue?session=local_mock-diary' },
        report: { status: 'done', summary_md: '已寫入 `journal/2026-10-07.md`。**建議**：明天把「回覆投資人」放第一件，它已經拖兩天。', reported_at: hoursFromNow(-25.3) },
      },
    },
    {
      id: 'rt-diary-1006',
      title: '每日日記 check-in',
      question_line: '今天最重要的三件事？',
      body_md: '',
      kind: 'text',
      tier: 't1',
      requested_tier: 't1',
      evidence: [],
      options: [],
      allow_free_text: true,
      default_action: { on_expire: 'noop_log' },
      wake_on_answer: { mode: 'continue' },
      source: { ...baseSource, routine_id: 'diary', routine_name: '每天寫日記', trigger: 'scheduled', expected_every_hours: 24 },
      state: 'expired',
      snooze_count: 0,
      forced_today_used: false,
      first_proposed: TODAY,
      last_seen: hoursFromNow(-50),
      created_at: hoursFromNow(-50),
      next_transition_at: hoursFromNow(-50),
    },
    {
      id: 'rt-weekly-0928',
      title: '每週目標檢討',
      question_line: '這週三個目標進度如何？',
      body_md: '',
      kind: 'text',
      tier: 't1',
      requested_tier: 't1',
      evidence: [],
      options: [],
      allow_free_text: true,
      default_action: { on_expire: 'noop_log' },
      wake_on_answer: { mode: 'continue' },
      source: { agent: 'codex', cwd: '/w', routine_id: 'weekly', routine_name: '每週目標檢討', trigger: 'scheduled', routine_kind: 'codex_cron', expected_every_hours: 168 },
      state: 'cancelled',
      snooze_count: 0,
      forced_today_used: false,
      first_proposed: TODAY,
      last_seen: hoursFromNow(-400),
      created_at: hoursFromNow(-400),
      next_transition_at: hoursFromNow(-400),
    },
    // 0) 草稿註冊題＋證據畫布（PRD §24，Claude Artifact 模式）
    // 題組（PRD §25）：今天 10 封信的回覆草稿；第 9 封 AI 建議不處理。
    {
      id: 'mail-reply-1009',
      title: '今天 10 封信的回覆草稿',
      question_line: '今天 10 封信的回覆草稿',
      why_md: '- 09:00 排程收信：今天 14 封新信，4 封是廣告已封存\n- 09:08 10 封需要你回，已依你過去的回信語氣寫好草稿\n- 現在 要你看過再寄；你確認的文字會一字不改寄出',
      body_md: '',
      kind: 'batch',
      tier: 't1',
      requested_tier: 't1',
      evidence: [],
      options: [],
      allow_free_text: true,
      default_action: { on_expire: 'noop_log' },
      wake_on_answer: { mode: 'continue' },
      source: { ...baseSource, routine_id: 'mail-daily', routine_name: '每日收信', trigger: 'scheduled', host: 'claude_desktop', conversation_id: 'mock-mail', host_session_id: 'local_mock-mail' },
      state: 'pending',
      snooze_count: 0,
      forced_today_used: false,
      first_proposed: TODAY,
      last_seen: now(),
      created_at: now(),
      next_transition_at: now(),
      items: MOCK_BATCH_ITEMS,
    },
    {
      id: 'pa-creative-b-scale-1008',
      title: '素材 B 三天 ROAS 穩定在 2.4，要不要把預算往它移',
      question_line: '素材 B 要不要加碼？',
      why_md: [
        '- 09:00 每日巡檢：素材 A 的 ROAS 掉到 1.6，連三天下滑',
        '- 09:12 比較同組三支素材，B 三天都在 2.3 以上',
        '- 09:20 B 的頻次才 1.4，還有空間',
        '- 現在 要你決定預算要不要往 B 移',
      ].join('\n'),
      body_md: '| 素材 | 花費 | ROAS | 頻次 |\n|---|---:|---:|---:|\n| A | NT$4,820 | 1.6 | 2.9 |\n| B | NT$3,110 | 2.4 | 1.4 |\n| C | NT$1,250 | 1.1 | 1.2 |\n\n![素材 B](./assets/b.svg)',
      kind: 'choice',
      tier: 't1',
      requested_tier: 't1',
      product: 'course-a',
      evidence: [],
      options: [
        { id: 'shift', label: 'A 的預算移 30% 給 B', detail: '建議：B 頻次低、ROAS 穩（見[巡檢表](https://sheets.example.com/daily-check)）', recommended: true },
        { id: 'add', label: 'B 另外加 NT$500／天', detail: '總預算增加' },
        { id: 'wait', label: '再觀察兩天', detail: '不動任何設定' },
      ],
      allow_free_text: true,
      default_action: { on_expire: 'noop_log' },
      wake_on_answer: { mode: 'continue' },
      source: { ...baseSource, host: 'claude_desktop', conversation_id: 'mock-conv-1', host_session_id: 'local_mock-1' },
      state: 'forced',
      snooze_count: 0,
      forced_today_used: false,
      first_proposed: TODAY,
      last_seen: now(),
      created_at: now(),
      next_transition_at: now(),
      draft: { revision: 2, content_hash: 'sha256:mock', canvas: 'tsx', registered_at: now() },
    },
    // 1) T0 黃燈動錢：TW-3 暫停（今晨 mkt-daily 升黃燈的零意圖吸盤）
    {
      id: 'pa-ios-tw3-pause-0731',
      title: 'iOS TW-3 零意圖吸盤條件滿足，建議暫停止血',
      question_line: '暫停 TW-3 吸盤廣告？',
      body_md: 'TW-3 連三日零成交且 LP 停留中位數 9 秒，符合零意圖吸盤判準。',
      kind: 'choice',
      tier: 't0',
      requested_tier: 't0',
      product: 'course-a',
      gate: 'yellow',
      evidence: [
        { text: '連 3 天成交 1 → 0 → 0', source: 'Stripe，07-28~30' },
        { text: '3 日花費 NT$2,742｜ad-attributed ROAS 0.0', source: 'Meta API，07-28~30' },
        { text: 'CTR 2.1% 但 LP 停留中位數 9 秒（零意圖吸盤）', source: 'PostHog 287005，07-24~30' },
      ],
      params_line: 'entity 120000000000000001｜ACTIVE → PAUSED｜可逆',
      options: [
        {
          id: 'pause',
          label: '暫停 TW-3',
          detail: '建議：止血日省 NT$914；明日復盤驗證是否誤殺',
          recommended: true,
        },
        {
          id: 'cut20',
          label: '降日預算 300 → 240（-20%）',
          detail: '保留學習訊號，週五再判生死',
        },
        { id: 'keep', label: '維持現狀', detail: '安全預設：不動任何錢，明日再看一天數據' },
      ],
      allow_free_text: true,
      money: {
        is_money_op: true,
        amount_twd: 914,
        entity_id: '120000000000000001',
        executor_prepare_cmd:
          'python3 executor.py prepare --product course-a --entity 120000000000000001 --action pause',
      },
      prediction_ref: 'projects/meta-ads/predictions/2026-07-31-ios-tw3-pause.md',
      default_action: { on_expire: 'escalate_discord' },
      deadline_at: hoursFromNow(4),
      wake_on_answer: { mode: 'fresh', playbook: 'playbooks/approved-money-op.md' },
      source: baseSource,
      state: 'forced',
      snooze_count: 0,
      forced_today_used: true,
      first_proposed: '2026-07-29',
      last_seen: now(),
      created_at: now(),
      next_transition_at: now(),
    },
    // 2) T0 紅燈多選項策略：course-b 餵餌 06:26 手動重啟在燒錢
    {
      id: 'pa-course-b-bait-restart-0731',
      title: 'course-b 餵餌 campaign 06:26 被手動重啟，今日已燒 NT$1,882 無歸因成交',
      question_line: 'Sample Pro 餵餌續燒或停？',
      body_md: '',
      kind: 'choice',
      tier: 't0',
      requested_tier: 't0',
      product: 'course-b',
      gate: 'red',
      evidence: [
        { text: '今日花費 NT$1,882｜歸因成交 0', source: 'Meta API + Stripe，07-31' },
        { text: '重啟非排程觸發：06:26 手動操作', source: 'Meta 變更紀錄，07-31' },
        { text: '07-30 另有 NT$5,940 未分類支出待認領', source: '帳務 ledger，07-30' },
      ],
      params_line: 'campaign 120000000000000002｜日預算 NT$1,900｜pause 參數已備',
      options: [
        {
          id: 'pause-now',
          label: '立即暫停餵餌',
          detail: '建議：紅燈期間先停損，查清 06:26 誰動的再議',
          recommended: true,
        },
        { id: 'cap-500', label: '設日預算上限 1,900 → 500', detail: '降檔觀察 3 天' },
        { id: 'ride-weekend', label: '燒到週日再結算', detail: '需要你拍板承擔 ~NT$5,700 風險' },
        { id: 'keep', label: '維持現狀', detail: '安全預設：不動，明晨日報再看' },
      ],
      allow_free_text: true,
      money: {
        is_money_op: true,
        amount_twd: 1900,
        entity_id: '120000000000000002',
        executor_prepare_cmd:
          'python3 executor.py prepare --product course-b --entity 120000000000000002 --action pause',
      },
      prediction_ref: 'projects/meta-ads/predictions/2026-07-31-course-b-bait-stop.md',
      default_action: { on_expire: 'escalate_discord' },
      deadline_at: hoursFromNow(9),
      wake_on_answer: { mode: 'fresh', playbook: 'playbooks/approved-money-op.md' },
      source: baseSource,
      state: 'pending',
      snooze_count: 0,
      forced_today_used: false,
      first_proposed: TODAY,
      last_seen: now(),
      created_at: now(),
      next_transition_at: now(),
    },
    // 3) T1 Kongko 批次審圖
    {
      id: 'pa-nova-kongko-batch-0731',
      title: 'Nova 新切角「拆夥焦慮」6 稿完成，等挑 2 張進發佈',
      question_line: 'Nova 六稿挑二，去挑圖？',
      body_md: '',
      kind: 'kongko_review',
      tier: 't1',
      requested_tier: 't1',
      product: 'nova',
      evidence: [
        { text: '切角「拆夥焦慮」6 稿產線完成待審', source: 'Kongko atelier，07-31' },
        { text: '現行主力素材已跑 19 天｜頻率 3.8（老化 6× 警報）', source: 'Meta API，07-12~30' },
      ],
      options: [
        { id: 'reviewed', label: '已在 Kongko 挑好並發佈', detail: '發佈=同意素材上架；啟用另出獨立 T0 題' },
        { id: 'kill-batch', label: '本批全殺，重下切角提示', detail: '產線重跑約 40 分鐘' },
        { id: 'later', label: '先擱置', detail: '安全預設：批次保留在畫布，明日再審' },
      ],
      allow_free_text: true,
      kongko_ref: 'batch_nova_0731_a',
      deep_link: 'http://localhost:3200/atelier/batch_nova_0731_a',
      default_action: { on_expire: 'noop_log' },
      wake_on_answer: { mode: 'fresh', playbook: 'playbooks/kongko-published.md' },
      source: baseSource,
      state: 'pending',
      snooze_count: 0,
      forced_today_used: false,
      first_proposed: TODAY,
      last_seen: now(),
      created_at: now(),
      next_transition_at: now(),
    },
    // 4) T1 素材過審 confirm（demo：答後執行流失敗轉紅）
    {
      id: 'pa-acme-creative-approved-0731',
      title: 'Acme 3 支新素材 14:20 全數過審，等排上線（維持 PAUSED 建檔）',
      question_line: 'Acme 新素材排上線？',
      body_md: '',
      kind: 'confirm',
      tier: 't1',
      requested_tier: 't1',
      product: 'acme',
      evidence: [
        { text: '3 支新素材全數過審（14:20）', source: 'Meta API，07-31' },
        { text: '現役素材老化 6× 警報中，等新素材接棒', source: 'mkt-daily 日報，07-30' },
      ],
      options: [],
      allow_free_text: true,
      default_action: { on_expire: 'noop_log' },
      wake_on_answer: { mode: 'fresh', playbook: 'playbooks/approved-money-op.md' },
      source: baseSource,
      state: 'pending',
      snooze_count: 0,
      forced_today_used: false,
      first_proposed: TODAY,
      last_seen: now(),
      created_at: now(),
      next_transition_at: now(),
    },
    // 5) T2 董事會題：nova 賭注結算後的方向
    {
      id: 'pa-nova-bets-restructure',
      title: 'Nova 4 檔賭注結算：3 檔 never_executed、1 檔歸因仲裁中——下一步怎麼排',
      question_line: 'Nova 賭注重排或收攤？',
      body_md: '',
      kind: 'choice',
      tier: 't2',
      requested_tier: 't2',
      product: 'nova',
      gate: 'red',
      evidence: [
        { text: '4 檔賭注 3 檔 never_executed', source: 'predictions 結算，07-31' },
        { text: '1 檔 attribution 仲裁中（Stripe vs Meta 歸因不合）', source: 'mkt-daily 董事會，07-31' },
        { text: '本月 nova 廣告支出 NT$18,400', source: '帳務 ledger，07 月' },
      ],
      options: [
        { id: 'wind-down', label: '收攤：降到維持檔', detail: '月支出壓到 NT$6,000 以下，主力讓給 course-b' },
        {
          id: 'rebet',
          label: '重排：賭注縮 2 檔、預算減半',
          detail: '建議：保留學習但強制每檔配 prediction 與到期日',
          recommended: true,
        },
        { id: 'hold', label: '維持現狀到 8/15', detail: '安全預設：SF 期間不動，回台再拍板' },
      ],
      allow_free_text: true,
      default_action: { on_expire: 'noop_log' },
      wake_on_answer: { mode: 'none' },
      source: baseSource,
      state: 'pending',
      snooze_count: 0,
      forced_today_used: false,
      first_proposed: '2026-07-24',
      last_seen: now(),
      created_at: now(),
      next_transition_at: now(),
    },
    // 6) T2 積壓遷移長題（重現 2026-07-31 首用排版 bug：超長 body_md、
    //    超長身世籤、「原提案動作」長文證據行——修法見 CONTRACT §9 分層捲動）
    {
      id: 'pa-acme-ad1j-leads-migration',
      title:
        'Acme 補 Ad-1J 進新 LEADS campaign 前先修事件追蹤——補洞順序與預算重啟條件（積壓遷移董事會分診）',
      question_line: 'Acme 補 Ad-1J 進新 LEADS？',
      body_md: [
        '**積壓遷移（董事會分診）** — 原黃/紅燈提案 `pa-acme-ad1j-leads`，category=manual_action，首次提案 2026-07-18。',
        '',
        '**為什麼**：Ad-1J 是 Acme 歷史 CTR 最高素材（3.4%），但 07-12 起 LEADS 事件斷代：PostHog 339138 收不到 lead 事件、Meta CAPI 只剩 PageView。現在把 Ad-1J 補進新 LEADS campaign 等於盲投——花了錢也無法歸因，違反「廣告決策只認 ad-attributed ROAS」鐵律。',
        '',
        '**補洞順序**：(1) 修 lead 事件上報（sales page 表單 submit → PostHog + CAPI 雙軌）；(2) 驗證 48h 事件量對得上表單後台；(3) 才建新 LEADS campaign 綁 utm_content，Ad-1J 以 PAUSED 建檔；(4) 預算重啟另出正式 T0 動錢題（params_line + prediction_ref 齊備才准一鍵）。',
        '',
        '**原提案動作**：同意後我先修追蹤、備妥素材複製參數清單，全程不動錢；啟用另出獨立題。',
        '',
        'verify_or_expires：2026-08-14',
        '',
        '> ⚠️ 本題來自一次性積壓大掃除 pack：分診用，不是動錢一鍵。選「照提案執行」後，動錢項仍須由當日流程重建帶 params_line + prediction_ref 的正式題、走 executor 護欄。',
      ].join('\n'),
      kind: 'choice',
      tier: 't2',
      requested_tier: 't2',
      product: 'acme',
      evidence: [
        {
          text: '原提案動作：同意後我把 Ad-1J 素材複製進新 LEADS campaign、綁 utm_content、維持 PAUSED 建檔，等追蹤修復驗證通過才排啟用，全程不動預算。',
          source:
            'pending-approvals.json（M60 積壓遷移，原提案 pa-acme-ad1j-leads，category=manual_action，first_proposed 2026-07-18，經 kunsu-migrate 2026-07-30 匯入董事會 pack）',
        },
        {
          text: 'LEADS 事件斷代：07-12 起 PostHog 339138 收不到 lead 事件，Meta CAPI 只剩 PageView',
          source: 'PostHog 339138 + Meta 事件管理工具，07-12~30',
        },
      ],
      options: [
        { id: 'approve', label: '照提案執行', detail: '先修追蹤再建檔（動錢項另建正式題）' },
        { id: 'reject', label: '否決封存', detail: 'pending-approvals 改 rejected，不再重提' },
        { id: 'discuss', label: '轉討論', detail: '值得談但不是一鍵能決——開互動 session' },
      ],
      allow_free_text: true,
      default_action: { on_expire: 'noop_log' },
      wake_on_answer: { mode: 'none' },
      source: { ...baseSource, routine_id: 'backlog-migration-2026-07-30' },
      state: 'pending',
      snooze_count: 0,
      forced_today_used: false,
      first_proposed: '2026-07-18',
      last_seen: now(),
      created_at: now(),
      next_transition_at: now(),
    },
    // 7) 已答收據（昨日 Lab 加碼 20%，已執行完畢）
    {
      id: 'pa-ios-lab-budget-3600',
      title: 'iOS Lab 廣告日預算 3,000 → 3,600（+20%）已批准並執行',
      question_line: 'Lab 日預算加碼 20%？',
      body_md: '',
      kind: 'choice',
      tier: 't0',
      requested_tier: 't0',
      product: 'course-a',
      gate: 'yellow',
      evidence: [
        { text: '7 日 ad-attributed ROAS 3.4｜連 5 天達標', source: 'Meta API + Stripe，07-23~29' },
        { text: '學習期已出、CPA 穩定 NT$412', source: 'Meta API，07-27~29' },
      ],
      params_line: 'adset 120000000000000003｜日預算 3,000 → 3,600（+20%）',
      options: [
        { id: 'raise', label: '加碼 20%', recommended: true },
        { id: 'keep', label: '維持現狀' },
      ],
      allow_free_text: true,
      money: {
        is_money_op: true,
        amount_twd: 600,
        entity_id: '120000000000000003',
      },
      decision_ref: 'projects/meta-ads/decisions/2026-07-31-ios-lab-budget-3600.md',
      prediction_ref: 'projects/meta-ads/predictions/2026-07-31-ios-lab-budget-3600.md',
      default_action: { on_expire: 'escalate_discord' },
      wake_on_answer: { mode: 'fresh', playbook: 'playbooks/approved-money-op.md' },
      source: baseSource,
      state: 'answered',
      snooze_count: 0,
      forced_today_used: true,
      first_proposed: '2026-07-30',
      last_seen: now(),
      created_at: '2026-07-31T01:15:00Z',
      next_transition_at: now(),
      answer: {
        answer_id: 'a3f1c2d4-8b7e-4f10-9c3a-5d2e6f7a8b90',
        choice_id: 'raise',
        channel: 'ui',
        answered_at: '2026-07-31T01:41:00Z',
        signature: 'hmac:9f2c11ab…',
        acked_at: '2026-07-31T01:41:20Z',
        executed_at: '2026-07-31T01:42:05Z',
        execution_result:
          'COMMIT_OK: ✅ adset 120000000000000003 日預算 3,000 → 3,600 已生效｜decision + prediction 已落盤（verify 08-07）｜Discord 已記',
      },
      // 與 shell 一致（CONTRACT §7、§9）：已作答錢題附伺服器算的 money_state。
      money_state: 'settled',
    },
    // 8) 結果不明（8/6 那類）：executor 已用掉授權、沒有回報結果，交付進死信——回看時有「錢已經動了／錢沒有動」（AC-MA-18／20）。
    {
      id: 'pa-ios-cbo-scale-6000',
      title: 'iOS CBO 日預算 5,000 → 6,000（+20%）',
      question_line: 'CBO 日預算加碼 20%？',
      body_md: '',
      kind: 'choice',
      tier: 't0',
      requested_tier: 't0',
      product: 'course-a',
      gate: 'yellow',
      evidence: [{ text: '7 日 ROAS 3.1｜CPA 穩定', source: 'Meta API，08-04~06' }],
      params_line: 'campaign 120000000000000004｜日預算 5,000 → 6,000（+20%）',
      options: [
        { id: 'raise', label: '加碼 20%', recommended: true },
        { id: 'keep', label: '維持現狀' },
      ],
      allow_free_text: true,
      money: {
        is_money_op: true,
        amount_twd: 1000,
        entity_id: '120000000000000004',
        executor_prepare_cmd: 'python3 executor.py prepare-budget --entity 120000000000000004 --new-budget 6000 --product course-a',
      },
      prediction_ref: 'projects/meta-ads/predictions/2026-08-06-ios-cbo-6000.md',
      default_action: { on_expire: 'escalate_discord' },
      wake_on_answer: { mode: 'fresh', playbook: 'playbooks/approved-money-op.md' },
      source: baseSource,
      state: 'answered',
      snooze_count: 0,
      forced_today_used: false,
      first_proposed: '2026-08-06',
      last_seen: hoursFromNow(-30),
      created_at: hoursFromNow(-31),
      next_transition_at: hoursFromNow(-30),
      answer: {
        answer_id: 'b7c1d2e3-0806-4cbo-9000-000000006000',
        choice_id: 'raise',
        channel: 'ui',
        answered_at: hoursFromNow(-30),
        executed_at: hoursFromNow(-29),
        execution_result: 'EXEC_FAILED: 死信（重試 3 次仍失敗）：spawn claude 失敗',
      },
      execution_blocked: '死信（重試 3 次仍失敗）：spawn claude 失敗',
      money_state: 'unknown',
    },
  ]
  return questions.map((question) => {
    // 只替進行中的題重設週期；已結案（含過期、取消）保留原本的時間，督導頁才看得到真實歷史。
    if (isTerminal(question.state) || question.internal) return question
    const started = question.state === 'forced' ? Date.now() - 15 * 60_000 : Date.now()
    return {
      ...question,
      created_at: new Date(started).toISOString(),
      cycle_started_at: new Date(started).toISOString(),
      warn_at: new Date(started + 12 * 60_000).toISOString(),
      force_at: new Date(started + 15 * 60_000).toISOString(),
      cycle_number: 1,
      next_transition_at: new Date(started + 12 * 60_000).toISOString(),
    }
  })
}

// ---------------------------------------------------------------- adapter

type ExecCb = (e: ExecEvent) => void

export function makeMockIpc(): KunsuIpc {
  const questions = new URLSearchParams(window.location.search).has('empty') ? [] : seedQuestions()
  const execSubs = new Map<string, Set<ExecCb>>()
  const listSubs = new Set<() => void>()
  const overlaySubs = new Set<(transition: OverlayTransition) => void>()
  let overlayTransitionId = 0
  let dnd: DndStatus = { ...INACTIVE_DND }
  // 主視窗設定與狀態（CONTRACT §15）：網址參數 ?local=broken、?ledger=backlog|stuck、?integrations=missing 模擬異常。
  const settings = makeSettingsMock({ storage: localStorage, scenario: scenarioFromSearch(window.location.search) })

  const find = (id: string) => questions.find((q) => q.id === id)
  const notifyList = () => listSubs.forEach((cb) => cb())
  // 瀏海的 shell 事件（瀏覽器開發沒有 shell）：舞台重讀與「回到預設位置」。設定頁（主視窗分頁）按回到預設位置時，經
  // BroadcastChannel 通知瀏海分頁（`?view=capsule`），等同 shell 刪檔後發 `kunsu://edge_position_reset`。
  const edgeStageSubs = new Set<() => void>()
  const edgeResetSubs = new Set<() => void>()
  const edgeChannel = typeof BroadcastChannel === 'undefined' ? null : new BroadcastChannel('kunsu:mock:edge')
  edgeChannel?.addEventListener('message', (e: MessageEvent) => {
    if (e.data === 'position_reset') edgeResetSubs.forEach((cb) => cb())
  })
  const edgePositionReset = () => {
    edgeResetSubs.forEach((cb) => cb())
    edgeChannel?.postMessage('position_reset')
  }
  // 瀏覽器 QA：指定的題 dismissal 失敗（AC-OP-06 部分失敗後重試）；`__kunsuMock.failDismiss(id, false)` 解除。
  const failDismissFor = new Set<string>()
  const emitExec = (e: ExecEvent) => execSubs.get(e.question_id)?.forEach((cb) => cb(e))

  // 瀏覽器 QA：在 console 模擬其他通道結案、session 開著時才到期的強制題（AC-DS-16、AC-OP-07）。只在 mock。
  ;(window as unknown as { __kunsuMock?: unknown }).__kunsuMock = {
    ids: () => questions.map((q) => `${q.id}（${q.state}）`),
    get: (id: string) => structuredClone(find(id)),
    closeElsewhere(id: string, channel: Channel = 'discord') {
      const q = find(id)
      if (!q) return
      q.state = 'answered'
      q.answer = { answer_id: `mock-${id}-elsewhere`, choice_id: q.options[0]?.id, channel, answered_at: now() }
      notifyList()
    },
    force(id: string) {
      const q = find(id)
      if (!q) return
      q.state = 'forced'
      notifyList()
    },
    /**
     * 重排某題的週期（瀏覽器 QA：設定頁改了提前警告之後的新週期）：`forceInMs` 後到期、提前 `warnMs` 警告（週期 15 分鐘或
     * 更長）；已進入警告就是 warned。
     */
    retime(id: string, forceInMs: number, warnMs = 3 * 60_000) {
      const q = find(id)
      if (!q || isTerminal(q.state)) return
      const forceAt = Date.now() + forceInMs
      const started = forceAt - Math.max(15 * 60_000, warnMs + 60_000)
      q.created_at = new Date(started).toISOString()
      q.cycle_started_at = new Date(started).toISOString()
      q.warn_at = new Date(forceAt - warnMs).toISOString()
      q.force_at = new Date(forceAt).toISOString()
      q.next_transition_at = forceInMs > warnMs ? q.warn_at : q.force_at
      q.state = forceInMs > warnMs ? 'pending' : 'warned'
      notifyList()
    },
    failDismiss(id: string, on = true) {
      if (on) failDismissFor.add(id)
      else failDismissFor.delete(id)
    },
    /** 題組草稿重新註冊（AC-BR-09）：第 1、2 項換成新版 AI 草稿，第 3 項只有原文多了補充（草稿沒變、雜湊變了）。 */
    reregister(id: string) {
      const q = find(id)
      if (!q?.items) return
      const revision = (q.draft?.revision ?? 1) + 1
      q.items = q.items.map((item, i) =>
        i < 2
          ? { ...item, proposal: `${item.proposal}\n\n（AI 第 ${revision} 版：補上下週的時間）`, content_hash: `${item.content_hash}-r${revision}` }
          : i === 2
            ? { ...item, context_md: `${item.context_md}\n\n（補充：對方又寄了一封）`, content_hash: `${item.content_hash}-r${revision}` }
            : item,
      )
      q.draft = { revision, content_hash: `sha256:mock-draft-r${revision}`, registered_at: now() }
      notifyList()
    },
    settings,
    /** 瀏覽器 QA：模擬 shell 的 `kunsu://edge_stage_changed`（螢幕配置改變、睡眠喚醒）。 */
    edgeStageChanged() {
      edgeStageSubs.forEach((cb) => cb())
    },
    /** 瀏覽器 QA：模擬 shell 的 `kunsu://edge_position_reset`（不刪 mock 保存值；要刪用 `settings.edgeResetPosition()`）。 */
    edgePositionReset,
  }

  /** 模擬答後執行流串流（prepare → guard → API → 落盤）。 */
  function simulateContinuation(q: Question) {
    const link = q.source.host_session_id ? `claude://code/continue?session=${q.source.host_session_id}` : undefined
    const setDelivery = (state: 'pending' | 'resuming' | 'resumed') => {
      if (!q.answer) return
      q.answer.delivery = { state, updated_at: now(), ...(link ? { conversation_link: link } : {}) }
      if (state === 'resumed') {
        q.answer.acked_at = now()
        const decisions = q.answer.items
        const count = (action: string) => decisions?.filter((d) => d.action === action).length ?? 0
        q.answer.report = {
          status: 'done',
          summary_md: decisions
            ? `已送出 ${count('approve')} 封（一字不改）、不處理 ${count('skip')} 封${count('revise') ? `；${count('revise')} 封依你的指示重寫中，改好以 \`${q.id}-r2\` 第 2 輪送來` : ''}。`
            : '已把素材 A 的預算移 30% 給 B（NT$1,450／天），明早 09:00 巡檢會回報 ROAS。',
          reported_at: now(),
        }
      }
      notifyList()
    }
    setDelivery('pending')
    window.setTimeout(() => setDelivery('resuming'), 1500)
    window.setTimeout(() => setDelivery('resumed'), 5000)
  }

  /**
   * 瀏覽器 QA：錢題的錢鏈結果（localStorage `kunsu:mock:money-outcome` = `blocked`｜`unknown`，預設成功）。`unknown` 模擬 executor 驗證後
   * commit 失敗：先送 api fail，錢鏈列還在跑（交付）一段時間後才變成結果不明（F3）。
   */
  function mockMoneyOutcome(): 'ok' | 'blocked' | 'unknown' {
    try {
      const v = localStorage.getItem('kunsu:mock:money-outcome')
      return v === 'blocked' || v === 'unknown' ? v : 'ok'
    } catch {
      return 'ok'
    }
  }

  /** 錢鏈適用（黃燈、UI 作答、執行型選項）：shell 會附 money_state。 */
  const moneyChainApplies = (q: Question) =>
    q.money?.is_money_op === true &&
    q.gate !== 'red' &&
    q.answer?.channel === 'ui' &&
    q.options.some((o) => o.id === q.answer?.choice_id && o.recommended === true)

  function simulateExecution(q: Question, { forceOk = false }: { forceOk?: boolean } = {}) {
    const qid = q.id
    const money = moneyChainApplies(q)
    const outcome = forceOk || !money ? 'ok' : mockMoneyOutcome()
    const fail = qid === 'pa-acme-creative-approved-0731' || outcome !== 'ok' // demo：執行失敗轉紅
    if (money) q.money_state = 'running'
    notifyList()
    const verifyDate = '08-07'
    const receipt: Receipt = {
      summary:
        qid === 'pa-ios-tw3-pause-0731'
          ? '✅ 已暫停 TW-3（entity 120000000000000001）'
          : `✅ 已執行：${q.question_line}`,
      decision_ref: `projects/meta-ads/decisions/2026-07-31-${qid}.md`,
      prediction_ref: q.prediction_ref ?? `projects/meta-ads/predictions/2026-07-31-${qid}.md`,
      prediction_verify: verifyDate,
      discord_logged: true,
    }

    type Frame = { delay: number; ev: Omit<ExecEvent, 'question_id'> }
    const frames: Frame[] = [
      { delay: 150, ev: { step: 'prepare', status: 'start', line: `$ ${q.money?.executor_prepare_cmd ?? 'prepare（非錢類：組執行參數）'}` } },
      { delay: 650, ev: { step: 'prepare', status: 'ok', line: 'token=exq_7f31 新鮮簽發（TTL 15 分鐘）｜param_hash 已綁定' } },
      { delay: 950, ev: { step: 'guard', status: 'start', line: '護欄重驗：NT$2,000 紅線｜≤20% 偏離｜3 天冷卻｜帳戶漂移 GET' } },
    ]
    if (fail) {
      frames.push({
        delay: 1700,
        ev: { step: 'guard', status: 'ok', line: '護欄全綠 ✓' },
      })
      frames.push({
        delay: 2400,
        ev: {
          step: 'api',
          status: 'start',
          line: 'POST /act_0000000001/ads（3 支素材排程上線）',
        },
      })
      frames.push({
        delay: 3200,
        ev: {
          step: 'api',
          status: 'fail',
          line: 'Meta API 400：creative 關聯的 campaign 已被封存（error_subcode 1487172）',
        },
      })
    } else {
      frames.push({ delay: 1700, ev: { step: 'guard', status: 'ok', line: '護欄全綠 ✓' } })
      frames.push({
        delay: 2300,
        ev: {
          step: 'api',
          status: 'start',
          line: `POST /${q.money?.entity_id ?? 'entity'}｜status=PAUSED`,
        },
      })
      frames.push({ delay: 3000, ev: { step: 'api', status: 'ok', line: '200 OK｜Meta 端已生效' } })
      frames.push({
        delay: 3400,
        ev: { step: 'ledger', status: 'start', line: 'append logs/asks/2026-07-31.jsonl…' },
      })
      frames.push({
        delay: 4100,
        ev: {
          step: 'ledger',
          status: 'ok',
          line: 'ledger ✓｜decision + prediction 已寫｜Discord 已記',
          receipt,
        },
      })
    }

    for (const f of frames) {
      setTimeout(() => {
        emitExec({ question_id: qid, ...f.ev })
        if (f.ev.status === 'fail') {
          const target = find(qid)
          if (target && target.answer && outcome === 'unknown') {
            // executor 已用掉授權：錢鏈先記結果不明，同一列接著交付（money_state 仍是 running），之後才定案（F3）。
            window.setTimeout(() => {
              if (!target.answer) return
              target.answer.executed_at = now()
              target.answer.execution_result = `EXEC_UNKNOWN: executor 已用掉授權但 commit 沒有回報結果：${f.ev.line}`
              notifyList()
            }, 1200)
            window.setTimeout(() => {
              target.money_state = 'unknown'
              notifyList()
            }, 4500)
          } else if (target && target.answer) {
            // 執行失敗絕不靜默：回列表標 execution_blocked（AC-AN-08）；與 server 一致寫 EXEC_BLOCKED 開頭。
            target.execution_blocked = f.ev.line
            target.answer.execution_result = `EXEC_BLOCKED: ${f.ev.line}`
            if (money) target.money_state = 'reauthorizable'
            notifyList()
          }
        }
        if (f.ev.receipt) {
          const target = find(qid)
          if (target && target.answer) {
            target.answer.acked_at = now()
            target.answer.executed_at = now()
            target.answer.execution_result = money ? `COMMIT_OK: ${f.ev.receipt.summary}` : f.ev.receipt.summary
            if (money) target.money_state = 'settled'
            delete target.execution_blocked
            target.decision_ref = f.ev.receipt.decision_ref
            notifyList()
          }
        }
      }, f.delay)
    }
  }

  return {
    isMock: true,
    ...settings,

    async listQuestions() {
      return structuredClone(questions)
    },

    async getQuestion(id) {
      const q = find(id)
      return q ? structuredClone(q) : null
    },

    async edgeInfo() {
      let saved: unknown = null
      try {
        saved = JSON.parse(localStorage.getItem('kunsu:mock:edge') ?? 'null')
      } catch {
        saved = null
      }
      return { width: window.innerWidth, height: window.innerHeight, menubar: 37, display: 'mock', display_id: null, saved }
    },
    async edgeHit() {},
    async edgeAnchor() {},
    async edgePrivacy() {
      return false
    },
    async deferReasons() {
      return { reasons: [] }
    },
    subscribeEdgePrivacy() {
      return () => {}
    },
    subscribeEdgePointer(cb) {
      // 瀏覽器開發沒有原生層：用 DOM 模擬「游標在瀏海上」的通知與每秒重送。
      let last: { x: number; y: number } | null = null
      let inside: boolean | null = null
      const onMove = (e: PointerEvent) => {
        last = { x: e.clientX, y: e.clientY }
      }
      const id = window.setInterval(() => {
        const now = last !== null && document.elementFromPoint(last.x, last.y)?.closest('.edge-band') != null
        if (now || now !== inside) cb(now)
        inside = now
      }, 1000)
      window.addEventListener('pointermove', onMove)
      return () => {
        window.clearInterval(id)
        window.removeEventListener('pointermove', onMove)
      }
    },
    subscribeEdgeStage(cb) {
      edgeStageSubs.add(cb)
      return () => {
        edgeStageSubs.delete(cb)
      }
    },
    subscribeEdgePositionReset(cb) {
      edgeResetSubs.add(cb)
      return () => {
        edgeResetSubs.delete(cb)
      }
    },
    /** 設定頁「回到預設位置」：刪 mock 保存值後通知瀏海（shell 刪檔後發 `kunsu://edge_position_reset`）。 */
    async edgeResetPosition() {
      await settings.edgeResetPosition()
      edgePositionReset()
    },
    async canvasFailure(questionId, revision, reason) {
      console.warn('畫布失敗（mock 診斷）', questionId, revision, reason)
    },
    async canvasActive() {
      return true
    },
    async canvasInactive() {},
    async surfacePong() {},
    subscribeCanvasPing() {
      return () => {}
    },
    async edgeSave(_display, _displayId, position) {
      try {
        localStorage.setItem('kunsu:mock:edge', JSON.stringify(position))
      } catch {
        // 瀏覽器不給存就算了（mock 用）
      }
    },
    subscribeEdgeDragEnd() {
      return () => {}
    },

    async getEvidence(questionId, revision) {
      const q = find(questionId)
      if (!q?.draft || q.draft.revision !== revision) return null
      return structuredClone(MOCK_EVIDENCE[questionId] ?? { assets: {} })
    },

    async reauthorizeQuestion(id) {
      const q = find(id)
      if (!q || q.money_state !== 'reauthorizable') throw new Error('這題不能重新授權')
      // 與 server 一致：重新授權後錢鏈列重新排隊（money_state = running），跑完才有新結果；mock 一律成功。
      q.money_state = 'running'
      notifyList()
      window.setTimeout(() => simulateExecution(q, { forceOk: true }), 300)
      return structuredClone(q)
    },

    async resolveMoneyOutcome(questionId, executed) {
      const q = find(questionId)
      if (!q?.answer || q.money_state !== 'unknown') throw new Error('這題沒有結果不明、等人工對帳的錢')
      q.answer.execution_result = executed
        ? 'RECONCILED_EXECUTED: 使用者人工對帳確認錢已經動了'
        : 'RECONCILED_NOT_EXECUTED: 使用者人工對帳確認錢沒有動'
      q.answer.executed_at = now()
      q.money_state = executed ? 'settled' : 'reauthorizable'
      delete q.execution_blocked
      notifyList()
      return structuredClone(q)
    },

    async answerQuestion(payload) {
      const q = find(payload.question_id)
      if (!q) throw new Error(`題目不存在：${payload.question_id}`)
      if (q.state === 'answered' || q.state === 'cancelled') {
        // 競態只承認一次（AC-AN-13）
        return structuredClone(q)
      }
      const answered_at = now()
      if (payload.type === 'reject') {
        q.state = 'cancelled'
        // 與 server 一致（CONTRACT §9）：使用者推掉的題記 `rejected`，分得出「退回」與 AI 取消。
        q.rejected = { at: answered_at, reason: payload.reason ?? null }
        q.answer = {
          free_text: `【退回重擬】${payload.reason ?? '這題不該問我'}`,
          channel: 'ui',
          answered_at,
        }
        notifyList()
        return structuredClone(q)
      }
      if (payload.type === 'batch') {
        // 與 server 一致（CONTRACT §14）：逐項決定照題目項目順序保存，整體指示存成 free_text。
        const order = new Map((q.items ?? []).map((item, i) => [item.id, i]))
        q.state = 'answered'
        q.answer = {
          answer_id: crypto.randomUUID(),
          channel: 'ui',
          answered_at,
          free_text: payload.instruction?.trim() || undefined,
          items: [...payload.items].sort((a, b) => (order.get(a.item_id) ?? 0) - (order.get(b.item_id) ?? 0)),
          ...(q.draft ? { revision: q.draft.revision, content_hash: q.draft.content_hash } : {}),
        }
        notifyList()
        simulateContinuation(q)
        return structuredClone(q)
      }
      q.state = 'answered'
      q.answer = {
        answer_id: crypto.randomUUID(),
        choice_id: payload.type === 'choice' ? payload.choice_id : undefined,
        // 與 server 一致：選項＋補充說明時，補充存成 free_text（AC-DS-06）。
        free_text: payload.type === 'free_text' ? payload.free_text : payload.note?.trim() || undefined,
        edited_params: payload.type === 'choice' ? payload.edited_params : undefined,
        ...(q.draft && payload.revision !== undefined ? { revision: payload.revision, content_hash: q.draft.content_hash } : {}),
        channel: 'ui',
        answered_at,
        // 簽章只在 approval 題（黃燈動錢）出現；mock 給個樣子
        signature:
          payload.type === 'choice' && q.money?.is_money_op && q.gate !== 'red'
            ? 'hmac:mock-sig'
            : undefined,
      }
      notifyList()
      // 一鍵執行流：只有黃燈 choice 且帶 wake 才起（紅燈/自由文字只落 ledger）
      if (q.wake_on_answer.mode === 'continue') {
        // 回答延續原對話：模擬交付 pending → resuming → resumed＋AI 回饋（AC-CC-13）。
        simulateContinuation(q)
        return structuredClone(q)
      }
      const oneClick =
        payload.type === 'choice' && q.gate !== 'red' && q.wake_on_answer.mode !== 'none'
      if (oneClick) simulateExecution(q)
      return structuredClone(q)
    },

    async dismissQuestion(id) {
      const q = find(id)
      if (!q) throw new Error(`題目不存在：${id}`)
      // 瀏覽器 QA：localStorage `kunsu:mock:fail-dismiss=1` 模擬 dismissal 失敗（AC-OP-06）。
      if (localStorage.getItem('kunsu:mock:fail-dismiss') === '1' || failDismissFor.has(id)) throw new Error('（mock）資料庫暫時鎖住了')
      // 與 server 一致：已在終態的題不接受 dismissal（`ServerError::Terminal`）。
      if (isTerminal(q.state)) throw new Error(`題目 ${id} 已在終態 ${q.state}，不接受此操作`)
      const started = Date.now()
      q.snooze_count += 1
      q.cycle_number = (q.cycle_number ?? 1) + 1
      q.cycle_started_at = new Date(started).toISOString()
      q.warn_at = new Date(started + 12 * 60_000).toISOString()
      q.force_at = new Date(started + 15 * 60_000).toISOString()
      q.next_transition_at = q.warn_at
      q.state = 'pending'
      notifyList()
      return structuredClone(q)
    },

    async editOption(id, optionId, newTo) {
      const q = find(id)
      if (!q) throw new Error(`題目不存在：${id}`)
      const opt = q.options.find((o) => o.id === optionId)
      if (!opt) throw new Error(`選項不存在：${optionId}`)
      const parsed = parseMoneyChange(opt.label)
      if (!parsed) throw new Error('此選項無可編輯的金額變更')
      opt.label = recalcLine(opt.label, newTo)
      const redline = checkRedline(parsed.from, newTo)
      // params_line 同步重算（所見即所簽）
      if (q.params_line && parseMoneyChange(q.params_line)) {
        q.params_line = recalcLine(q.params_line, newTo)
      } else {
        q.params_line = `${q.money?.entity_id ?? q.id}｜${opt.label}`
      }
      if (redline.breached) {
        q.gate = 'red' // 視覺升紅＋鎖一鍵；server 端亦同步升級
      }
      notifyList()
      return { question: structuredClone(q), redline } satisfies EditOptionResult
    },

    async openDeepLink(target) {
      if (target.startsWith('kunsu://')) {
        console.info(`[mock] shell command → ${target}`)
        return
      }
      if (/^https?:\/\//.test(target)) {
        window.open(target, '_blank', 'noopener')
      } else {
        // vault 相對路徑：真實 app 由 Rust 端開 Finder/編輯器；mock 只提示
        console.info(`[mock] open_deep_link → ${target}`)
        window.alert(`（mock）開啟 vault 檔案：\n${target}`)
      }
    },

    async openOverlayFromCapsule(anchor) {
      overlayTransitionId += 1
      const transition: OverlayTransition = {
        id: overlayTransitionId,
        origin_x: anchor.local_x,
        origin_y: anchor.local_y,
        origin_size: anchor.size,
        source: 'preview',
      }
      overlaySubs.forEach((cb) => cb(transition))
    },

    async resizeCapsule() {
      // web/mock 無視窗殼：no-op
    },

    async startCapsuleDrag() {
      // web/mock 沒有原生視窗：保留 gesture controller 形狀供瀏覽器互動 QA。
      return {
        move() {},
        async end() {
          return 'top-left' as const
        },
      }
    },

    async killSwitch() {
      await this.dndSet(240, 'user')
    },

    async dndGet() {
      if (dnd.active && dnd.ends_at) {
        const remaining = Math.max(0, Math.ceil((new Date(dnd.ends_at).getTime() - Date.now()) / 1000))
        dnd = remaining === 0 ? { ...INACTIVE_DND } : { ...dnd, remaining_seconds: remaining }
      }
      return structuredClone(dnd)
    },

    async dndSet(durationMinutes, actor = 'ui') {
      if (!Number.isInteger(durationMinutes) || durationMinutes < 1 || durationMinutes > 1440) {
        throw new Error('勿擾時間必須在 1 到 1440 分鐘之間')
      }
      const starts = new Date()
      const ends = new Date(starts.getTime() + durationMinutes * 60_000)
      dnd = { active: true, starts_at: starts.toISOString(), ends_at: ends.toISOString(), remaining_seconds: durationMinutes * 60, actor }
      notifyList()
      return structuredClone(dnd)
    },

    async dndClear() {
      dnd = { ...INACTIVE_DND }
      notifyList()
      return structuredClone(dnd)
    },

    subscribeExecution(questionId, cb) {
      if (!execSubs.has(questionId)) execSubs.set(questionId, new Set())
      execSubs.get(questionId)!.add(cb)
      return () => execSubs.get(questionId)?.delete(cb)
    },

    subscribeQuestions(cb) {
      listSubs.add(cb)
      return () => listSubs.delete(cb)
    },

    subscribeOverlayTransition(cb) {
      overlaySubs.add(cb)
      return () => overlaySubs.delete(cb)
    },

    subscribeOpenSettings() {
      return () => {}
    },

    subscribeOpenFlow() {
      return () => {}
    },

    async overlayReady() {
      // 瀏覽器預覽沒有原生殼：模擬「網頁就緒後拉到一筆待播轉場」。
      overlayTransitionId += 1
      return { id: overlayTransitionId, origin_x: 28, origin_y: 61, origin_size: 18, source: 'preview' }
    },

    async overlayMounted(id) {
      console.info(`[mock] overlay mounted → ${id}`)
    },

    async overlayEscapeAck() {},

    async overlayCurrent(questionId) {
      console.info(`[mock] overlay current → ${questionId ?? '（無）'}`)
    },
  }
}
