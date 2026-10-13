//! CONTRACT §3 狀態機純函數。
//!
//! 每一題非 internal、非終態問題都有獨立的牆鐘週期：12 分鐘警告、15 分鐘
//! 強制詢問。guards 只會延後蓋板，不會重設週期；未回答 dismiss 會開始完整新週期。

use chrono::Duration;
use serde::{Deserialize, Serialize};

use crate::guards::GuardState;
use crate::policy::Policy;
use crate::time::Ts;
use crate::types::{Channel, DefaultAction, OnExpire, Question, State, Tier, WakeMode};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    Tick,
    Answered {
        channel: Channel,
    },
    /// 關閉未回答的詢問介面；開始下一個完整週期。
    Dismiss,
    /// 舊 API 相容別名，語義與 Dismiss 完全一致。
    Snooze,
    Cancel,
    Supersede,
    DeliveryComplete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotifyKind {
    NewQuestion,
    /// 距強制詢問只剩 policy.force.warn_before_min。
    WarnCountdown,
    ForcedOverlay,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscordReason {
    /// 舊 ledger 相容值；新流程不再因 dismiss 降級。
    SnoozeCapDowngrade,
    MoneyExpired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalKind {
    Answered,
    Expired,
    Cancelled,
    Superseded,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "effect", rename_all = "snake_case")]
pub enum Effect {
    Notify {
        kind: NotifyKind,
    },
    Discord {
        reason: DiscordReason,
    },
    RunDefault {
        action: DefaultAction,
    },
    WriteLedger {
        terminal: TerminalKind,
    },
    SpawnReactor,
    GenerateActivationQuestion {
        kongko_ref: String,
    },
    ScheduleNext {
        at: Ts,
    },
    /// Legacy telemetry counter; unlike the old flow it has no cap or downgrade meaning.
    IncrementSnooze,
    IncrementCycle,
    RestartForceCycle {
        cycle_started_at: Ts,
        warn_at: Ts,
        force_at: Ts,
    },
    /// Legacy compatibility effects; the new force-cycle path never emits these.
    DowngradeTier {
        to: Tier,
    },
    MarkForcedBudgetUsed,
    HideOverlay,
}

fn retry_at(now: Ts, policy: &Policy) -> Ts {
    now + Duration::seconds(policy.force.deferred_retry_s as i64)
}

fn cycle_times(started_at: Ts, policy: &Policy) -> (Ts, Ts) {
    let force_at = started_at + Duration::minutes(policy.force.cycle_min as i64);
    let warn_at = force_at - Duration::minutes(policy.force.warn_before_min as i64);
    (warn_at, force_at)
}

/// 下一個應持久化的轉移時刻。牆鐘欄位是正本，server restart/sleep 不重算。
pub fn next_due(q: &Question, now: Ts, policy: &Policy) -> Ts {
    // 到期參數用建題時固定在題目上的值（INV-26）；舊題沒有快照才用目前 policy。
    let mut candidates = vec![q.created_at + Duration::days(q.max_pending_days(policy) as i64)];
    if let Some(deadline) = q.deadline_at {
        candidates.push(deadline);
    }
    if q.tier == Tier::T1 {
        candidates.push(q.created_at + Duration::hours(q.expiry_hours(policy) as i64));
    }
    if !q.internal {
        match q.state {
            State::Created | State::Pending | State::Snoozed => {
                if let Some(at) = q.warn_at {
                    candidates.push(at);
                }
            }
            State::Warned => {
                if let Some(at) = q.force_at {
                    candidates.push(at);
                }
            }
            State::Forced => candidates.push(retry_at(now, policy)),
            _ => {}
        }
    }
    let earliest = candidates.into_iter().min().expect("至少有 max-age 排程");
    if earliest <= now {
        retry_at(now, policy)
    } else {
        earliest
    }
}

fn expire_effects(q: &Question) -> Vec<Effect> {
    let mut effects = Vec::new();
    if q.is_money_op() {
        if q.default_action.on_expire != OnExpire::PickOption {
            effects.push(Effect::RunDefault {
                action: q.default_action.clone(),
            });
        }
        effects.push(Effect::Discord {
            reason: DiscordReason::MoneyExpired,
        });
    } else {
        effects.push(Effect::RunDefault {
            action: q.default_action.clone(),
        });
    }
    effects.push(Effect::WriteLedger {
        terminal: TerminalKind::Expired,
    });
    effects
}

fn tick(q: &Question, now: Ts, policy: &Policy, guards: &GuardState) -> (State, Vec<Effect>) {
    if q.deadline_at.is_some_and(|deadline| now >= deadline) {
        return (State::Expired, expire_effects(q));
    }
    let age = now - q.created_at;
    // 最長等待天數（INV-14）與一般題到期時間都用建題時固定的值（INV-26）：之後改設定不會讓已在等的題提早到期。
    if age >= Duration::days(q.max_pending_days(policy) as i64) {
        return if q.tier == Tier::T2 {
            (
                State::Superseded,
                vec![Effect::WriteLedger {
                    terminal: TerminalKind::Superseded,
                }],
            )
        } else {
            (State::Expired, expire_effects(q))
        };
    }
    if q.tier == Tier::T1 && age >= Duration::hours(q.expiry_hours(policy) as i64) {
        return (State::Expired, expire_effects(q));
    }

    match q.state {
        State::Created => (
            State::Pending,
            vec![
                Effect::Notify {
                    kind: NotifyKind::NewQuestion,
                },
                Effect::ScheduleNext {
                    at: next_due(q, now, policy),
                },
            ],
        ),
        State::Pending | State::Snoozed => {
            if !q.internal && q.warn_at.is_some_and(|warn_at| now >= warn_at) {
                let next = q.force_at.unwrap_or_else(|| cycle_times(now, policy).1);
                (
                    State::Warned,
                    vec![
                        Effect::Notify {
                            kind: NotifyKind::WarnCountdown,
                        },
                        Effect::ScheduleNext {
                            at: if next <= now {
                                retry_at(now, policy)
                            } else {
                                next
                            },
                        },
                    ],
                )
            } else {
                (
                    q.state,
                    vec![Effect::ScheduleNext {
                        at: next_due(q, now, policy),
                    }],
                )
            }
        }
        State::Warned => {
            let due = q.force_at.is_some_and(|force_at| now >= force_at);
            if !due {
                return (
                    State::Warned,
                    vec![Effect::ScheduleNext {
                        at: next_due(q, now, policy),
                    }],
                );
            }
            if q.internal || guards.blocks_force(now, policy) {
                (
                    State::Warned,
                    vec![Effect::ScheduleNext {
                        at: retry_at(now, policy),
                    }],
                )
            } else {
                (
                    State::Forced,
                    vec![
                        Effect::Notify {
                            kind: NotifyKind::ForcedOverlay,
                        },
                        Effect::ScheduleNext {
                            at: retry_at(now, policy),
                        },
                    ],
                )
            }
        }
        State::Forced => {
            if guards.blocks_force(now, policy) {
                (
                    State::Warned,
                    vec![
                        Effect::HideOverlay,
                        Effect::ScheduleNext {
                            at: retry_at(now, policy),
                        },
                    ],
                )
            } else {
                (
                    State::Forced,
                    vec![Effect::ScheduleNext {
                        at: retry_at(now, policy),
                    }],
                )
            }
        }
        state => (state, vec![]),
    }
}

fn dismiss(q: &Question, now: Ts, policy: &Policy) -> (State, Vec<Effect>) {
    if q.internal || matches!(q.state, State::Created) {
        return (q.state, vec![]);
    }
    let (warn_at, force_at) = cycle_times(now, policy);
    (
        State::Pending,
        vec![
            Effect::IncrementSnooze,
            Effect::IncrementCycle,
            Effect::RestartForceCycle {
                cycle_started_at: now,
                warn_at,
                force_at,
            },
            Effect::ScheduleNext { at: warn_at },
        ],
    )
}

/// 永不 I/O、永不讀時鐘；所有牆鐘時刻由呼叫端傳入或題目持久化欄位提供。
pub fn transition(
    q: &Question,
    event: Event,
    now: Ts,
    policy: &Policy,
    guards: &GuardState,
) -> (State, Vec<Effect>) {
    let state = q.state;
    if state.is_terminal() {
        return match event {
            // 只有已回答的題收割後標 routed；過期、取消、被取代保持原終態（歷史與督導分得出來，AC-RC-09）。
            Event::DeliveryComplete if state == State::Answered => (State::Routed, vec![]),
            _ => (state, vec![]),
        };
    }
    match event {
        Event::Answered { channel } => {
            let mut effects = vec![Effect::WriteLedger {
                terminal: TerminalKind::Answered,
            }];
            // 錢題不論 wake 都進 reactor：使用者批准的錢鏈一定會跑；wake=none 只表示不喚醒 agent（CONTRACT §7）。
            if q.wake_on_answer.mode != WakeMode::None || q.is_money_op() {
                effects.push(Effect::SpawnReactor);
            }
            if channel == Channel::Kongko
                && let Some(kongko_ref) = &q.kongko_ref
            {
                effects.push(Effect::GenerateActivationQuestion {
                    kongko_ref: kongko_ref.clone(),
                });
            }
            (State::Answered, effects)
        }
        Event::Dismiss | Event::Snooze => dismiss(q, now, policy),
        Event::Cancel => (
            State::Cancelled,
            vec![Effect::WriteLedger {
                terminal: TerminalKind::Cancelled,
            }],
        ),
        Event::Supersede => (
            State::Superseded,
            vec![Effect::WriteLedger {
                terminal: TerminalKind::Superseded,
            }],
        ),
        Event::Tick => tick(q, now, policy, guards),
        Event::DeliveryComplete => (state, vec![]),
    }
}
