use super::*;
use crate::boundary::{file_exit_type, http_client_exit_type, process_exit_type};
use crate::connector::{
    FILE_CONNECTOR_ID, HTTP_CLIENT_CONNECTOR_ID, HTTP_SERVER_CONNECTOR_ID, PROCESS_CONNECTOR_ID,
    WEBSOCKET_CLIENT_CONNECTOR_ID, WEBSOCKET_SERVER_CONNECTOR_ID,
};
use crate::datatype::{DataType, TypeError};
use alloc::vec::Vec;

fn contract(id: &str, mode: Option<&str>) -> CallSemantics {
    outbound_call_semantics(
        &ContractId::parse(id).unwrap(),
        ConnectorRoles::Outbound,
        ConnectorExchange::Call,
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
    // The new HTTP response case is not silently grafted onto the current @2.
    assert_eq!(
        contract(HTTP_CLIENT_CONNECTOR_ID, None)
            .validate_retry_cases(&["ServerError".into()], true),
        Err(RetryCaseError::Unknown("ServerError".into()))
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
            ConnectorRoles::Inbound,
            ConnectorRoles::Outbound,
            ConnectorRoles::InboundAndOutbound,
        ] {
            for exchange in [
                ConnectorExchange::Event,
                ConnectorExchange::Call,
                ConnectorExchange::Session,
            ] {
                assert_eq!(
                    outbound_call_semantics(&id, roles, exchange, mode).is_some(),
                    roles == ConnectorRoles::Outbound && exchange == ConnectorExchange::Call
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
        ("third-party/process@1", Some("job")),
        ("rastok/process@99", Some("job")),
    ] {
        assert!(
            outbound_call_semantics(
                &ContractId::parse(id).unwrap(),
                ConnectorRoles::Outbound,
                ConnectorExchange::Call,
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
            &["Completed"][..],
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
fn repetition_safety_is_the_full_visibility_by_effect_matrix() {
    for visibility in [
        TerminalVisibility::BeforeEffect,
        TerminalVisibility::OutcomeUnknown,
        TerminalVisibility::AfterEffect,
    ] {
        for effect in [
            Effect::None,
            Effect::Class(EffectClass::Idempotent),
            Effect::Class(EffectClass::Compensable),
            Effect::Class(EffectClass::Irreversible),
        ] {
            let expected = visibility == TerminalVisibility::BeforeEffect
                || effect == Effect::None
                || effect == Effect::Class(EffectClass::Idempotent);
            assert_eq!(visibility.allows_recurrence(effect), expected);
        }
    }
}

#[test]
fn terminal_case_selection_controls_recurrence_but_done_never_repeats() {
    let profile = contract(PROCESS_CONNECTOR_ID, Some("job"));
    let unsafe_effect = Effect::Class(EffectClass::Irreversible);
    for command in [LoopCommandKind::Next, LoopCommandKind::Again] {
        assert_eq!(
            profile.validate_command(
                "exit",
                Some(&["SpawnFailed".into()]),
                command,
                unsafe_effect
            ),
            Ok(())
        );
        assert_eq!(
            profile.validate_command(
                "exit",
                Some(&["TimeoutExpired".into()]),
                command,
                unsafe_effect
            ),
            Err(CallCommandError::UnsafeRecurrence {
                case: "TimeoutExpired".into(),
                visibility: TerminalVisibility::OutcomeUnknown
            })
        );
        assert!(
            profile
                .validate_command("exit", None, command, unsafe_effect)
                .is_err()
        );
    }
    assert_eq!(
        profile.validate_command("exit", None, LoopCommandKind::Done, unsafe_effect),
        Ok(())
    );
    assert_eq!(
        profile.validate_command(
            "exit",
            Some(&["Ghost".into()]),
            LoopCommandKind::Done,
            Effect::None
        ),
        Err(CallCommandError::UnknownCase("Ghost".into()))
    );
    assert_eq!(
        profile.validate_command("in", None, LoopCommandKind::Done, Effect::None),
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
        for command in [
            LoopCommandKind::Next,
            LoopCommandKind::Again,
            LoopCommandKind::Done,
        ] {
            assert_eq!(
                profile.validate_command(
                    port,
                    None,
                    command,
                    Effect::Class(EffectClass::Idempotent)
                ),
                Err(CallCommandError::RepeatedFrames)
            );
        }
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
fn single_response_frames_prove_visibility_and_replace_has_no_frame() {
    for (id, mode) in [
        (FILE_CONNECTOR_ID, Some("read")),
        (HTTP_CLIENT_CONNECTOR_ID, None),
    ] {
        let profile = contract(id, mode);
        assert_eq!(
            profile.frame_production("out"),
            Some(FrameProduction::ExactlyOneOn(&["Completed"]))
        );
        assert_eq!(profile.frame_production("err"), None);
        assert_eq!(
            profile.validate_command(
                "out",
                None,
                LoopCommandKind::Again,
                Effect::Class(EffectClass::Compensable)
            ),
            Err(CallCommandError::UnsafeRecurrence {
                case: "Completed".into(),
                visibility: TerminalVisibility::AfterEffect
            })
        );
        assert_eq!(
            profile.validate_command(
                "out",
                None,
                LoopCommandKind::Again,
                Effect::Class(EffectClass::Idempotent)
            ),
            Ok(())
        );
    }
    let replace = contract(FILE_CONNECTOR_ID, Some("replace"));
    assert_eq!(
        replace.frame_production("out"),
        Some(FrameProduction::Never)
    );
    assert_eq!(
        replace.validate_command(
            "out",
            None,
            LoopCommandKind::Again,
            Effect::Class(EffectClass::Irreversible)
        ),
        Ok(())
    );
}

#[test]
fn each_terminal_case_is_checked_for_each_effect_and_command() {
    for (id, mode) in [
        (PROCESS_CONNECTOR_ID, Some("job")),
        (FILE_CONNECTOR_ID, Some("read")),
        (FILE_CONNECTOR_ID, Some("replace")),
        (HTTP_CLIENT_CONNECTOR_ID, None),
    ] {
        let profile = contract(id, mode);
        for (case, visibility) in profile.terminal_cases() {
            for effect in [
                Effect::None,
                Effect::Class(EffectClass::Idempotent),
                Effect::Class(EffectClass::Compensable),
                Effect::Class(EffectClass::Irreversible),
            ] {
                for command in [
                    LoopCommandKind::Next,
                    LoopCommandKind::Again,
                    LoopCommandKind::Done,
                ] {
                    let allowed = command == LoopCommandKind::Done
                        || *visibility == TerminalVisibility::BeforeEffect
                        || matches!(
                            effect,
                            Effect::None | Effect::Class(EffectClass::Idempotent)
                        );
                    assert_eq!(
                        profile
                            .validate_command("exit", Some(&[(*case).into()]), command, effect)
                            .is_ok(),
                        allowed,
                        "{id} {case} {command:?} {effect}"
                    );
                }
            }
        }
    }
}
