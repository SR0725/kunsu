//! 回答延續原對話的交付階梯（BDD conversation-continuity，AC-CC-03/04/05/06/07/09/10/11）。
//! 純函數表驅動：輸入由 server 層探測，這裡只驗證決策。

use kunsu_core::continuity::*;
use kunsu_core::types::{AgentKind, Source, Wake, WakeMode};

fn src(agent: AgentKind) -> Source {
    Source { agent, cwd: "/work".into(), ..Default::default() }
}

fn claude_desktop() -> Source {
    Source {
        host: Some(HostKind::ClaudeDesktop),
        conversation_id: Some("cli-1".into()),
        host_session_id: Some("local_abc-123".into()),
        ..src(AgentKind::Claude)
    }
}

fn input(current: DeliveryState) -> LadderInput {
    LadderInput {
        continuation: true,
        wake_mode: WakeMode::Continue,
        current,
        waiter_connected: false,
        waiter_lease_active: false,
        conversation_held: false,
    }
}

// ---------------------------------------------------------------------------
// 交付狀態
// ---------------------------------------------------------------------------

#[test]
fn delivery_only_pending_and_held_are_open() {
    use DeliveryState::*;
    for (s, open) in [
        (Pending, true),
        (Held, true),
        // resuming：reactor 佔住交付權的過渡狀態，對其他路徑而言已定案（INV-18 例外由 reactor 自己結束）。
        (Resuming, false),
        (LiveWaiter, false),
        (Resumed, false),
        (PickedUp, false),
        (Fresh, false),
        (NoContinuation, false),
        (Dead, false),
    ] {
        assert_eq!(s.is_final(), !open, "{s:?}");
        assert!(!s.as_str().is_empty());
    }
    assert_eq!(LiveWaiter.as_str(), "live_waiter");
    assert_eq!(NoContinuation.as_str(), "no_continuation");
}

// ---------------------------------------------------------------------------
// 原對話身分與 deep link（AC-CC-03、AC-CC-13）
// ---------------------------------------------------------------------------

#[test]
fn conversation_id_prefers_source_then_legacy_resume_session() {
    let wake_none = Wake::default();
    assert_eq!(conversation_id(&claude_desktop(), &wake_none).as_deref(), Some("cli-1"));
    let legacy = Wake { mode: WakeMode::Resume, session_id: Some("old-sid".into()), playbook: None };
    assert_eq!(conversation_id(&src(AgentKind::Claude), &legacy).as_deref(), Some("old-sid"));
    let fresh_with_sid = Wake { mode: WakeMode::Fresh, session_id: Some("x".into()), playbook: None };
    assert_eq!(conversation_id(&src(AgentKind::Claude), &fresh_with_sid), None, "只有 resume 模式的 session_id 才算");
    assert_eq!(conversation_id(&src(AgentKind::Claude), &wake_none), None);
    let blank = Source { conversation_id: Some("  ".into()), ..src(AgentKind::Claude) };
    assert_eq!(conversation_id(&blank, &wake_none), None, "空白 id 不算");
}

#[test]
fn continuation_applies_when_conversation_known_except_internal() {
    let wake = Wake::default();
    assert!(continuation_applies(&claude_desktop(), &wake, false));
    assert!(!continuation_applies(&claude_desktop(), &wake, true), "doctor internal 題永不接續");
    assert!(!continuation_applies(&src(AgentKind::Claude), &wake, false), "沒有原對話身分 → 現行行為");
}

#[test]
fn conversation_link_for_claude_desktop_and_codex() {
    assert_eq!(
        conversation_link(&claude_desktop()).as_deref(),
        Some("claude://code/continue?session=local_abc-123")
    );
    let codex = Source {
        host: Some(HostKind::CodexApp),
        conversation_id: Some("01a1-thread".into()),
        ..src(AgentKind::Codex)
    };
    assert_eq!(conversation_link(&codex).as_deref(), Some("codex://threads/01a1-thread"));
    let codex_cli = Source { host: Some(HostKind::CodexCli), ..codex.clone() };
    assert_eq!(conversation_link(&codex_cli).as_deref(), Some("codex://threads/01a1-thread"));
    let claude_cli = Source {
        host: Some(HostKind::ClaudeCli),
        conversation_id: Some("cli-1".into()),
        ..src(AgentKind::Claude)
    };
    assert_eq!(conversation_link(&claude_cli), None, "純 CLI 對話沒有可打開的 GUI");
    let bad_host_session = Source { host_session_id: Some("local_bad\"><script>".into()), ..claude_desktop() };
    assert_eq!(conversation_link(&bad_host_session), None, "host_session_id 必須符合 local_[A-Za-z0-9-]");
}

// ---------------------------------------------------------------------------
// 交付階梯（AC-CC-04/05/06/07/10/11）
// ---------------------------------------------------------------------------

#[test]
fn ladder_table() {
    use DeliveryState::*;
    // (說明, 輸入, 預期)
    let cases: Vec<(&str, LadderInput, LadderStep)> = vec![
        ("已交給等待者 → 不再動", input(LiveWaiter), LadderStep::Done),
        ("已追加 → 不再動", input(Resumed), LadderStep::Done),
        ("heartbeat 已取走 → 不再動", input(PickedUp), LadderStep::Done),
        ("已開新對話 → 不再動", input(Fresh), LadderStep::Done),
        ("等待者在線 → 等它取走", LadderInput { waiter_connected: true, ..input(Pending) }, LadderStep::AwaitWaiter),
        ("兩輪之間 10 秒租約 → 仍算在等", LadderInput { waiter_lease_active: true, ..input(Pending) }, LadderStep::AwaitWaiter),
        ("held 時等待者回來 → 交給它", LadderInput { waiter_connected: true, ..input(Held) }, LadderStep::AwaitWaiter),
        ("原對話被活行程持有 → held", LadderInput { conversation_held: true, ..input(Pending) }, LadderStep::Hold),
        ("held 重檢仍被持有 → 繼續 held", LadderInput { conversation_held: true, ..input(Held) }, LadderStep::Hold),
        ("原對話已結束 → 追加", input(Pending), LadderStep::Resume),
        ("held 釋放後 → 追加", input(Held), LadderStep::Resume),
        (
            "沒有原對話身分 + fresh → 開新對話",
            LadderInput { continuation: false, wake_mode: WakeMode::Fresh, ..input(Pending) },
            LadderStep::Fresh { reason: "沒有原對話可接續".into() },
        ),
        (
            "沒有原對話身分 + none → 不喚醒",
            LadderInput { continuation: false, wake_mode: WakeMode::None, ..input(Pending) },
            LadderStep::NoWake,
        ),
    ];
    for (why, i, expected) in cases {
        assert_eq!(next_step(&i), expected, "{why}");
    }
}

#[test]
fn ladder_waiter_beats_held_conversation() {
    // 等待者本身就在原對話裡：它在 → 交給它，絕不因登記檔顯示對話活著而 held。
    let i = LadderInput { waiter_connected: true, conversation_held: true, ..input(DeliveryState::Pending) };
    assert_eq!(next_step(&i), LadderStep::AwaitWaiter);
}

// ---------------------------------------------------------------------------
// resume 失敗分類（AC-CC-07、AC-CC-10）
// ---------------------------------------------------------------------------

/// 只看 stderr 與結尾的錯誤事件，不看 agent 的工作內容（QA 第二輪 C）；agent 已經開始處理就不再重試或
/// 開新對話（QA 第二輪 B）；「被鎖」只屬於 Codex。
#[test]
fn resume_failure_classification() {
    let codex_locked_stderr = "Error: thread 019a already has an active writer";
    assert_eq!(classify_resume_failure(true, "", codex_locked_stderr), ResumeFailure::Locked);
    let codex_locked_event = r#"{"type":"error","message":"thread 019a already has an active writer"}"#;
    assert_eq!(classify_resume_failure(true, codex_locked_event, ""), ResumeFailure::Locked);
    assert_eq!(
        classify_resume_failure(false, "", codex_locked_stderr),
        ResumeFailure::Transient,
        "Claude 沒有「被鎖」這回事"
    );
    assert_eq!(
        classify_resume_failure(false, "", "No conversation found with session ID: 0000"),
        ResumeFailure::NotFound
    );
    assert_eq!(
        classify_resume_failure(true, "", "Error: thread/resume: thread/resume failed: no rollout found for thread id 0000 (code -32600)"),
        ResumeFailure::NotFound
    );
    assert_eq!(classify_resume_failure(false, "", "connection reset by peer"), ResumeFailure::Transient);

    // agent 的工作內容裡出現那些字（例如讀到 BDD）不算。
    let claude_turn = concat!(
        r#"{"type":"system","subtype":"init","session_id":"s1"}"#,
        "\n",
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"No conversation found / already has an active writer"}]}}"#,
        "\n",
        r#"{"type":"result","subtype":"error_max_budget_usd","is_error":true,"result":"budget"}"#,
    );
    assert_eq!(classify_resume_failure(false, claude_turn, "API overloaded"), ResumeFailure::Started);
    let codex_turn = concat!(
        r#"{"type":"thread.started","thread_id":"t1"}"#,
        "\n",
        r#"{"type":"item.completed","item":{"type":"agent_message","text":"no rollout found"}}"#,
        "\n",
        r#"{"type":"turn.failed","error":{"message":"rate limited"}}"#,
    );
    assert_eq!(classify_resume_failure(true, codex_turn, ""), ResumeFailure::Started);
    // 只有 init、還沒開始處理 → 可以重試。
    let init_only = r#"{"type":"system","subtype":"init","session_id":"s1"}"#;
    assert_eq!(classify_resume_failure(false, init_only, "overloaded"), ResumeFailure::Transient);
    // QA 第三輪：Claude 的 API 錯誤以帶 `error` 的 assistant 事件（model "<synthetic>"）呈現，不算開始處理。
    let api_error = r#"{"type":"assistant","error":"overloaded","message":{"model":"<synthetic>","content":[{"type":"text","text":"API Error: 529 overloaded"}]}}"#;
    assert_eq!(classify_resume_failure(false, api_error, ""), ResumeFailure::Transient);
    let synthetic = r#"{"type":"assistant","message":{"model":"<synthetic>","content":[{"type":"text","text":"Credit balance too low"}]}}"#;
    assert!(!agent_started(false, synthetic));
    // 錯誤事件裡的「找不到」照樣算。
    let not_found_event = r#"{"type":"result","is_error":true,"result":"No conversation found with session ID: x"}"#;
    assert_eq!(classify_resume_failure(false, not_found_event, ""), ResumeFailure::NotFound);
}

// ---------------------------------------------------------------------------
// 主機判定（AC-CC-01/02）
// ---------------------------------------------------------------------------

#[test]
fn host_detection_from_environment() {
    assert_eq!(
        host_from_env(Some("com.anthropic.claudefordesktop"), Some("claude-desktop"), false),
        Some(HostKind::ClaudeDesktop)
    );
    assert_eq!(host_from_env(None, Some("cli"), false), Some(HostKind::ClaudeCli));
    assert_eq!(host_from_env(None, Some("sdk-cli"), false), Some(HostKind::ClaudeCli));
    assert_eq!(host_from_env(Some("com.openai.codex"), None, true), Some(HostKind::CodexApp));
    assert_eq!(host_from_env(None, None, true), Some(HostKind::CodexCli));
    assert_eq!(host_from_env(None, None, false), None);
}

#[test]
fn source_serde_accepts_identity_fields_and_old_payloads() {
    let old: Source = serde_json::from_str(r#"{"agent":"claude","cwd":"/x"}"#).unwrap();
    assert_eq!(old.conversation_id, None);
    let full: Source = serde_json::from_str(
        r#"{"agent":"codex","cwd":"/x","host":"codex_app","conversation_id":"t1","attended":false,
            "trigger":"scheduled","routine_id":"r","routine_name":"日記","routine_kind":"codex_cron",
            "expected_every_hours":24}"#,
    )
    .unwrap();
    assert_eq!(full.host, Some(HostKind::CodexApp));
    assert_eq!(full.trigger, Some(Trigger::Scheduled));
    assert_eq!(full.routine_kind, Some(RoutineKind::CodexCron));
    assert!(serde_json::from_str::<Source>(r#"{"agent":"claude","cwd":"/x","host":"vscode"}"#).is_err(), "host 白名單");
}

// ---------------------------------------------------------------------------
// AC-CC-13：「打開原對話」只放行合法的 Claude／Codex deep link
// ---------------------------------------------------------------------------

#[test]
fn only_well_formed_conversation_links_are_openable() {
    assert!(is_conversation_link("claude://code/continue?session=local_abc-123"));
    assert!(is_conversation_link("codex://threads/01a1_t-9"));
    for bad in [
        "claude://code/continue?session=local_abc&x=1",
        "claude://code/continue?session=abc",
        "codex://threads/../../etc",
        "codex://threads/",
        "file:///etc/passwd",
        "claude://settings",
    ] {
        assert!(!is_conversation_link(bad), "{bad}");
    }
}

// ---------------------------------------------------------------------------
// QA 修正：原對話 id 不能被當成 CLI 旗標注入（--last 會接到別的 thread）
// ---------------------------------------------------------------------------

#[test]
fn codex_thread_ids_must_be_uuids() {
    use kunsu_core::validate::{validate_create, RejectReason};
    let mut req: kunsu_core::types::CreateRequest = serde_json::from_value(serde_json::json!({
        "id": "pa-x", "title": "t", "question_line": "要不要？", "kind": "text", "tier": "t1",
        "default_action": {"on_expire": "noop_log"}, "source": {"agent": "codex", "cwd": "/w", "host": "codex_app"}
    }))
    .unwrap();
    req.source.conversation_id = Some("my-thread".into());
    assert!(
        matches!(validate_create(&req), Err(RejectReason::BadConversationIdentity { field: "source.conversation_id" })),
        "Codex 會把非 UUID 當 thread 名稱去找，可能接到別的對話"
    );
    req.source.conversation_id = Some("019a2b3c-4d5e-7f60-8a9b-0c1d2e3f4a5b".into());
    assert_eq!(validate_create(&req), Ok(()));
    // Claude 的 session id 不受這條限制（仍走一般身分規則）。
    req.source.agent = AgentKind::Claude;
    req.source.host = None;
    req.source.conversation_id = Some("conv-claude_1".into());
    assert_eq!(validate_create(&req), Ok(()));
}

#[test]
fn conversation_identity_rejects_flag_injection() {
    use kunsu_core::validate::{validate_create, RejectReason};
    let base = {
        let mut r: kunsu_core::types::CreateRequest = serde_json::from_value(serde_json::json!({
            "id": "pa-x", "title": "t", "question_line": "要不要？", "kind": "text", "tier": "t1",
            "default_action": {"on_expire": "noop_log"}, "source": {"agent": "codex", "cwd": "/w"}
        }))
        .unwrap();
        r.source.conversation_id = Some("019a3c2e-7b1d-7c11-9f00-1234abcd5678".into());
        r
    };
    assert_eq!(validate_create(&base), Ok(()));
    for bad in ["--last", "-x", "a b", "a;rm", &"x".repeat(129)] {
        let mut r = base.clone();
        r.source.conversation_id = Some(bad.to_string());
        assert!(
            matches!(validate_create(&r), Err(RejectReason::BadConversationIdentity { .. })),
            "{bad:?} 應被拒"
        );
    }
    let mut host = base.clone();
    host.source.host_session_id = Some("local_../../x".into());
    assert!(matches!(validate_create(&host), Err(RejectReason::BadConversationIdentity { .. })));
    let mut legacy = base.clone();
    legacy.source.conversation_id = None;
    legacy.wake_on_answer = Wake { mode: WakeMode::Resume, session_id: Some("--continue".into()), playbook: None };
    assert!(matches!(validate_create(&legacy), Err(RejectReason::BadConversationIdentity { .. })));
}
