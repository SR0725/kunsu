import Foundation

/// 模擬資料（同 docs/prototypes/unified-design.html）；正式版改由 Mac 上的 Kunsu 提供。
public enum Fixtures {
    public static func tonight() -> BoardState {
        let decisions = [
            Decision(id: "notch", agent: .codex, project: "kunsu", sessionID: "kunsu",
                     question: "邊緣瀏海拖到角落附近放開時，要吸附成 L 形，還是滑回直邊？",
                     why: "決定後 Codex 會照這個改 BDD 和測試，大約 20 分鐘。",
                     options: [Option("吸附成 L 形", detail: "距離角落 30% 以內就吸附，跟現在桌面一致", recommended: true),
                               Option("滑回直邊", detail: "永遠停在直邊，留 24 px 間距")], time: "23:31"),
            Decision(id: "hook", agent: .claude, project: "Threads 草稿", sessionID: nil,
                     question: "明天早上那篇 Threads，開頭要用哪一句？",
                     why: "三句都是根據你過去 30 天互動最好的貼文寫的，選好後明早 8:30 排程發出。",
                     options: [Option("我寫了 8 年程式，最後發現最難的不是技術，而是每天都要決定什麼不做。", recommended: true),
                               Option("一人公司最貴的成本不是錢，是你每天浪費在不重要決定上的那兩個小時。"),
                               Option("我把所有要我拍板的事丟給一個 App，它每 15 分鐘就來煩我一次，結果我真的變快了。"),
                               Option("都不好，請 AI 重寫三句")], time: "23:22"),
            Decision(id: "budget", agent: .claude, project: "ads-review", sessionID: "ads",
                     question: "「課程早鳥」CPA 漲到 NT$412，要把日預算降到 NT$800 嗎？",
                     why: "不處理的話今晚到明早大約再花 NT$1,100，預估多 2–3 筆轉換。",
                     options: [Option("降到 NT$800", detail: "立刻生效，學習期不重置", recommended: true),
                               Option("先不動", detail: "明早復盤再決定")],
                     money: [MoneyParam("廣告組", "課程早鳥｜再行銷 30 天"), MoneyParam("目前日預算", "NT$1,500"), MoneyParam("改為", "NT$800")],
                     time: "23:18"),
        ]
        let sessions: [String: Session] = [
            "kunsu": Session(id: "kunsu", agent: .codex, status: .reply, turns: [
                .user("外接螢幕上拖邊緣瀏海會跳一下，幫我修"), .tool("讀了 4 個檔 · 搜尋 2 次"),
                .ai("找到了：換螢幕時 grab offset 用的是舊螢幕的座標，第一幀就會跳。我改成在換 stage 時重新量。"),
                .tool("改了 3 個檔 · 跑了測試 ✓"),
                .ai("測試全綠了。要我直接 commit 並跑 build-install 嗎？\nA. commit 並安裝\nB. 先給我看 diff"),
            ]),
            "threads": Session(id: "threads", name: "blog-site", agent: .claude, status: .permission, turns: [
                .user("tracker 修好就推上去"), .tool("改了 2 個檔 · 跑了測試 ✓"), .ai("修好了，測試通過。接下來要推到遠端。"),
            ], permission: Permission(command: "git push origin feat/tracker", reason: "把 tracker 修正推到遠端分支，之後才能開 PR。")),
            "ads": Session(id: "ads", name: "ads-review", agent: .claude, status: .running("看素材疲勞中"), turns: [
                .user("每晚復盤：看 Meta 三個課程廣告組"), .tool("抓了 3 份報表"),
                .ai("「課程早鳥」CPA 從 NT$268 漲到 NT$412，我問你要不要降預算。"),
            ]),
        ]
        return BoardState(decisions: decisions, sessions: sessions, history: history)
    }

    public static func quiet() -> BoardState {
        var s = tonight()
        s.decisions = []
        s.sessions["kunsu"]?.status = .idle
        s.sessions["threads"]?.status = .idle
        s.sessions["threads"]?.permission = nil
        return s
    }

    static let history: [HistoryDay] = [
        HistoryDay("今晚", [
            HistoryEntry(time: "21:05", agent: .codex, project: "demo-shop", question: "你開了新任務", answer: "landing page hero 改成新文案", result: "AI：改好了，已部署到 Zeabur 預覽環境。"),
            HistoryEntry(time: "22:31", agent: .claude, project: "notes", question: "這週決策紀錄要放進 W41，還是新開一份？", answer: "放進 W41", result: "AI：整理好了，寫在 2026-W41.md。"),
        ]),
        HistoryDay("昨天", [
            HistoryEntry(time: "22:10", agent: .codex, project: "kunsu", question: "手機中轉要用 Tailscale 還是 Zeabur？", answer: "Zeabur", result: "AI：已寫進 PRD §30。"),
            HistoryEntry(time: "23:50", agent: .claude, project: "ads-review", question: "「主課程」素材疲勞了，要換成新影片嗎？", answer: "換成新影片", result: "AI：已上架，學習期重新開始。"),
        ]),
    ]
}
