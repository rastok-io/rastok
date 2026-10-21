use super::*;
use crate::connector::stdlib::FILE_CONNECTOR_ID;
use crate::connector::stdlib::HTTP_CLIENT_CONNECTOR_ID;
use crate::connector::stdlib::HTTP_SERVER_CONNECTOR_ID;
use crate::connector::stdlib::PROCESS_CONNECTOR_ID;
use crate::connector::stdlib::WEBSOCKET_CLIENT_CONNECTOR_ID;
use crate::connector::stdlib::WEBSOCKET_SERVER_CONNECTOR_ID;
use crate::connector::stdlib::file_exit_type;
use crate::connector::stdlib::http_client_exit_type;
use crate::connector::stdlib::process_exit_type;
use crate::datatype::{DataType, TypeError};
use alloc::string::ToString;
use alloc::vec::Vec;

fn contract(id: &str, mode: Option<&str>) -> CallSemantics {
    outbound_call_semantics(
        &ContractId::parse(id).unwrap(),
        ConnectorDirection::Outbound,
        ExchangeKind::Call,
        mode,
    )
    .expect("a standard outbound Call profile")
}

#[test]
fn retry_safety_is_derived_for_every_terminal_without_an_effect_class() {
    for (id, mode) in [
        (PROCESS_CONNECTOR_ID, Some("job")),
        (FILE_CONNECTOR_ID, Some("read")),
        (FILE_CONNECTOR_ID, Some("replace")),
        (HTTP_CLIENT_CONNECTOR_ID, None),
    ] {
        let profile = contract(id, mode);
        for (case, visibility) in profile.terminal_cases() {
            for idempotent in [false, true] {
                let result = profile.validate_retry_cases(&[(*case).to_string()], idempotent);
                assert_eq!(
                    result.is_ok(),
                    idempotent || *visibility == TerminalVisibility::BeforeEffect,
                    "{id}: {case}, idempotent={idempotent}"
                );
                if let Err(error) = result {
                    assert_eq!(
                        error,
                        RetryCaseError::Unsafe {
                            case: (*case).to_string(),
                            visibility: *visibility
                        }
                    );
                }
            }
        }
        assert_eq!(profile.validate_retry_cases(&[], false), Ok(()));
    }
}

#[test]
fn retry_never_trusts_unknown_or_noncanonical_case_sets() {
    let profile = contract(PROCESS_CONNECTOR_ID, Some("job"));
    for cases in [
        ["SpawnFailed", "SpawnFailed"],
        ["TimeoutExpired", "SpawnFailed"],
    ] {
        assert_eq!(
            profile.validate_retry_cases(&cases.map(String::from), true),
            Err(RetryCaseError::Noncanonical)
        );
    }
    assert_eq!(
        profile.validate_retry_cases(&["RetryExpired".into()], true),
        Err(RetryCaseError::Unknown("RetryExpired".into()))
    );
    // The new case belongs only to the new Contract, never an old-id alias.
    assert!(
        outbound_call_semantics(
            &ContractId::parse("std/http-client@2").unwrap(),
            ConnectorDirection::Outbound,
            ExchangeKind::Call,
            None
        )
        .is_none()
    );
    assert_eq!(
        contract(HTTP_CLIENT_CONNECTOR_ID, None)
            .validate_retry_cases(&["ServerError".into()], true),
        Ok(())
    );
}

#[test]
fn call_profiles_require_exact_identity_roles_exchange_and_mode() {
    for (id, mode) in [
        (PROCESS_CONNECTOR_ID, Some("job")),
        (FILE_CONNECTOR_ID, Some("read")),
        (FILE_CONNECTOR_ID, Some("replace")),
        (HTTP_CLIENT_CONNECTOR_ID, None),
    ] {
        let id = ContractId::parse(id).unwrap();
        for roles in [
            ConnectorDirection::Inbound,
            ConnectorDirection::Outbound,
            ConnectorDirection::InboundAndOutbound,
        ] {
            for exchange in [
                ExchangeKind::Event,
                ExchangeKind::Call,
                ExchangeKind::Session,
            ] {
                assert_eq!(
                    outbound_call_semantics(&id, roles, exchange, mode).is_some(),
                    roles == ConnectorDirection::Outbound && exchange == ExchangeKind::Call
                );
            }
        }
    }
    for (id, mode) in [
        (PROCESS_CONNECTOR_ID, None),
        (PROCESS_CONNECTOR_ID, Some("service")),
        (FILE_CONNECTOR_ID, None),
        (FILE_CONNECTOR_ID, Some("watch")),
        (HTTP_CLIENT_CONNECTOR_ID, Some("job")),
        (HTTP_SERVER_CONNECTOR_ID, None),
        (WEBSOCKET_CLIENT_CONNECTOR_ID, None),
        (WEBSOCKET_SERVER_CONNECTOR_ID, None),
        ("org.third-party/process@1", Some("job")),
        ("std/process@99", Some("job")),
    ] {
        assert!(
            outbound_call_semantics(
                &ContractId::parse(id).unwrap(),
                ConnectorDirection::Outbound,
                ExchangeKind::Call,
                mode
            )
            .is_none(),
            "no inferred guarantees for {id} / {mode:?}"
        );
    }
}

#[test]
fn every_terminal_type_case_has_exactly_one_canonical_visibility_row() {
    type ExitType = fn() -> Result<DataType, TypeError>;
    for (id, mode, exit_type) in [
        (
            PROCESS_CONNECTOR_ID,
            Some("job"),
            process_exit_type as ExitType,
        ),
        (FILE_CONNECTOR_ID, Some("read"), file_exit_type as ExitType),
        (
            FILE_CONNECTOR_ID,
            Some("replace"),
            file_exit_type as ExitType,
        ),
        (
            HTTP_CLIENT_CONNECTOR_ID,
            None,
            http_client_exit_type as ExitType,
        ),
    ] {
        let profile = contract(id, mode);
        assert_eq!(profile.request(), "in");
        assert_eq!(profile.terminal(), "exit");
        let DataType::TaggedUnion(union) = exit_type().unwrap() else {
            panic!("a terminal union");
        };
        let names: Vec<_> = union.cases().iter().map(|case| case.name()).collect();
        let classified: Vec<_> = profile
            .terminal_cases()
            .iter()
            .map(|(case, _)| *case)
            .collect();
        assert_eq!(
            classified, names,
            "case changes must update their repetition guarantees: {id}"
        );
    }
}

#[test]
fn ambiguous_failure_names_never_become_before_effect_guarantees() {
    use TerminalVisibility::{AfterEffect, BeforeEffect, OutcomeUnknown};
    for (id, mode, before, after) in [
        (
            PROCESS_CONNECTOR_ID,
            Some("job"),
            &["SpawnFailed"][..],
            &["Exited"][..],
        ),
        (
            FILE_CONNECTOR_ID,
            Some("read"),
            &[
                "AtomicReplaceUnavailable",
                "InvalidPath",
                "NotAFile",
                "NotFound",
                "PermissionDenied",
            ][..],
            &["Completed", "ReadLimitExceeded"][..],
        ),
        (
            HTTP_CLIENT_CONNECTOR_ID,
            None,
            &["ConnectFailed", "DnsFailed", "TlsFailed"][..],
            &["Completed", "ServerError"][..],
        ),
    ] {
        for (case, actual) in contract(id, mode).terminal_cases() {
            let expected = if before.contains(case) {
                BeforeEffect
            } else if after.contains(case) {
                AfterEffect
            } else {
                OutcomeUnknown
            };
            assert_eq!(*actual, expected, "{id} case {case}");
        }
    }
}

#[test]
fn repetition_safety_is_the_full_visibility_by_guarantee_matrix() {
    for visibility in [
        TerminalVisibility::BeforeEffect,
        TerminalVisibility::OutcomeUnknown,
        TerminalVisibility::AfterEffect,
    ] {
        for idempotent in [false, true] {
            assert_eq!(
                visibility.allows_repetition(idempotent),
                visibility == TerminalVisibility::BeforeEffect || idempotent
            );
        }
    }
}

#[test]
fn loop_command_sources_are_checked_without_claiming_retry_safety() {
    let profile = contract(PROCESS_CONNECTOR_ID, Some("job"));
    for case in ["SpawnFailed", "TimeoutExpired", "Exited"] {
        assert_eq!(
            profile.validate_command("exit", Some(&[case.into()])),
            Ok(())
        );
    }
    assert_eq!(profile.validate_command("exit", None), Ok(()));
    assert_eq!(
        profile.validate_command("exit", Some(&["Ghost".into()])),
        Err(CallCommandError::UnknownCase("Ghost".into()))
    );
    assert_eq!(
        profile.validate_command("in", None),
        Err(CallCommandError::UnknownPort)
    );
}

#[test]
fn stream_frames_cannot_issue_an_unbounded_number_of_any_verdict() {
    let profile = contract(PROCESS_CONNECTOR_ID, Some("job"));
    for port in ["out", "err"] {
        assert_eq!(
            profile.frame_production(port),
            Some(FrameProduction::ZeroOrMore)
        );
        assert_eq!(
            profile.validate_command(port, None),
            Err(CallCommandError::RepeatedFrames)
        );
    }
}

#[test]
fn frame_counts_follow_membership_in_the_whole_terminal_case_set() {
    let production = FrameProduction::ExactlyOneOn(&["Accepted", "Warning"]);
    for case in ["Accepted", "Failed", "Warning"] {
        for count in [0, 1, 2, usize::MAX] {
            assert_eq!(
                production.accepts_count(case, count),
                count == usize::from(case != "Failed"),
                "{case} / {count}"
            );
            assert_eq!(
                FrameProduction::Never.accepts_count(case, count),
                count == 0
            );
            assert!(FrameProduction::ZeroOrMore.accepts_count(case, count));
        }
    }
}

#[test]
fn frame_case_sets_are_nonempty_canonical_and_owned_by_the_contract() {
    for (id, mode) in [
        (PROCESS_CONNECTOR_ID, Some("job")),
        (FILE_CONNECTOR_ID, Some("read")),
        (FILE_CONNECTOR_ID, Some("replace")),
        (HTTP_CLIENT_CONNECTOR_ID, None),
    ] {
        let profile = contract(id, mode);
        for (_, production) in profile.frames {
            if let FrameProduction::ExactlyOneOn(cases) = production {
                assert!(!cases.is_empty());
                assert!(cases.windows(2).all(|pair| pair[0] < pair[1]));
                for case in *cases {
                    assert!(
                        profile
                            .terminal_cases()
                            .contains(&(case, TerminalVisibility::AfterEffect))
                    );
                }
            }
        }
    }
}

#[test]
fn single_response_frames_follow_both_http_response_cases_and_replace_has_no_frame() {
    for (id, mode, cases) in [
        (FILE_CONNECTOR_ID, Some("read"), &["Completed"][..]),
        (
            HTTP_CLIENT_CONNECTOR_ID,
            None,
            &["Completed", "ServerError"][..],
        ),
    ] {
        let profile = contract(id, mode);
        assert_eq!(
            profile.frame_production("out"),
            Some(FrameProduction::ExactlyOneOn(cases))
        );
        assert_eq!(profile.frame_production("err"), None);
        assert_eq!(profile.validate_command("out", None), Ok(()));
    }
    let replace = contract(FILE_CONNECTOR_ID, Some("replace"));
    assert_eq!(
        replace.frame_production("out"),
        Some(FrameProduction::Never)
    );
    assert_eq!(replace.validate_command("out", None), Ok(()));
}

#[test]
fn http_status_classification_is_contract_checked_not_just_a_tag() {
    use crate::connector::stdlib::HttpClientExit;
    let profile = contract(HTTP_CLIENT_CONNECTOR_ID, None);
    for status in [499, 500, 599, 600] {
        let expected = if (500..=599).contains(&status) {
            "ServerError"
        } else {
            "Completed"
        };
        let value = HttpClientExit::response(status).into_value();
        assert_eq!(profile.terminal_case(&value), Some(expected));
        let forged = if expected == "Completed" {
            HttpClientExit::ServerError { status }
        } else {
            HttpClientExit::Completed { status }
        };
        assert_eq!(profile.terminal_case(&forged.into_value()), None);
        assert!(
            profile
                .frame_production("out")
                .unwrap()
                .accepts_count(expected, 1)
        );
        assert!(
            !profile
                .frame_production("out")
                .unwrap()
                .accepts_count(expected, 0)
        );
        assert!(
            !profile
                .frame_production("out")
                .unwrap()
                .accepts_count(expected, 2)
        );
    }
    for status in [0, 99, 1000, u16::MAX] {
        assert_eq!(
            profile.terminal_case(&HttpClientExit::response(status).into_value()),
            None
        );
    }
}
