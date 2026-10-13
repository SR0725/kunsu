//! CONTRACT §2 server 端硬驗證 + 十秒可決 lint，全部收在一個純函數。
//! enum 白名單由 serde 解析層擋（未知 variant 直接 deserialize error）；
//! 這裡驗 serde 型別擋不住的語義規則。錯誤訊息要讓 agent 讀得懂、
//! 知道怎麼修（server 拿 Display 直接回 400 body）。

use std::fmt;

use crate::types::{CreateRequest, Gate, ItemDefault, Kind, OnExpire};

/// 拒收原因。`is_malformed()` = 十秒可決 lint 類（CONTRACT：reject 標 `malformed`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RejectReason {
    /// id 是 upsert/dedup 鍵，不可為空。
    IdEmpty,
    /// id 字元/長度白名單（路徑安全：id 會成為 approval 檔名的一部分）。
    IdInvalidFormat { id: String },
    /// `money.executor_prepare_cmd` 不在白名單（INVARIANTS 第 17 條
    /// `inv_prepare_cmd_whitelist`；ask_create 預檢，不合格 400）。
    BadPrepareCmd { reason: String },
    /// prepare cmd 的 `--entity` 與 `money.entity_id` 不一致（所見即所簽前置）。
    PrepareCmdEntityMismatch { cmd_entity: String, entity_id: String },
    /// title 超過 80 字（以字元計）。
    TitleTooLong { len: usize },
    /// question_line 超過 24 字（以字元計）。語義不驗，只驗長度。
    QuestionLineTooLong { len: usize },
    /// visual_symbol 只容許一個短圖形／短字標，避免 agent 把內容塞進視覺欄位。
    VisualSymbolTooLong { len: usize },
    /// allow_free_text 恆 true。
    FreeTextMustBeAllowed,
    /// evidence 超過 3 條。
    TooManyEvidence { len: usize },
    /// evidence 第 index 條缺 source 身世標籤。
    EvidenceMissingSource { index: usize },
    /// 錢類題缺 prediction_ref（inv_money_requires_prediction）。
    MoneyRequiresPrediction,
    /// 錢類題 on_expire=pick_option——錢永不因超時自動動（inv_money_never_auto_expire）。
    MoneyNeverAutoExpire,
    /// gate=red 必須 choice 多選項拍板（inv_red_needs_options）。
    RedGateNeedsChoice,
    /// 十秒可決 lint：choice 題 options 必須 2..=4。
    ChoiceOptionsOutOfRange { len: usize },
    /// 十秒可決 lint：動錢題必附 params_line（所見即所簽）。
    MoneyNeedsParamsLine,
    /// 動錢題必須是 choice 且有執行型選項（recommended）；否則使用者按下去錢鏈永遠不會跑。
    MoneyNeedsExecuteOption,
    /// 非紅燈的動錢題必須帶 `money.executor_prepare_cmd`；否則使用者按「批准並執行」錢鏈不會跑。
    MoneyNeedsPrepareCmd,
    /// on_expire=pick_option 必須指定 option_id。
    PickOptionNeedsOptionId,
    /// 原對話身分格式不合法（會被組進 CLI 參數；不得以 `-` 開頭或含空白／符號）。
    BadConversationIdentity { field: &'static str },
    /// pick_option 指到不存在的選項。
    PickOptionUnknownOptionId { option_id: String },
    /// `source.expected_every_hours` 必須是有限正數（AC-RC-01b）。
    BadRoutineCadence,
    /// 題組項目數必須 2..=20（PRD §25）。
    BatchItemsOutOfRange { len: usize },
    /// 只有題組可以帶 items。
    ItemsOnlyForBatch { len: usize },
    /// 題組沒有選項（逐項決定取代選項）。
    BatchNoOptions { len: usize },
    /// 題組不能放動錢的項目：錢題仍一題一題走授權鏈。
    BatchMoneyForbidden,
    /// 題組不能帶 kongko_ref：Kongko 發佈事件沒有逐項決定，不能結案題組（AC-BR-05）。
    BatchKongkoRef,
    /// 題組 id（不含輪次後綴）超過 116 字：第 2 輪以後的 `<id>-rN` 會超過題目 id 上限（AC-BR-06）。
    BatchIdTooLong { len: usize },
    /// 題組第 index 項（0-based）的欄位問題。
    BatchItem { index: usize, item_id: String, problem: BatchItemProblem },
}

/// 題組單項的問題（`RejectReason::BatchItem`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BatchItemProblem {
    IdInvalid,
    IdDuplicate,
    TitleEmpty,
    TitleTooLong { len: usize },
    ProposalEmpty,
    ProposalTooLong { len: usize },
    SubtitleTooLong { len: usize },
    SkipNeedsReason,
    /// `context_md` 沒通過 Markdown 安全檢查（server 端套用草稿的規則）。
    Markdown { detail: String },
}

impl BatchItemProblem {
    fn code(&self) -> &'static str {
        match self {
            BatchItemProblem::IdInvalid => "batch_item_id_invalid",
            BatchItemProblem::IdDuplicate => "batch_item_id_duplicate",
            BatchItemProblem::TitleEmpty => "batch_item_title_empty",
            BatchItemProblem::TitleTooLong { .. } => "batch_item_title_too_long",
            BatchItemProblem::ProposalEmpty => "batch_item_proposal_empty",
            BatchItemProblem::ProposalTooLong { .. } => "batch_item_proposal_too_long",
            BatchItemProblem::SubtitleTooLong { .. } => "batch_item_subtitle_too_long",
            BatchItemProblem::SkipNeedsReason => "batch_item_skip_needs_reason",
            BatchItemProblem::Markdown { .. } => "batch_item_markdown",
        }
    }

    /// 出問題的欄位名（錯誤訊息與草稿行號用）。
    pub fn field(&self) -> &'static str {
        match self {
            BatchItemProblem::IdInvalid | BatchItemProblem::IdDuplicate => "id",
            BatchItemProblem::TitleEmpty | BatchItemProblem::TitleTooLong { .. } => "title",
            BatchItemProblem::ProposalEmpty | BatchItemProblem::ProposalTooLong { .. } => "proposal",
            BatchItemProblem::SubtitleTooLong { .. } => "subtitle",
            BatchItemProblem::SkipNeedsReason => "default_reason",
            BatchItemProblem::Markdown { .. } => "context_md",
        }
    }
}

impl RejectReason {
    /// 十秒可決 lint 類 → CONTRACT 要求 reject 標 `malformed`。
    pub fn is_malformed(&self) -> bool {
        matches!(
            self,
            RejectReason::ChoiceOptionsOutOfRange { .. } | RejectReason::MoneyNeedsParamsLine
        )
    }

    /// 機器可讀代碼（server 回 400 body 附上，方便 agent 分支處理）。
    pub fn code(&self) -> &'static str {
        match self {
            RejectReason::IdEmpty => "id_empty",
            RejectReason::IdInvalidFormat { .. } => "id_invalid_format",
            RejectReason::BadPrepareCmd { .. } => "bad_prepare_cmd",
            RejectReason::PrepareCmdEntityMismatch { .. } => "prepare_cmd_entity_mismatch",
            RejectReason::TitleTooLong { .. } => "title_too_long",
            RejectReason::QuestionLineTooLong { .. } => "question_line_too_long",
            RejectReason::VisualSymbolTooLong { .. } => "visual_symbol_too_long",
            RejectReason::FreeTextMustBeAllowed => "free_text_must_be_allowed",
            RejectReason::TooManyEvidence { .. } => "too_many_evidence",
            RejectReason::EvidenceMissingSource { .. } => "evidence_missing_source",
            RejectReason::MoneyRequiresPrediction => "money_requires_prediction",
            RejectReason::MoneyNeverAutoExpire => "money_never_auto_expire",
            RejectReason::RedGateNeedsChoice => "red_gate_needs_choice",
            RejectReason::ChoiceOptionsOutOfRange { .. } => "malformed_choice_options",
            RejectReason::MoneyNeedsParamsLine => "malformed_money_params_line",
            RejectReason::MoneyNeedsExecuteOption => "money_needs_execute_option",
            RejectReason::MoneyNeedsPrepareCmd => "money_needs_prepare_cmd",
            RejectReason::PickOptionNeedsOptionId => "pick_option_needs_option_id",
            RejectReason::BadConversationIdentity { .. } => "bad_conversation_identity",
            RejectReason::BadRoutineCadence => "bad_routine_cadence",
            RejectReason::PickOptionUnknownOptionId { .. } => "pick_option_unknown_option_id",
            RejectReason::BatchItemsOutOfRange { .. } => "batch_items_out_of_range",
            RejectReason::ItemsOnlyForBatch { .. } => "items_only_for_batch",
            RejectReason::BatchNoOptions { .. } => "batch_no_options",
            RejectReason::BatchMoneyForbidden => "batch_money_forbidden",
            RejectReason::BatchKongkoRef => "batch_kongko_ref",
            RejectReason::BatchIdTooLong { .. } => "batch_id_too_long",
            RejectReason::BatchItem { problem, .. } => problem.code(),
        }
    }
}

impl fmt::Display for RejectReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RejectReason::IdEmpty => write!(f, "id 不可為空：它是 dedup/upsert 鍵，請沿用穩定 id（如 pa-*）"),
            RejectReason::IdInvalidFormat { id } => write!(
                f,
                "id {id:?} 非法：只允許 ASCII [A-Za-z0-9._-]、長度 ≤ {MAX_ID_LEN}、不得以 . 開頭（id 會成為檔名，路徑安全白名單）"
            ),
            RejectReason::BadPrepareCmd { reason } => write!(
                f,
                "money.executor_prepare_cmd 不在白名單（inv_prepare_cmd_whitelist）：{reason}。只認 python3 <…>/executor.py prepare-budget|prepare-bid 加 --entity/--new-budget/--new-value/--field/--product"
            ),
            RejectReason::PrepareCmdEntityMismatch { cmd_entity, entity_id } => write!(
                f,
                "prepare cmd 的 --entity {cmd_entity:?} 與 money.entity_id {entity_id:?} 不一致——所見即所簽要求兩者相同"
            ),
            RejectReason::TitleTooLong { len } => write!(f, "title 長度 {len} 超過 80 字上限，請精簡"),
            RejectReason::QuestionLineTooLong { len } => {
                write!(f, "question_line 長度 {len} 超過 24 字上限：這是蓋板大字，必須是一句可決斷的短句")
            }
            RejectReason::VisualSymbolTooLong { len } => {
                write!(f, "visual_symbol 長度 {len} 超過 4 字上限：只允許一個短圖形或短字標")
            }
            RejectReason::FreeTextMustBeAllowed => write!(f, "allow_free_text 恆為 true，使用者永遠可以用自由文字回答"),
            RejectReason::TooManyEvidence { len } => write!(f, "evidence 有 {len} 條，上限 3 條：挑最關鍵的"),
            RejectReason::EvidenceMissingSource { index } => {
                write!(f, "evidence[{index}] 缺 source 身世標籤：每條證據必須標明出處")
            }
            RejectReason::MoneyRequiresPrediction => {
                write!(f, "動錢題必附 prediction_ref：沒有配對 prediction 的錢類決策不允許落地")
            }
            RejectReason::MoneyNeverAutoExpire => {
                write!(f, "動錢題的 default_action.on_expire 不可為 pick_option：錢永不因超時自動動")
            }
            RejectReason::RedGateNeedsChoice => {
                write!(f, "gate=red 的題必須 kind=choice 多選項拍板，不能 confirm/text")
            }
            RejectReason::ChoiceOptionsOutOfRange { len } => {
                write!(f, "choice 題 options 必須 2..=4 個（收到 {len} 個）——十秒可決 lint（malformed）")
            }
            RejectReason::MoneyNeedsParamsLine => {
                write!(f, "動錢題必附 params_line 精確變更一行（所見即所簽）——十秒可決 lint（malformed）")
            }
            RejectReason::MoneyNeedsPrepareCmd => write!(
                f,
                "黃燈（一鍵執行）的動錢題必須帶 money.executor_prepare_cmd——沒有 prepare 命令錢鏈不會跑；不能一鍵執行就設 gate: red"
            ),
            RejectReason::MoneyNeedsExecuteOption => write!(
                f,
                "動錢題必須是 choice，並有一個執行型選項（recommended: true）與安全選項——純確認題或沒有執行型選項時，使用者按下去錢鏈永遠不會跑"
            ),
            RejectReason::PickOptionNeedsOptionId => {
                write!(f, "on_expire=pick_option 必須同時指定 default_action.option_id")
            }
            RejectReason::PickOptionUnknownOptionId { option_id } => {
                write!(f, "default_action.option_id={option_id:?} 不在 options 清單裡")
            }
            RejectReason::BadConversationIdentity { field } => write!(
                f,
                "原對話身分 {field} 格式不合法：只能是英數、-、_（≤128 字、不得以 - 開頭）；Claude 桌面 host_session_id 必須是 local_ 開頭"
            ),
            RejectReason::BadRoutineCadence => {
                write!(f, "source.expected_every_hours 必須是正數小時（例如 24＝每天、168＝每週）")
            }
            RejectReason::BatchItemsOutOfRange { len } => write!(
                f,
                "題組要 1–20 項（收到 {len} 項）：超過 20 項先自己篩或拆成兩題"
            ),
            RejectReason::ItemsOnlyForBatch { len } => {
                write!(f, "只有 kind: batch（題組）可以帶 items（收到 {len} 項）")
            }
            RejectReason::BatchNoOptions { len } => write!(
                f,
                "題組不帶 options（收到 {len} 個）：每項的決定是採用／不處理／請 AI 改"
            ),
            RejectReason::BatchMoneyForbidden => write!(
                f,
                "題組不能放動錢的項目（money.is_money_op）：錢題請一題一題建，走批准授權鏈"
            ),
            RejectReason::BatchKongkoRef => write!(
                f,
                "題組不能帶 kongko_ref：Kongko 發佈事件沒有逐項決定，不能結案題組；題組只能在 Kunsu 面板逐項作答，素材題請用一般題"
            ),
            RejectReason::BatchIdTooLong { len } => write!(
                f,
                "題組 id 太長（{len} 字）：題組 id（不含 -rN 輪次後綴）最多 {} 字，要留空間給第 2 輪以後的 `<id>-r2`～`-r99`（題目 id 上限 {MAX_ID_LEN} 字）",
                crate::batch::MAX_BATCH_BASE_ID_LEN
            ),
            RejectReason::BatchItem { index, item_id, problem } => {
                let field = problem.field();
                write!(f, "items[{index}]（第 {} 項，id {item_id:?}）的 {field}：", index + 1)?;
                match problem {
                    BatchItemProblem::IdInvalid => write!(
                        f,
                        "只允許 ASCII [A-Za-z0-9._-]、長度 ≤ {MAX_ID_LEN}、不得以 . 開頭（項目 id 必須路徑安全）"
                    ),
                    BatchItemProblem::IdDuplicate => write!(f, "id {item_id:?} 跟前面的項目重複：每項 id 在題內唯一"),
                    BatchItemProblem::TitleEmpty => write!(f, "標題不能空白：寫寄件人＋主旨、留言者或申請人"),
                    BatchItemProblem::TitleTooLong { len } => write!(f, "標題 {len} 字超過 80 字上限"),
                    BatchItemProblem::ProposalEmpty => write!(f, "AI 的草稿或建議不能空白"),
                    BatchItemProblem::ProposalTooLong { len } => {
                        write!(f, "草稿或建議 {len} 字超過 5,000 字上限")
                    }
                    BatchItemProblem::SubtitleTooLong { len } => write!(f, "副標 {len} 字超過 200 字上限"),
                    BatchItemProblem::SkipNeedsReason => {
                        write!(f, "default: skip（建議不處理）要寫明原因")
                    }
                    BatchItemProblem::Markdown { detail } => write!(f, "{detail}"),
                }
            }
        }
    }
}

impl std::error::Error for RejectReason {}

fn is_blank(s: &str) -> bool {
    s.trim().is_empty()
}

/// id 長度上限（路徑安全白名單）。
pub const MAX_ID_LEN: usize = 120;

/// 路徑安全的 id 白名單（INVARIANTS 路徑安全條款）：ASCII `[A-Za-z0-9._-]`、
/// 長度 ≤ 120、非空、不以 `.` 開頭（杜絕 `.`/`..`/隱藏檔與任何路徑分隔符）。
pub fn valid_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= MAX_ID_LEN
        && !s.starts_with('.')
        && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// 建題硬驗證 + 十秒可決 lint（CONTRACT §2）。serde 解析成功 ≠ 通過；
/// server 必須先 deserialize（enum 白名單）再呼叫本函數。
/// 8-4-4-4-12 的十六進位 UUID。
fn is_uuid(s: &str) -> bool {
    let parts: Vec<&str> = s.split('-').collect();
    parts.len() == 5
        && parts.iter().zip([8, 4, 4, 4, 12]).all(|(p, n)| p.len() == n && p.chars().all(|c| c.is_ascii_hexdigit()))
}

pub fn validate_create(req: &CreateRequest) -> Result<(), RejectReason> {
    if is_blank(&req.id) {
        return Err(RejectReason::IdEmpty);
    }
    if !valid_id(&req.id) {
        return Err(RejectReason::IdInvalidFormat { id: req.id.clone() });
    }
    let title_len = req.title.chars().count();
    if title_len > 80 {
        return Err(RejectReason::TitleTooLong { len: title_len });
    }
    let ql_len = req.question_line.chars().count();
    if ql_len > 24 {
        return Err(RejectReason::QuestionLineTooLong { len: ql_len });
    }
    if let Some(symbol) = req.visual_symbol.as_deref() {
        let symbol_len = symbol.chars().count();
        if symbol_len > 4 {
            return Err(RejectReason::VisualSymbolTooLong { len: symbol_len });
        }
    }
    if !req.allow_free_text {
        return Err(RejectReason::FreeTextMustBeAllowed);
    }
    // 原對話身分會被組進 CLI 參數（--resume <id>、resume <thread>）：不得是旗標或含空白／符號。
    let conv_ok = |s: &str| {
        s.len() <= 128
            && !s.starts_with('-')
            && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    };
    // Codex 的 thread id 是 UUID；非 UUID 會被 `codex exec resume` 當成 thread 名稱去找，可能接到別的對話。
    let codex = req.source.agent == crate::types::AgentKind::Codex
        || matches!(req.source.host, Some(crate::continuity::HostKind::CodexApp | crate::continuity::HostKind::CodexCli));
    if let Some(c) = req.source.conversation_id.as_deref().filter(|c| !c.trim().is_empty())
        && (!conv_ok(c) || (codex && !is_uuid(c)))
    {
        return Err(RejectReason::BadConversationIdentity { field: "source.conversation_id" });
    }
    if let Some(h) = req.source.host_session_id.as_deref().filter(|h| !h.trim().is_empty())
        && !h.strip_prefix("local_").is_some_and(|rest| {
            !rest.is_empty() && rest.len() <= 80 && rest.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        })
    {
        return Err(RejectReason::BadConversationIdentity { field: "source.host_session_id" });
    }
    if req.wake_on_answer.mode == crate::types::WakeMode::Resume
        && let Some(sid) = req.wake_on_answer.session_id.as_deref().filter(|s| !s.trim().is_empty())
        && !conv_ok(sid)
    {
        return Err(RejectReason::BadConversationIdentity { field: "wake_on_answer.session_id" });
    }
    if req.source.expected_every_hours.is_some_and(|h| !(h.is_finite() && h > 0.0)) {
        return Err(RejectReason::BadRoutineCadence);
    }
    if req.evidence.len() > 3 {
        return Err(RejectReason::TooManyEvidence { len: req.evidence.len() });
    }
    if let Some(index) = req.evidence.iter().position(|e| is_blank(&e.source)) {
        return Err(RejectReason::EvidenceMissingSource { index });
    }

    let is_money = req.money.as_ref().is_some_and(|m| m.is_money_op);
    // 題組（PRD §25）：先於錢題檢查，讓「題組不能動錢」成為第一個原因。
    if req.kind == Kind::Batch {
        validate_batch(req, is_money)?;
    } else if !req.items.is_empty() {
        return Err(RejectReason::ItemsOnlyForBatch { len: req.items.len() });
    }
    if is_money && req.prediction_ref.as_deref().is_none_or(is_blank) {
        return Err(RejectReason::MoneyRequiresPrediction);
    }
    // INVARIANTS 第 17 條 inv_prepare_cmd_whitelist：ask_create 預檢。
    // 不合格的 prepare cmd 一律 400——不允許任何非白名單命令進 DB。
    if let Some(m) = &req.money
        && let Some(cmd) = m.executor_prepare_cmd.as_deref()
    {
        let op = crate::prepare_cmd::parse_prepare_cmd(cmd)
            .map_err(|reason| RejectReason::BadPrepareCmd { reason })?;
        if let Some(entity_id) = m.entity_id.as_deref()
            && entity_id != op.entity
        {
            return Err(RejectReason::PrepareCmdEntityMismatch {
                cmd_entity: op.entity,
                entity_id: entity_id.to_string(),
            });
        }
    }
    if is_money && req.default_action.on_expire == OnExpire::PickOption {
        return Err(RejectReason::MoneyNeverAutoExpire);
    }
    if req.gate == Some(Gate::Red) && req.kind != Kind::Choice {
        return Err(RejectReason::RedGateNeedsChoice);
    }

    // 十秒可決 lint
    if req.kind == Kind::Choice && !(2..=4).contains(&req.options.len()) {
        return Err(RejectReason::ChoiceOptionsOutOfRange { len: req.options.len() });
    }
    if is_money && req.params_line.as_deref().is_none_or(is_blank) {
        return Err(RejectReason::MoneyNeedsParamsLine);
    }
    if is_money && (req.kind != Kind::Choice || !req.options.iter().any(|o| o.recommended == Some(true))) {
        return Err(RejectReason::MoneyNeedsExecuteOption);
    }
    if is_money
        && req.gate != Some(Gate::Red)
        && req.money.as_ref().and_then(|m| m.executor_prepare_cmd.as_deref()).is_none_or(is_blank)
    {
        return Err(RejectReason::MoneyNeedsPrepareCmd);
    }

    if req.default_action.on_expire == OnExpire::PickOption {
        match &req.default_action.option_id {
            None => return Err(RejectReason::PickOptionNeedsOptionId),
            Some(oid) if !req.options.iter().any(|o| &o.id == oid) => {
                return Err(RejectReason::PickOptionUnknownOptionId { option_id: oid.clone() });
            }
            _ => {}
        }
    }

    Ok(())
}

/// 題組的建題規則（CONTRACT §14）。
fn validate_batch(req: &CreateRequest, is_money: bool) -> Result<(), RejectReason> {
    use crate::batch::{MAX_DRAFT_CHARS, MAX_ITEMS, MAX_TITLE_CHARS, MIN_ITEMS};
    if is_money {
        return Err(RejectReason::BatchMoneyForbidden);
    }
    if req.kongko_ref.as_deref().is_some_and(|r| !is_blank(r)) {
        return Err(RejectReason::BatchKongkoRef);
    }
    let base_len = crate::batch::round_of(&req.id).0.chars().count();
    if base_len > crate::batch::MAX_BATCH_BASE_ID_LEN {
        return Err(RejectReason::BatchIdTooLong { len: base_len });
    }
    if !req.options.is_empty() {
        return Err(RejectReason::BatchNoOptions { len: req.options.len() });
    }
    if !(MIN_ITEMS..=MAX_ITEMS).contains(&req.items.len()) {
        return Err(RejectReason::BatchItemsOutOfRange { len: req.items.len() });
    }
    for (index, item) in req.items.iter().enumerate() {
        let fail = |problem| RejectReason::BatchItem { index, item_id: item.id.clone(), problem };
        if !valid_id(&item.id) {
            return Err(fail(BatchItemProblem::IdInvalid));
        }
        if req.items[..index].iter().any(|prev| prev.id == item.id) {
            return Err(fail(BatchItemProblem::IdDuplicate));
        }
        if is_blank(&item.title) {
            return Err(fail(BatchItemProblem::TitleEmpty));
        }
        let title_len = item.title.chars().count();
        if title_len > MAX_TITLE_CHARS {
            return Err(fail(BatchItemProblem::TitleTooLong { len: title_len }));
        }
        // AI 建議不處理的可編輯項目給空草稿或真的回覆，不准佔位文字（2026-10-09 作者）：只有這種項目可以是空草稿。
        let may_be_empty = item.editable && item.default == ItemDefault::Skip;
        if is_blank(&item.proposal) && !may_be_empty {
            return Err(fail(BatchItemProblem::ProposalEmpty));
        }
        // 只核可的建議也有上限：回原對話的訊息逐項附上建議，要塞得進命令列參數（QA continuity 第 9 輪）。
        let proposal_len = item.proposal.chars().count();
        if proposal_len > MAX_DRAFT_CHARS {
            return Err(fail(BatchItemProblem::ProposalTooLong { len: proposal_len }));
        }
        let subtitle_len = item.subtitle.as_deref().map(|s| s.chars().count()).unwrap_or(0);
        if subtitle_len > crate::batch::MAX_SUBTITLE_CHARS {
            return Err(fail(BatchItemProblem::SubtitleTooLong { len: subtitle_len }));
        }
        if item.default == ItemDefault::Skip && item.default_reason.as_deref().is_none_or(is_blank) {
            return Err(fail(BatchItemProblem::SkipNeedsReason));
        }
    }
    Ok(())
}
