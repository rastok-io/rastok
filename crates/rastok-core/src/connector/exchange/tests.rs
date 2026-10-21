use crate::connector::exchange::*;
use crate::connector::stdlib::*;
use crate::envelope::{self, Data};
use crate::node;
use alloc::{
    string::{String, ToString},
    vec,
    vec::Vec,
};

#[test]
fn prepare_lines_frames_exactly_one_trailing_newline() {
    use crate::envelope::{Data, Encoding, Value};
    let framed = prepare_lines(&Data::materialized(Value::Text(String::from("ligne")))).unwrap();
    assert_eq!(framed.bytes(), b"ligne\n");
    let already = prepare_lines(&Data::materialized(Value::Text(String::from("ligne\n")))).unwrap();
    assert_eq!(already.bytes(), b"ligne\n");
    let raw = prepare_lines(&Data::encoded(Encoding::Raw, *b"verbatim")).unwrap();
    assert_eq!(raw.bytes(), b"verbatim");
    // The seal is the blake3 of exactly those bytes.
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"verbatim");
    let mut digest = [0u8; 32];
    hasher.finalize_xof().fill(&mut digest);
    assert_eq!(raw.digest(), &digest);
}

#[test]
fn the_bytes_codec_never_falls_back_to_lines() {
    use crate::envelope::{Data, Value};
    let framed = prepare_request(
        RequestCodec::Bytes,
        &Data::materialized(Value::Bytes(alloc::vec![0, 0xff, b'\n'])),
    )
    .unwrap();
    assert_eq!(framed.bytes(), &[0, 0xff, b'\n']);
    assert!(
        prepare_request(
            RequestCodec::Bytes,
            &Data::materialized(Value::Text(String::from("not bytes"))),
        )
        .is_err()
    );
}

#[test]
fn codec_refusals_describe_value_kinds_without_formatting_payloads() {
    use crate::envelope::{Data, Value};
    let sentinel = "classified-payload-must-not-escape";
    let payload = Data::materialized(Value::Map(alloc::collections::BTreeMap::from([(
        String::from(sentinel),
        Value::Int(1),
    )])));
    for refusal in [
        network_payload_bytes("text", &payload).expect_err("map is not text"),
        bytes_payload(&payload).expect_err("map is not bytes"),
        prepare_lines(&payload).expect_err("map is not lines"),
    ] {
        assert!(refusal.contains("Map"));
        assert!(!refusal.contains(sentinel));
    }
}

#[test]
fn network_codecs_frame_one_body_or_message_without_transport_delimiters() {
    use crate::envelope::{Data, Encoding, Value};

    assert_eq!(
        network_payload_bytes(
            "json",
            &Data::materialized(Value::Struct(alloc::vec![(
                String::from("answer"),
                Value::Int(42),
            )])),
        )
        .unwrap(),
        br#"{"answer":42}"#
    );
    assert_eq!(
        network_payload_bytes(
            "text",
            &Data::materialized(Value::Text(String::from("line\n"))),
        )
        .unwrap(),
        b"line\n"
    );
    assert_eq!(
        network_payload_bytes("bytes", &Data::encoded(Encoding::Raw, [0, 0xff, b'\n'])).unwrap(),
        [0, 0xff, b'\n']
    );
    assert!(network_payload_bytes("json", &Data::encoded(Encoding::Raw, b"{}".to_vec())).is_err());
}

#[test]
fn inbound_provenance_stays_opaque_but_answers_the_recipes() {
    let provenance = IngressProvenance::single(node::Path::new(["inbound"]).unwrap());
    assert_eq!(provenance.connector_path().to_string(), "inbound");
    assert_eq!(provenance.replica(), 0);
}

#[test]
fn completion_construction_seals_the_terminal_barrier() {
    let out = EnvelopeDraft {
        port: node::Port::conventional_out(),
        data: Data::materialized(crate::value::Value::Int(1)),
        ext: envelope::Namespace::new(),
    };
    let exit = EnvelopeDraft {
        port: node::Port::conventional_exit(),
        data: Data::materialized(ProcessExit::TimeoutExpired.into_value()),
        ext: envelope::Namespace::new(),
    };
    assert_eq!(
        Completion::try_new(vec![out.clone()]),
        Err(CompletionError::MissingExit)
    );
    assert_eq!(
        Completion::try_new(vec![exit.clone(), out]),
        Err(CompletionError::OutputAfterExit)
    );
    assert_eq!(
        Completion::try_new(vec![exit.clone(), exit]),
        Err(CompletionError::MultipleExits(2))
    );
}

#[test]
fn every_process_exit_projection_inhabits_the_one_union() {
    // Populated optional fields pin the Null-first branch convention used
    // by `into_value`; changing canonical union order must break here.
    let exits = [
        ProcessExit::Exited {
            exit_code: -1,
            signal: Some(9),
        },
        ProcessExit::SpawnFailed {
            message: String::from("no executable"),
        },
        ProcessExit::TimeoutExpired,
        ProcessExit::DecodeFailed {
            message: String::from("bad line"),
            line: Some(String::from("{")),
        },
        ProcessExit::CaptureFailed,
        ProcessExit::OutputOverflow {
            limit: 42,
            reason: OutputOverflowReason::Budget,
        },
        ProcessExit::OutputOverflow {
            limit: 42,
            reason: OutputOverflowReason::Capacity,
        },
    ];
    let ty = process_exit_type().expect("the standard ProcessExit type is valid");
    for exit in exits {
        let value = exit.into_value();
        assert!(ty.accepts(&value), "{value:?} must inhabit ProcessExit");
    }
}

#[test]
fn file_exit_is_the_exact_portable_terminal_union() {
    let ty = file_exit_type().expect("the standard FileExit type is valid");
    let crate::datatype::DataType::TaggedUnion(union) = &ty else {
        panic!("FileExit must remain a tagged union")
    };
    let cases: Vec<_> = union.cases().iter().map(|case| case.name()).collect();
    assert_eq!(
        cases,
        [
            "AtomicReplaceUnavailable",
            "Completed",
            "InvalidPath",
            "IoFailed",
            "NotAFile",
            "NotFound",
            "OutcomeUnknown",
            "PermissionDenied",
            "ReadLimitExceeded",
            "TimeoutExpired",
        ]
    );
    let limit = union
        .cases()
        .iter()
        .find(|case| case.name() == "ReadLimitExceeded")
        .expect("the bounded-read case");
    assert_eq!(limit.payload().fields().len(), 1);
    assert_eq!(limit.payload().fields()[0].name(), "limit");
    assert_eq!(
        limit.payload().fields()[0].ty(),
        &crate::datatype::DataType::UINT
    );
    assert!(
        union
            .cases()
            .iter()
            .filter(|case| case.name() != "ReadLimitExceeded")
            .all(|case| case.payload().fields().is_empty())
    );
    for exit in [
        FileExit::Completed,
        FileExit::NotFound,
        FileExit::NotAFile,
        FileExit::PermissionDenied,
        FileExit::InvalidPath,
        FileExit::ReadLimitExceeded { limit: 42 },
        FileExit::AtomicReplaceUnavailable,
        FileExit::IoFailed,
        FileExit::TimeoutExpired,
        FileExit::OutcomeUnknown,
    ] {
        let value = exit.into_value();
        assert!(ty.accepts(&value), "{value:?} must inhabit FileExit");
    }
}

#[test]
fn every_network_exit_projection_inhabits_its_closed_union() {
    let client_ty = http_client_exit_type().expect("the HttpClientExit type");
    for exit in [
        HttpClientExit::Completed { status: 418 },
        HttpClientExit::DnsFailed,
        HttpClientExit::ConnectFailed,
        HttpClientExit::TlsFailed,
        HttpClientExit::ProtocolFailed,
        HttpClientExit::TimeoutExpired,
        HttpClientExit::BodyTooLarge { limit: 42 },
        HttpClientExit::DecodeFailed,
        HttpClientExit::OutcomeUnknown,
    ] {
        let value = exit.into_value();
        assert!(
            client_ty.accepts(&value),
            "{value:?} must inhabit HttpClientExit"
        );
    }

    let server_ty = http_server_exit_type().expect("the HttpServerExit type");
    for exit in [
        HttpServerExit::Completed { status: 202 },
        HttpServerExit::RequestExpired,
        HttpServerExit::Overloaded,
        HttpServerExit::PeerDisconnected,
        HttpServerExit::OutcomeUnknown,
    ] {
        let value = exit.into_value();
        assert!(
            server_ty.accepts(&value),
            "{value:?} must inhabit HttpServerExit"
        );
    }

    let websocket_ty = websocket_exit_type().expect("the WebSocketExit type");
    for exit in [
        WebSocketExit::Closed {
            code: 1000,
            reason: String::from("done"),
        },
        WebSocketExit::DnsFailed,
        WebSocketExit::ConnectFailed,
        WebSocketExit::TlsFailed,
        WebSocketExit::HandshakeRejected { status: 403 },
        WebSocketExit::ProtocolFailed,
        WebSocketExit::TransportLost,
        WebSocketExit::TimeoutExpired,
        WebSocketExit::MessageTooLarge { limit: 42 },
        WebSocketExit::DecodeFailed,
        WebSocketExit::OutcomeUnknown,
    ] {
        let value = exit.into_value();
        assert!(
            websocket_ty.accepts(&value),
            "{value:?} must inhabit WebSocketExit"
        );
    }
}
