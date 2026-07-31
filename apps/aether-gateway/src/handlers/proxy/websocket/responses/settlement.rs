//! 一次 provider attempt 的结算判定。
//!
//! 评审第 5 条：现状 `ResponsesWebSocketTurnOutcome` 一个枚举同时表达
//! 「供应商这一轮怎么结束的」和「内容有没有完整交付给客户端」，`finalize()`
//! 再用 `outcome.cancelled()` 一个布尔驱动 billing、candidate 状态和供应商效果。
//! 于是 provider 终态已经到达、只是最后一跳写客户端失败时，供应商事实会被
//! 覆盖掉。这里把两件事拆成正交事实，并把结算动作收进一张可逐行测试的表。

use super::turn::ResponsesWebSocketTurnOutcome;

/// 客户端取消/断开时对外记录的状态码。
const CLIENT_CANCELLED_STATUS_CODE: u16 = 499;

/// 流式超时状态码；现状只有它会额外投射 pool stream timeout 效果。
const STREAM_TIMEOUT_STATUS_CODE: u16 = 504;

/// provider 侧观察到的终态。
///
/// 形状刻意保持 transport 中立：HTTP 流式与 WS turn 的差异只在事实从哪来。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AttemptProviderOutcome {
    /// 观察到了供应商的终态事件。
    Terminal {
        status_code: u16,
        /// 供应商自己声明这一轮被取消（`response.cancelled`）。
        cancelled_by_provider: bool,
    },
    /// 供应商没能给出终态：断链、超时、gateway 侧失败。
    Aborted {
        status_code: u16,
        reason: &'static str,
        stream_timeout: bool,
    },
}

impl AttemptProviderOutcome {
    pub(super) const fn status_code(self) -> u16 {
        match self {
            Self::Terminal { status_code, .. } | Self::Aborted { status_code, .. } => status_code,
        }
    }

    pub(super) const fn cancelled_by_provider(self) -> bool {
        matches!(
            self,
            Self::Terminal {
                cancelled_by_provider: true,
                ..
            }
        )
    }

    pub(super) const fn stream_timeout(self) -> bool {
        matches!(self, Self::Aborted {
            stream_timeout: true,
            ..
        })
    }

    pub(super) const fn is_terminal(self) -> bool {
        matches!(self, Self::Terminal { .. })
    }
}

/// 这一个 attempt 的内容是否完整交付给了客户端。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AttemptClientDelivery {
    Complete,
    Aborted { reason: &'static str },
}

impl AttemptClientDelivery {
    pub(super) const fn aborted_reason(self) -> Option<&'static str> {
        match self {
            Self::Complete => None,
            Self::Aborted { reason } => Some(reason),
        }
    }

    pub(super) const fn is_aborted(self) -> bool {
        matches!(self, Self::Aborted { .. })
    }
}

/// 一次 attempt 结算时的两个正交事实。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct AttemptTerminalFacts {
    pub(super) provider: AttemptProviderOutcome,
    pub(super) delivery: AttemptClientDelivery,
}

impl AttemptTerminalFacts {
    /// 记入 usage / candidate / 效果的人类可读原因。
    pub(super) const fn reason(self) -> &'static str {
        if let Some(reason) = self.delivery.aborted_reason() {
            return reason;
        }
        match self.provider {
            AttemptProviderOutcome::Terminal {
                cancelled_by_provider: true,
                ..
            } => "provider cancelled the response",
            AttemptProviderOutcome::Terminal { .. } => {
                "provider returned a terminal response event"
            }
            AttemptProviderOutcome::Aborted { reason, .. } => reason,
        }
    }

    /// 供应商侧强制错误原因：只有「供应商没给出终态、且内容已完整交付客户端」
    /// 才算，用于给终态摘要补 `parser_error`。
    ///
    /// 客户端投递失败不是供应商的错误，所以那一侧返回 `None`——与现状
    /// `ResponsesWebSocketTurnOutcome::forced_error()` 对 `Cancelled` 返回
    /// `None` 一致。
    pub(super) const fn forced_error(self) -> Option<&'static str> {
        match (self.provider, self.delivery) {
            (
                AttemptProviderOutcome::Aborted { reason, .. },
                AttemptClientDelivery::Complete,
            ) => Some(reason),
            _ => None,
        }
    }
}

/// 这条 usage 记录是否计费。`Void` 等价于现状传给
/// `record_stream_terminal(.., cancelled = true)` 的那一侧。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AttemptBilling {
    Billed,
    Void,
}

impl AttemptBilling {
    pub(super) const fn is_void(self) -> bool {
        matches!(self, Self::Void)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AttemptCandidateStatus {
    Success,
    Failed,
    Cancelled,
}

/// candidate 行上记录的错误分类。
///
/// 与 [`AttemptCandidateStatus`] 刻意分开：`missing_terminal` 为真而记账层
/// 判定不算失败（report kind 不要求观察到终态事件）时，现状会写出
/// 「状态 Success + error_type=stream_missing_terminal_event」的组合，
/// 这里必须原样保留。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AttemptCandidateError {
    None,
    Cancelled,
    /// 供应商已经给出终态，但这一轮内容没能完整交付给客户端。账单照记，
    /// candidate 行上留下这条事实。
    ClientDeliveryFailed,
    MissingTerminal,
    TerminalError,
}

/// 一轮 turn 结束后要投射给供应商/密钥池的效果。
///
/// 每个分支都会释放 pool key lease：`ProviderFailure` 由 `PoolError` 释放，
/// `ProviderSuccess` 由 `PoolSuccessStream` 释放，其余情况直接释放。少一条
/// 分支就会把 lease 挂到 TTL 过期，等于短时间占死一把 key。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ResponsesWebSocketTurnEffect {
    /// 既不投射成功也不投射失败，只把 lease 还回去。
    ReleasePoolKeyLease,
    ProviderFailure,
    ProviderSuccess,
}

#[cfg(test)]
impl ResponsesWebSocketTurnEffect {
    /// 把「每个分支都必须释放 lease」这条不变量显式化，便于测试锁住
    /// 「没进任何分支导致 lease 泄漏」这类回归。
    const fn releases_pool_key_lease(self) -> bool {
        match self {
            Self::ReleasePoolKeyLease | Self::ProviderFailure | Self::ProviderSuccess => true,
        }
    }
}

/// 判定一轮 turn 结束后要投射的效果。
///
/// 关键分支是「记账层判成 failed，但这一轮没有投射供应商失败」：例如合法的
/// `response.incomplete`（写满 max_output_tokens）。共享 usage 判定目前仍会
/// 把这类终态记成失败，但供应商本身工作正常，既不该扣健康分，也不能因为落
/// 不到任何分支而漏掉 lease 释放。
pub(super) const fn classify_responses_websocket_turn_effect(
    cancelled: bool,
    projects_provider_failure: bool,
    failed: bool,
) -> ResponsesWebSocketTurnEffect {
    if cancelled {
        ResponsesWebSocketTurnEffect::ReleasePoolKeyLease
    } else if projects_provider_failure {
        ResponsesWebSocketTurnEffect::ProviderFailure
    } else if failed {
        ResponsesWebSocketTurnEffect::ReleasePoolKeyLease
    } else {
        ResponsesWebSocketTurnEffect::ProviderSuccess
    }
}

/// 把「结算触发信号」+「已观察到的 provider 终态」+「已记录的投递结果」映射成
/// 两个正交事实。
///
/// `ResponsesWebSocketTurnOutcome` 描述的是 relay loop 为什么现在结算这一
/// attempt，它对 provider 的信息量并不总是完整的：
///
/// - `ProviderTerminal` / `Failure` 本身就在描述供应商这一轮的结果，是权威的。
/// - `Cancelled` 只说明「我们为客户端或连接层面的原因停下了」，不携带任何
///   provider 信息。已经观察到的 provider 终态是独立事实，不能被它覆盖——
///   这正是评审第 5 条要求分开记录的那一处。
///
/// `recorded_delivery` 是 relay loop 明确记下的投递失败（写客户端 socket 失败）。
/// 它与结算信号推出的投递结果取「只要有一侧失败就是失败」，并优先保留明确记录
/// 的原因。
pub(super) fn attempt_facts_for_outcome(
    observed_provider_terminal: Option<AttemptProviderOutcome>,
    recorded_delivery: AttemptClientDelivery,
    settling: ResponsesWebSocketTurnOutcome,
) -> AttemptTerminalFacts {
    let facts = match settling {
        ResponsesWebSocketTurnOutcome::ProviderTerminal {
            status_code,
            cancelled,
        } => AttemptTerminalFacts {
            provider: AttemptProviderOutcome::Terminal {
                status_code,
                cancelled_by_provider: cancelled,
            },
            delivery: AttemptClientDelivery::Complete,
        },
        ResponsesWebSocketTurnOutcome::Failure {
            status_code,
            reason,
        } => AttemptTerminalFacts {
            provider: AttemptProviderOutcome::Aborted {
                status_code,
                reason,
                // 现状只有 504 一族（首事件/终态超时）会投射 pool stream timeout。
                stream_timeout: status_code == STREAM_TIMEOUT_STATUS_CODE,
            },
            delivery: AttemptClientDelivery::Complete,
        },
        ResponsesWebSocketTurnOutcome::Cancelled { reason } => AttemptTerminalFacts {
            provider: observed_provider_terminal.unwrap_or(AttemptProviderOutcome::Aborted {
                status_code: CLIENT_CANCELLED_STATUS_CODE,
                reason,
                stream_timeout: false,
            }),
            delivery: AttemptClientDelivery::Aborted { reason },
        },
    };
    AttemptTerminalFacts {
        delivery: match recorded_delivery {
            AttemptClientDelivery::Aborted { .. } => recorded_delivery,
            AttemptClientDelivery::Complete => facts.delivery,
        },
        ..facts
    }
}

/// 客户端投递失败时应该用哪个结算信号。
///
/// provider 终态已经到达就用那条终态：它是权威的 provider 事实，绝不能被
/// `client_disconnected()` 覆盖掉——那正是把已完成响应记成 void billing 的原因。
/// 供应商还没给出终态时，客户端断开才是这一 attempt 的全部结论。
pub(super) fn settle_signal_for_client_delivery_failure(
    terminal_outcome: Option<ResponsesWebSocketTurnOutcome>,
) -> ResponsesWebSocketTurnOutcome {
    terminal_outcome.unwrap_or_else(ResponsesWebSocketTurnOutcome::client_disconnected)
}

/// 这一个 attempt 的账单是否作废。
///
/// 只有两种情况作废：供应商自己声明取消，或者供应商根本没给出终态而客户端
/// 又已经走了。**供应商已经给出终态时，客户端最后一跳投递失败不作废账单**：
/// 供应商已经完成推理并消耗了 token，客户端还能用 `previous_response_id`
/// 续取这条响应，把成本记成 0 等于让上游账单凭空消失。
pub(super) const fn attempt_billing_is_void(facts: AttemptTerminalFacts) -> bool {
    facts.provider.cancelled_by_provider()
        || (facts.delivery.is_aborted() && !facts.provider.is_terminal())
}

/// attempt 对外记录的状态码。
///
/// 状态码现在纯粹是 provider 事实：客户端投递失败不再把一条已经拿到 200
/// 终态的记录改写成 499。作废分支的 provider 状态码本身就是 499
/// （`response.cancelled` 映射 499，`Cancelled` 信号的兜底也是 499），
/// 所以这些行的取值不变。
pub(super) const fn attempt_status_code(facts: AttemptTerminalFacts) -> u16 {
    facts.provider.status_code()
}

/// 结算判定的输入：两个正交事实 + 记账层对这条 report 的判定 + 终态摘要事实。
#[derive(Debug, Clone, Copy)]
pub(super) struct AttemptSettlementInputs {
    pub(super) facts: AttemptTerminalFacts,
    /// `aether_usage_runtime::stream_report_represents_failure(payload)` 的结果。
    pub(super) report_represents_failure: bool,
    /// 终态摘要里是否观察到了 finish。
    pub(super) observed_finish: bool,
    /// 终态摘要里是否带解析错误。
    pub(super) has_parser_error: bool,
}

/// 结算动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct AttemptSettlement {
    pub(super) status_code: u16,
    pub(super) billing: AttemptBilling,
    pub(super) candidate_status: AttemptCandidateStatus,
    pub(super) candidate_error: AttemptCandidateError,
    pub(super) provider_effect: ResponsesWebSocketTurnEffect,
    pub(super) submit_execution_report: bool,
}

/// 由两个正交事实推出结算动作。唯一的判定入口，表驱动测试逐行锁死。
///
/// provider 终态已到达时，客户端投递失败只影响 candidate 的错误分类，不再作废
/// 账单、不再把状态码改成 499、也不再跳过供应商效果和 execution report。
pub(super) const fn classify_attempt_settlement(
    inputs: AttemptSettlementInputs,
) -> AttemptSettlement {
    let AttemptSettlementInputs {
        facts,
        report_represents_failure,
        observed_finish,
        has_parser_error,
    } = inputs;

    let void = attempt_billing_is_void(facts);
    let status_code = attempt_status_code(facts);
    let failed = !void && report_represents_failure;
    let missing_terminal = !void && !observed_finish;
    let projects_provider_failure = !void
        && (status_code >= 400
            || facts.forced_error().is_some()
            || has_parser_error
            || missing_terminal);

    let candidate_status = if void {
        AttemptCandidateStatus::Cancelled
    } else if failed {
        AttemptCandidateStatus::Failed
    } else {
        AttemptCandidateStatus::Success
    };
    // 投递失败排在供应商侧分类之前：这条记录之所以特别，正是因为内容没送到
    // 客户端手上。供应商侧的判定仍然通过 candidate_status 和 error_message
    // 保留下来。
    let candidate_error = if void {
        AttemptCandidateError::Cancelled
    } else if facts.delivery.is_aborted() {
        AttemptCandidateError::ClientDeliveryFailed
    } else if missing_terminal {
        AttemptCandidateError::MissingTerminal
    } else if failed {
        AttemptCandidateError::TerminalError
    } else {
        AttemptCandidateError::None
    };

    AttemptSettlement {
        status_code,
        billing: if void {
            AttemptBilling::Void
        } else {
            AttemptBilling::Billed
        },
        candidate_status,
        candidate_error,
        provider_effect: classify_responses_websocket_turn_effect(
            void,
            projects_provider_failure,
            failed,
        ),
        submit_execution_report: !void,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        attempt_facts_for_outcome, classify_attempt_settlement,
        classify_responses_websocket_turn_effect, settle_signal_for_client_delivery_failure,
        AttemptBilling, AttemptCandidateError, AttemptCandidateStatus, AttemptClientDelivery,
        AttemptProviderOutcome, AttemptSettlement, AttemptSettlementInputs, AttemptTerminalFacts,
        ResponsesWebSocketTurnEffect,
    };
    use super::super::turn::ResponsesWebSocketTurnOutcome;

    fn settle(
        provider: AttemptProviderOutcome,
        delivery: AttemptClientDelivery,
        report_represents_failure: bool,
        observed_finish: bool,
        has_parser_error: bool,
    ) -> AttemptSettlement {
        classify_attempt_settlement(AttemptSettlementInputs {
            facts: AttemptTerminalFacts { provider, delivery },
            report_represents_failure,
            observed_finish,
            has_parser_error,
        })
    }

    const fn terminal(status_code: u16) -> AttemptProviderOutcome {
        AttemptProviderOutcome::Terminal {
            status_code,
            cancelled_by_provider: false,
        }
    }

    const fn provider_cancelled() -> AttemptProviderOutcome {
        AttemptProviderOutcome::Terminal {
            status_code: 499,
            cancelled_by_provider: true,
        }
    }

    const fn aborted(status_code: u16, reason: &'static str) -> AttemptProviderOutcome {
        AttemptProviderOutcome::Aborted {
            status_code,
            reason,
            stream_timeout: status_code == 504,
        }
    }

    /// §1.6 现状 outcome → 双事实映射表，逐行。
    #[test]
    fn every_settle_signal_maps_to_a_provider_outcome_and_a_client_delivery() {
        assert_eq!(
            attempt_facts_for_outcome(
                None,
                AttemptClientDelivery::Complete,
                ResponsesWebSocketTurnOutcome::ProviderTerminal {
                    status_code: 200,
                    cancelled: false,
                },
            ),
            AttemptTerminalFacts {
                provider: terminal(200),
                delivery: AttemptClientDelivery::Complete,
            }
        );
        assert_eq!(
            attempt_facts_for_outcome(
                None,
                AttemptClientDelivery::Complete,
                ResponsesWebSocketTurnOutcome::ProviderTerminal {
                    status_code: 499,
                    cancelled: true,
                },
            ),
            AttemptTerminalFacts {
                provider: provider_cancelled(),
                delivery: AttemptClientDelivery::Complete,
            }
        );
        assert_eq!(
            attempt_facts_for_outcome(None, AttemptClientDelivery::Complete, ResponsesWebSocketTurnOutcome::upstream_closed()),
            AttemptTerminalFacts {
                provider: aborted(
                    502,
                    "upstream WebSocket closed before provider terminal event"
                ),
                delivery: AttemptClientDelivery::Complete,
            }
        );
        assert_eq!(
            attempt_facts_for_outcome(None, AttemptClientDelivery::Complete, ResponsesWebSocketTurnOutcome::client_disconnected()),
            AttemptTerminalFacts {
                provider: aborted(499, "client disconnected before provider terminal event"),
                delivery: AttemptClientDelivery::Aborted {
                    reason: "client disconnected before provider terminal event",
                },
            }
        );

        // 超时一族必须保留 stream_timeout 标记，否则 pool stream timeout 效果丢失。
        let first_event_timeout =
            attempt_facts_for_outcome(None, AttemptClientDelivery::Complete, ResponsesWebSocketTurnOutcome::first_event_timeout());
        assert!(first_event_timeout.provider.stream_timeout());
        let terminal_timeout =
            attempt_facts_for_outcome(None, AttemptClientDelivery::Complete, ResponsesWebSocketTurnOutcome::terminal_timeout());
        assert!(terminal_timeout.provider.stream_timeout());
        // 非 504 的失败不得被当成流式超时。
        assert!(!attempt_facts_for_outcome(
            None,
            AttemptClientDelivery::Complete,
            ResponsesWebSocketTurnOutcome::upstream_closed()
        )
        .provider
        .stream_timeout());
        // provider 终态即使状态码是 504 也不投射 stream timeout：现状
        // `stream_timeout()` 只匹配 Failure 分支。
        assert!(!attempt_facts_for_outcome(
            None,
            AttemptClientDelivery::Complete,
            ResponsesWebSocketTurnOutcome::ProviderTerminal {
                status_code: 504,
                cancelled: false,
            },
        )
        .provider
        .stream_timeout());
    }

    /// `Cancelled` 不携带 provider 信息，已观察到的终态不能被它覆盖；
    /// `ProviderTerminal` / `Failure` 本身就是权威的 provider 事实。
    #[test]
    fn an_observed_provider_terminal_survives_a_client_side_cancellation() {
        let observed = terminal(200);

        let facts = attempt_facts_for_outcome(
            Some(observed),
            AttemptClientDelivery::Complete,
            ResponsesWebSocketTurnOutcome::client_disconnected(),
        );
        assert_eq!(facts.provider, observed);
        assert_eq!(
            facts.delivery,
            AttemptClientDelivery::Aborted {
                reason: "client disconnected before provider terminal event",
            }
        );

        // 权威信号不被已记录事实改写。
        let facts = attempt_facts_for_outcome(
            Some(observed),
            AttemptClientDelivery::Complete,
            ResponsesWebSocketTurnOutcome::upstream_closed(),
        );
        assert_eq!(
            facts.provider,
            aborted(502, "upstream WebSocket closed before provider terminal event")
        );
        assert_eq!(facts.delivery, AttemptClientDelivery::Complete);
    }

    /// 投递失败时 `forced_error` 必须为 `None`：客户端走了不是供应商的错误。
    /// 与现状 `ResponsesWebSocketTurnOutcome::forced_error()` 对 `Cancelled`
    /// 返回 `None` 一致。
    #[test]
    fn only_a_provider_abort_with_complete_delivery_is_a_forced_error() {
        assert_eq!(
            AttemptTerminalFacts {
                provider: aborted(502, "upstream failed"),
                delivery: AttemptClientDelivery::Complete,
            }
            .forced_error(),
            Some("upstream failed")
        );
        assert_eq!(
            AttemptTerminalFacts {
                provider: aborted(499, "client went away"),
                delivery: AttemptClientDelivery::Aborted {
                    reason: "client went away"
                },
            }
            .forced_error(),
            None
        );
        assert_eq!(
            AttemptTerminalFacts {
                provider: terminal(200),
                delivery: AttemptClientDelivery::Complete,
            }
            .forced_error(),
            None
        );
    }

    #[test]
    fn the_recorded_reason_prefers_the_client_delivery_failure() {
        assert_eq!(
            AttemptTerminalFacts {
                provider: terminal(200),
                delivery: AttemptClientDelivery::Aborted {
                    reason: "client went away"
                },
            }
            .reason(),
            "client went away"
        );
        assert_eq!(
            AttemptTerminalFacts {
                provider: provider_cancelled(),
                delivery: AttemptClientDelivery::Complete,
            }
            .reason(),
            "provider cancelled the response"
        );
        assert_eq!(
            AttemptTerminalFacts {
                provider: terminal(200),
                delivery: AttemptClientDelivery::Complete,
            }
            .reason(),
            "provider returned a terminal response event"
        );
        assert_eq!(
            AttemptTerminalFacts {
                provider: aborted(502, "upstream failed"),
                delivery: AttemptClientDelivery::Complete,
            }
            .reason(),
            "upstream failed"
        );
    }

    /// §1.6 结算表，逐行。
    #[test]
    fn settlement_table_row_provider_cancelled_is_void_regardless_of_delivery() {
        for delivery in [
            AttemptClientDelivery::Complete,
            AttemptClientDelivery::Aborted { reason: "gone" },
        ] {
            for report_represents_failure in [false, true] {
                let settlement =
                    settle(provider_cancelled(), delivery, report_represents_failure, true, false);
                assert_eq!(
                    settlement,
                    AttemptSettlement {
                        status_code: 499,
                        billing: AttemptBilling::Void,
                        candidate_status: AttemptCandidateStatus::Cancelled,
                        candidate_error: AttemptCandidateError::Cancelled,
                        provider_effect: ResponsesWebSocketTurnEffect::ReleasePoolKeyLease,
                        submit_execution_report: false,
                    },
                    "delivery={delivery:?} report_failure={report_represents_failure}"
                );
            }
        }
    }

    #[test]
    fn settlement_table_row_aborted_provider_with_aborted_delivery_is_void() {
        let settlement = settle(
            aborted(499, "client went away"),
            AttemptClientDelivery::Aborted {
                reason: "client went away",
            },
            true,
            false,
            false,
        );
        assert_eq!(
            settlement,
            AttemptSettlement {
                status_code: 499,
                billing: AttemptBilling::Void,
                candidate_status: AttemptCandidateStatus::Cancelled,
                candidate_error: AttemptCandidateError::Cancelled,
                provider_effect: ResponsesWebSocketTurnEffect::ReleasePoolKeyLease,
                submit_execution_report: false,
            }
        );
    }

    #[test]
    fn settlement_table_row_clean_provider_terminal_is_a_billed_success() {
        let settlement = settle(terminal(200), AttemptClientDelivery::Complete, false, true, false);
        assert_eq!(
            settlement,
            AttemptSettlement {
                status_code: 200,
                billing: AttemptBilling::Billed,
                candidate_status: AttemptCandidateStatus::Success,
                candidate_error: AttemptCandidateError::None,
                provider_effect: ResponsesWebSocketTurnEffect::ProviderSuccess,
                submit_execution_report: true,
            }
        );
    }

    /// 合法 `response.incomplete`：记账层判失败，但供应商工作正常，
    /// 不扣健康分、只释放 lease，并且账单照记。
    #[test]
    fn settlement_table_row_legitimate_incomplete_is_billed_without_provider_failure() {
        let settlement = settle(terminal(200), AttemptClientDelivery::Complete, true, true, false);
        assert_eq!(
            settlement,
            AttemptSettlement {
                status_code: 200,
                billing: AttemptBilling::Billed,
                candidate_status: AttemptCandidateStatus::Failed,
                candidate_error: AttemptCandidateError::TerminalError,
                provider_effect: ResponsesWebSocketTurnEffect::ReleasePoolKeyLease,
                submit_execution_report: true,
            }
        );
    }

    #[test]
    fn settlement_table_row_provider_abort_projects_a_provider_failure() {
        let settlement = settle(
            aborted(502, "upstream WebSocket closed before provider terminal event"),
            AttemptClientDelivery::Complete,
            true,
            false,
            false,
        );
        assert_eq!(
            settlement,
            AttemptSettlement {
                status_code: 502,
                billing: AttemptBilling::Billed,
                candidate_status: AttemptCandidateStatus::Failed,
                candidate_error: AttemptCandidateError::MissingTerminal,
                provider_effect: ResponsesWebSocketTurnEffect::ProviderFailure,
                submit_execution_report: true,
            }
        );
    }

    /// ✱ 修正后的那一行：provider 终态已到达，客户端投递失败不再作废账单。
    ///
    /// 供应商已经完成推理并消耗 token，客户端还能用 `previous_response_id`
    /// 续取这条响应；把成本记成 0 等于让上游账单凭空消失。投递失败作为独立
    /// 事实留在 candidate 的错误分类里。
    #[test]
    fn settlement_table_row_client_delivery_failure_keeps_a_reached_terminal_billed() {
        let settlement = settle(
            terminal(200),
            AttemptClientDelivery::Aborted {
                reason: "gateway could not relay the provider event to the client",
            },
            false,
            true,
            false,
        );
        assert_eq!(
            settlement,
            AttemptSettlement {
                status_code: 200,
                billing: AttemptBilling::Billed,
                candidate_status: AttemptCandidateStatus::Success,
                candidate_error: AttemptCandidateError::ClientDeliveryFailed,
                provider_effect: ResponsesWebSocketTurnEffect::ProviderSuccess,
                submit_execution_report: true,
            }
        );

        // 除了 candidate 的错误分类，其余判定与「投递成功」完全一致。
        let delivered = settle(terminal(200), AttemptClientDelivery::Complete, false, true, false);
        assert_eq!(settlement.status_code, delivered.status_code);
        assert_eq!(settlement.billing, delivered.billing);
        assert_eq!(settlement.candidate_status, delivered.candidate_status);
        assert_eq!(settlement.provider_effect, delivered.provider_effect);
        assert_eq!(
            settlement.submit_execution_report,
            delivered.submit_execution_report
        );
        assert_ne!(settlement.candidate_error, delivered.candidate_error);
    }

    /// 供应商还没给出终态时，客户端投递失败仍然作废账单：这一轮确实没有产出。
    #[test]
    fn a_delivery_failure_without_a_provider_terminal_still_voids_the_bill() {
        let settlement = settle(
            aborted(499, "client went away"),
            AttemptClientDelivery::Aborted {
                reason: "client went away",
            },
            false,
            false,
            false,
        );
        assert_eq!(settlement.status_code, 499);
        assert_eq!(settlement.billing, AttemptBilling::Void);
        assert_eq!(
            settlement.candidate_status,
            AttemptCandidateStatus::Cancelled
        );
        assert_eq!(settlement.candidate_error, AttemptCandidateError::Cancelled);
        assert!(!settlement.submit_execution_report);
    }

    /// 供应商自己声明取消时，即使内容送到了客户端也不计费。
    #[test]
    fn a_provider_declared_cancellation_is_void_even_when_delivered() {
        let settlement =
            settle(provider_cancelled(), AttemptClientDelivery::Complete, false, true, false);
        assert_eq!(settlement.billing, AttemptBilling::Void);
        assert_eq!(settlement.candidate_error, AttemptCandidateError::Cancelled);
    }

    /// 结算信号的选择：provider 终态已到达就用它，否则才是 client 断开。
    /// 这是修正的核心——旧实现无条件用 client_disconnected() 覆盖，
    /// 于是已完成的响应被记成 void billing。
    #[test]
    fn a_reached_terminal_is_the_settle_signal_for_a_delivery_failure() {
        let terminal_outcome = ResponsesWebSocketTurnOutcome::ProviderTerminal {
            status_code: 200,
            cancelled: false,
        };
        assert_eq!(
            settle_signal_for_client_delivery_failure(Some(terminal_outcome)),
            terminal_outcome
        );
        assert_eq!(
            settle_signal_for_client_delivery_failure(None),
            ResponsesWebSocketTurnOutcome::client_disconnected()
        );
    }

    /// 明确记录的投递失败不会被结算信号推出的「投递成功」覆盖。
    #[test]
    fn a_recorded_delivery_failure_survives_a_provider_terminal_settle_signal() {
        let facts = attempt_facts_for_outcome(
            Some(terminal(200)),
            AttemptClientDelivery::Aborted {
                reason: "write failed"
            },
            ResponsesWebSocketTurnOutcome::ProviderTerminal {
                status_code: 200,
                cancelled: false,
            },
        );
        assert_eq!(facts.provider, terminal(200));
        assert_eq!(
            facts.delivery,
            AttemptClientDelivery::Aborted {
                reason: "write failed"
            }
        );
        // 投递失败不是供应商的错误，摘要不该因此补 parser_error。
        assert_eq!(facts.forced_error(), None);
    }

    /// 记账层判 Success，但摘要没观察到 finish：现状会写出
    /// 「candidate=Success + error_type=stream_missing_terminal_event」，
    /// 所以状态与错误分类必须各自独立。
    #[test]
    fn a_missing_terminal_can_coexist_with_a_successful_candidate_status() {
        let settlement = settle(terminal(200), AttemptClientDelivery::Complete, false, false, false);
        assert_eq!(settlement.candidate_status, AttemptCandidateStatus::Success);
        assert_eq!(
            settlement.candidate_error,
            AttemptCandidateError::MissingTerminal
        );
        // missing_terminal 仍然要投射供应商失败。
        assert_eq!(
            settlement.provider_effect,
            ResponsesWebSocketTurnEffect::ProviderFailure
        );
    }

    #[test]
    fn a_parser_error_projects_a_provider_failure_even_on_a_clean_status_code() {
        let settlement = settle(terminal(200), AttemptClientDelivery::Complete, true, true, true);
        assert_eq!(
            settlement.provider_effect,
            ResponsesWebSocketTurnEffect::ProviderFailure
        );
        assert_eq!(settlement.billing, AttemptBilling::Billed);
    }

    #[test]
    fn a_legitimate_incomplete_still_releases_the_pool_key_lease() {
        // 共享 usage 判定目前仍把 response.incomplete 记成终态失败，于是会出现
        // failed=true 而 projects_provider_failure=false 的组合。这种组合必须
        // 明确落到「只释放 lease」的分支，否则 lease 会挂到 TTL 过期。
        let effect = classify_responses_websocket_turn_effect(false, false, true);

        assert_eq!(effect, ResponsesWebSocketTurnEffect::ReleasePoolKeyLease);
        assert!(effect.releases_pool_key_lease());
    }

    #[test]
    fn every_turn_effect_releases_the_pool_key_lease() {
        for (cancelled, projects_provider_failure, failed, expected) in [
            (
                true,
                false,
                false,
                ResponsesWebSocketTurnEffect::ReleasePoolKeyLease,
            ),
            (
                true,
                true,
                true,
                ResponsesWebSocketTurnEffect::ReleasePoolKeyLease,
            ),
            (
                false,
                true,
                true,
                ResponsesWebSocketTurnEffect::ProviderFailure,
            ),
            (
                false,
                false,
                true,
                ResponsesWebSocketTurnEffect::ReleasePoolKeyLease,
            ),
            (
                false,
                false,
                false,
                ResponsesWebSocketTurnEffect::ProviderSuccess,
            ),
        ] {
            let effect = classify_responses_websocket_turn_effect(
                cancelled,
                projects_provider_failure,
                failed,
            );
            assert_eq!(
                effect, expected,
                "cancelled={cancelled} projects_provider_failure={projects_provider_failure} failed={failed}"
            );
            assert!(
                effect.releases_pool_key_lease(),
                "every effect branch must release the pool key lease"
            );
        }
    }

    /// 每一个结算分支都必须释放 lease：这条不变量跨越整张结算表。
    #[test]
    fn every_settlement_branch_releases_the_pool_key_lease() {
        let providers = [
            terminal(200),
            terminal(429),
            provider_cancelled(),
            aborted(502, "upstream failed"),
            aborted(504, "timed out"),
        ];
        let deliveries = [
            AttemptClientDelivery::Complete,
            AttemptClientDelivery::Aborted { reason: "gone" },
        ];
        for provider in providers {
            for delivery in deliveries {
                for report_represents_failure in [false, true] {
                    for observed_finish in [false, true] {
                        for has_parser_error in [false, true] {
                            let settlement = settle(
                                provider,
                                delivery,
                                report_represents_failure,
                                observed_finish,
                                has_parser_error,
                            );
                            assert!(
                                settlement.provider_effect.releases_pool_key_lease(),
                                "provider={provider:?} delivery={delivery:?}"
                            );
                            // 作废账单的分支一律不提交 execution report。
                            assert_eq!(
                                settlement.submit_execution_report,
                                !settlement.billing.is_void(),
                                "provider={provider:?} delivery={delivery:?}"
                            );
                        }
                    }
                }
            }
        }
    }
}
