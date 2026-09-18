//! The contract metamodel and the effective sheet (D-2026-08-12-1):
//! presence, profiles, defaults injected before the hash, the plan
//! carrying effective attributes, and the Driver consuming resolved
//! values only.

#![allow(clippy::disallowed_types, clippy::disallowed_methods)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use rastok::compiler::compile;
use rastok::contract::{
    AttributeSpec, ConnectorContract, ContractCatalog, DefaultValue, EvaluationSite,
    FrontendContract, FrontendFraming, FrontendReader, PortContract, PortDataType, PortTypeChoice,
    PortTypeRule, Presence, Profile, ProfileSelector, ProfileSupport, SheetDefect, ValueDomain,
};
use rastok::node::Direction;

fn compile_one(source: &str) -> Result<rastok::compiler::Compilation, rastok::diagnostics::Bag> {
    compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
}

fn process_graph(body: &str) -> String {
    format!(
        "graph g : App {{\n  connector shell : Process {{\n{body}\n  }}\n  in -> shell -> out\n  shell.err -> nothing\n  shell.exit -> nothing\n}}\n"
    )
}

fn file_graph(body: &str) -> String {
    format!(
        "graph g : App {{\n  connector file : File {{\n{body}\n  }}\n  in -> file\n  file.out -> nothing\n  file.exit -> nothing\n}}\n"
    )
}

fn network_graph(kind: &str, body: &str, wiring: &str) -> String {
    format!(
        "graph g : App {{\n  connector network : {kind} {{\n{body}\n  }}\n{wiring}\n  network.exit -> nothing\n}}\n"
    )
}

const FULL: &str =
    "    command = \"cat\" ; codec = lines\n    idempotent = true ; behavior = deterministic";

fn refusal_with(source: &str, code: &str, needle: &str) {
    let bag = compile_one(source).expect_err("must refuse");
    assert!(
        bag.diagnostics()
            .iter()
            .any(|d| d.code().as_str() == code && d.message().contains(needle)),
        "wanted {code} containing {needle:?}, got: {bag:?}"
    );
}

#[test]
fn all_four_network_contracts_cross_the_source_to_validated_plan_boundary() {
    let cases = [
        (
            "HttpClient",
            "    url = \"https://example.test/v1\"\n    headers = { content_type = media_type(\"application/octet-stream\") }\n    custom_headers = { \"X-Trace-Id\": [\"one\"] }\n    tls_certificate = base64! { Y2VydA== }\n    tls_private_key = base64! { a2V5 }\n    timeout = duration(\"5s\")\n    codec = bytes\n    idempotent = true ; behavior = deterministic",
            "  in -> network -> out",
            rastok::connector::HTTP_CLIENT_CONNECTOR_ID,
            rastok::connector::ConnectorExchange::Call,
            "a1bf3c54b12e0bb08795d67b2849560fa1b529ec97f802484d7578ef77134f56",
        ),
        (
            "HttpServer",
            "    listen = \"127.0.0.1:8080\"\n    codec = bytes",
            "  network -> out",
            rastok::connector::HTTP_SERVER_CONNECTOR_ID,
            rastok::connector::ConnectorExchange::Call,
            "de5b489357e12e52bbd7b536e133d103dc6e7430734fa65e476dfee38e99fcdf",
        ),
        (
            "WebSocketClient",
            "    url = \"wss://example.test/socket\"\n    handshake_timeout = duration(\"5s\")\n    codec = text\n    behavior = deterministic",
            "  in -> network -> out",
            rastok::connector::WEBSOCKET_CLIENT_CONNECTOR_ID,
            rastok::connector::ConnectorExchange::Session,
            "e5d8c57b20427655842aa608f078858c744ede0a9648f3a4b881bb3c4b612020",
        ),
        (
            "WebSocketServer",
            "    listen = \"127.0.0.1:8081\"\n    handshake_timeout = duration(\"5s\")\n    codec = json",
            "  in -> network -> out",
            rastok::connector::WEBSOCKET_SERVER_CONNECTOR_ID,
            rastok::connector::ConnectorExchange::Session,
            "37c7043e337fb958e7a743ddd1d9ad1693b1bf9d169297f2b882efbafe356667",
        ),
    ];
    let mut hashes = Vec::new();
    for (kind, body, wiring, expected_id, expected_exchange, expected_hash) in cases {
        let source = network_graph(kind, body, wiring);
        let compilation =
            compile_one(&source).unwrap_or_else(|bag| panic!("{kind} should compile: {bag:#?}"));
        let node = compilation
            .plan
            .plan()
            .nodes
            .iter()
            .find(|node| node.path.to_string() == "network")
            .expect("the network connector");
        let spec = node.connector().expect("a connector spec");
        assert_eq!(spec.contract.as_str(), expected_id);
        assert_eq!(spec.exchange, expected_exchange);
        assert_eq!(
            spec.attribute("zero_rtt"),
            Some(&rastok::plan::AttrSetting::Word(String::from("disabled")))
        );
        assert!(spec.attribute("versions").is_some());
        let package = rastok::plan::encode(&compilation.plan, &compilation.hash);
        let (decoded, hash) = rastok::plan::decode(&package)
            .unwrap_or_else(|error| panic!("the {kind} package must decode: {error}"));
        assert_eq!(hash, compilation.hash);
        assert_eq!(*decoded, *compilation.plan);
        hashes.push((kind, compilation.hash.to_string(), expected_hash));
    }
    assert!(
        hashes
            .iter()
            .all(|(_, actual, expected)| actual == expected),
        "representative Contract identities moved: {hashes:#?}"
    );
}

#[test]
fn typed_http_headers_keep_known_fields_precise_and_extensions_open() {
    let source = network_graph(
        "HttpClient",
        "    url = \"https://example.test/v1\"\n    headers = { content_type = media_type(\"Application/Problem+JSON; Charset=UTF-8\") }\n    custom_headers = { \"X-Trace-ID\": [\"one\", \"two\"] }\n    timeout = duration(\"5s\")\n    codec = bytes\n    idempotent = true ; behavior = deterministic",
        "  in -> network -> out",
    );
    compile_one(&source).expect("typed and extension request fields compile together");

    let wrong_known = network_graph(
        "HttpClient",
        "    url = \"https://example.test/v1\"\n    headers = { content_type = \"not/a media type with spaces\" }\n    timeout = duration(\"5s\")\n    codec = bytes\n    idempotent = true ; behavior = deterministic",
        "  in -> network -> out",
    );
    refusal_with(&wrong_known, "E-CONTRACT-ATTR", "TypedHttpHeaders");

    let managed_custom = network_graph(
        "HttpClient",
        "    url = \"https://example.test/v1\"\n    custom_headers = { \"Content-Type\": [\"application/json\"] }\n    timeout = duration(\"5s\")\n    codec = bytes\n    idempotent = true ; behavior = deterministic",
        "  in -> network -> out",
    );
    refusal_with(&managed_custom, "E-CONTRACT-ATTR", "CustomHttpHeaders");

    let colliding_custom = network_graph(
        "HttpClient",
        "    url = \"https://example.test/v1\"\n    custom_headers = { \"X-Trace\": [\"one\"], \"x-trace\": [\"two\"] }\n    timeout = duration(\"5s\")\n    codec = bytes\n    idempotent = true ; behavior = deterministic",
        "  in -> network -> out",
    );
    refusal_with(&colliding_custom, "E-CONTRACT-ATTR", "CustomHttpHeaders");
}

#[test]
fn network_defaults_are_effective_before_identity() {
    let omitted = network_graph(
        "HttpClient",
        "    url = \"https://example.test/v1\"\n    timeout = duration(\"5s\")\n    codec = json\n    idempotent = true ; behavior = deterministic",
        "  in -> network -> out",
    );
    let explicit = network_graph(
        "HttpClient",
        "    url = \"https://example.test/v1\"\n    method = \"GET\"\n    timeout = duration(\"5s\")\n    codec = json\n    max_body = \"8MiB\"\n    versions = [\"http/3\", \"http/2\", \"http/1.1\"]\n    zero_rtt = disabled\n    idempotent = true ; behavior = deterministic",
        "  in -> network -> out",
    );
    let omitted = compile_one(&omitted).expect("the omitted defaults compile");
    let explicit = compile_one(&explicit).expect("the exact defaults compile");
    assert_eq!(
        omitted.hash.to_string(),
        "135282f162cbc350e091d230ff4d35077c920003a8f541c6173a47e671c1c661",
        "the v19 Contract/default witness stays pinned"
    );
    assert_eq!(omitted.hash, explicit.hash);
    assert_eq!(*omitted.plan, *explicit.plan);
}

#[test]
fn unfinished_network_features_refuse_instead_of_selecting_a_nearby_behavior() {
    let response_to_ack = r#"
graph g : App {
  connector server : HttpServer {
    listen = "127.0.0.1:8080"
    codec = text
  }
  operator relay { input in: text ; on in { out = data } }
  server -> relay -> server
}
"#;
    refusal_with(
        response_to_ack,
        "E-CONTRACT-ATTR",
        "has no graph-response input",
    );

    let bad_tls = network_graph(
        "HttpClient",
        "    url = \"https://example.test\"\n    timeout = duration(\"5s\")\n    codec = bytes\n    idempotent = true ; behavior = deterministic\n    tls_certificate = base64! { Y2VydA== }",
        "  in -> network -> out",
    );
    refusal_with(
        &bad_tls,
        "E-CONTRACT-ATTR",
        "requires both certificate bytes and private-key bytes",
    );

    let bad_version = network_graph(
        "HttpClient",
        "    url = \"https://example.test\"\n    timeout = duration(\"5s\")\n    codec = bytes\n    idempotent = true ; behavior = deterministic\n    versions = [\"http/4\"]",
        "  in -> network -> out",
    );
    refusal_with(&bad_version, "E-CONTRACT-ATTR", "ChoiceList");

    let early_data = network_graph(
        "WebSocketClient",
        "    url = \"wss://example.test/socket\"\n    handshake_timeout = duration(\"5s\")\n    codec = text\n    behavior = deterministic\n    zero_rtt = enabled",
        "  in -> network -> out",
    );
    refusal_with(&early_data, "E-CONTRACT-ATTR", "fixed to `disabled`");
}

#[test]
fn http_server_respond_requires_one_direct_text_correlator_only_when_roots_mix() {
    let direct = r#"
graph direct : App {
  connector server : HttpServer {
    listen = "127.0.0.1:8080"
    codec = text
    mode = respond
    timeout = duration("1s")
  }
  operator relay { input in: text ; on in { out = data } }
  server -> relay -> server
  server.exit -> nothing
}
"#;
    compile_one(direct).expect("direct lineage identifies the retained Call without a correlator");

    for (spelling, code, reason) in [
        (
            "correlate = \"constant\"",
            "E-CONTRACT-ATTR",
            "direct `data` or `meta` path",
        ),
        (
            "correlate = meta.sys.origin_id + \"\"",
            "E-CONTRACT-ATTR",
            "direct `data` or `meta` path",
        ),
        ("correlate = size(data)", "E-PORT-TYPE", "requires text"),
    ] {
        let malformed = direct.replace(
            "    timeout = duration(\"1s\")",
            &format!("    timeout = duration(\"1s\")\n    {spelling}"),
        );
        refusal_with(&malformed, code, reason);
    }

    let mixed = r#"
graph mixed : App {
  input other: text
  connector server : HttpServer {
    listen = "127.0.0.1:8080"
    codec = json
    mode = respond
    timeout = duration("1s")
  }
  operator pair {
    input request: variant
    input other: text
    on (request, other) {
      out = { data = request.data ; meta.app.request_id = other.data }
    }
  }
  server -> pair.request
  other -> pair.other
  pair -> server
  server.exit -> nothing
}
"#;
    refusal_with(mixed, "E-CONTRACT-ATTR", "root-mixing join");
    let attributed = mixed.replace(
        "    timeout = duration(\"1s\")",
        "    timeout = duration(\"1s\")\n    correlate = meta.app.request_id",
    );
    compile_one(&attributed)
        .expect("an explicitly attributed join output selects its retained Call by text token");
}

fn forged_http_client_error(
    mutate: impl Fn(&mut rastok::plan::Node),
) -> rastok::plan::PlanValidationError {
    let source = network_graph(
        "HttpClient",
        "    url = \"https://example.test/v1\"\n    timeout = duration(\"5s\")\n    codec = json\n    idempotent = true ; behavior = deterministic",
        "  in -> network -> out",
    );
    let compilation = compile_one(&source).expect("the HttpClient sheet checks");
    let mut plan: rastok::plan::ExecutionPlan = (*compilation.plan).clone();
    let network = plan
        .nodes
        .iter_mut()
        .find(|node| node.path.to_string() == "network")
        .expect("the network connector");
    mutate(network);
    rastok::plan::ValidatedExecutionPlan::try_from(plan)
        .expect_err("the Registry-free network mirror refuses the forgery")
}

fn connector_mut(node: &mut rastok::plan::Node) -> &mut rastok::plan::ConnectorSpec {
    match &mut node.execution {
        rastok::plan::NodeExecution::Connector(spec) => spec,
        rastok::plan::NodeExecution::Operator => panic!("the test selected a connector"),
    }
}

#[test]
fn the_core_rejects_network_sheet_forgeries_without_a_contract_catalog() {
    let missing = forged_http_client_error(|node| {
        connector_mut(node)
            .attributes
            .retain(|(name, _)| name != "versions");
    });
    assert!(matches!(
        missing,
        rastok::plan::PlanValidationError::MissingContractAttribute {
            attribute: "versions",
            ..
        }
    ));

    let unsupported = forged_http_client_error(|node| {
        let spec = connector_mut(node);
        let setting = spec
            .attributes
            .iter_mut()
            .find(|(name, _)| name == "zero_rtt")
            .expect("the materialized default");
        setting.1 = rastok::plan::AttrSetting::Word(String::from("enabled"));
    });
    assert!(matches!(
        unsupported,
        rastok::plan::PlanValidationError::ContractAttributeMismatch {
            attribute,
            ..
        } if attribute == "zero_rtt"
    ));

    let wrong_exit = forged_http_client_error(|node| {
        node.ports
            .iter_mut()
            .find(|port| port.port.name() == "exit")
            .expect("the terminal port")
            .ty = Some(rastok::datatype::DataType::Text);
    });
    assert!(matches!(
        wrong_exit,
        rastok::plan::PlanValidationError::ContractPortMismatch { .. }
    ));

    let half_identity = forged_http_client_error(|node| {
        let spec = connector_mut(node);
        spec.attributes.push((
            String::from("tls_certificate"),
            rastok::plan::AttrSetting::Value(rastok::value::Value::Bytes(b"cert".to_vec())),
        ));
        spec.attributes.sort_by(|left, right| left.0.cmp(&right.0));
    });
    assert!(matches!(
        half_identity,
        rastok::plan::PlanValidationError::ContractAttributeMismatch {
            attribute,
            ..
        } if attribute == "tls_certificate/tls_private_key"
    ));
}

// ---------- the metamodel holds its own door ----------

fn sheet(common: &'static [AttributeSpec], selector: Option<ProfileSelector>) -> ConnectorContract {
    const PORTS: &[PortContract] = &[
        PortContract {
            name: "in",
            direction: Direction::Input,

            ty: PortTypeRule::Fixed(PortDataType::Variant),
        },
        PortContract {
            name: "out",
            direction: Direction::Output,

            ty: PortTypeRule::Fixed(PortDataType::Variant),
        },
        PortContract {
            name: "err",
            direction: Direction::Output,

            ty: PortTypeRule::Fixed(PortDataType::Variant),
        },
        PortContract {
            name: "exit",
            direction: Direction::Output,

            ty: PortTypeRule::Fixed(PortDataType::Variant),
        },
    ];
    ConnectorContract {
        id: "test/probe@1",
        name: "Probe",
        roles: rastok::connector::ConnectorRoles::Outbound,
        exchange: rastok::connector::ConnectorExchange::Call,
        ports: PORTS,
        common,
        selector,
    }
}

#[test]
fn a_duplicate_attribute_is_a_sheet_defect() {
    const DUP: &[AttributeSpec] = &[
        AttributeSpec {
            name: "x",
            presence: Presence::Optional,
            domain: ValueDomain::Text,
            latest_site: EvaluationSite::Plan,
        },
        AttributeSpec {
            name: "x",
            presence: Presence::Optional,
            domain: ValueDomain::Text,
            latest_site: EvaluationSite::Plan,
        },
    ];
    assert_eq!(
        ContractCatalog::try_m0()
            .expect("the standard catalogue holds")
            .try_with(sheet(DUP, None))
            .unwrap_err(),
        SheetDefect::DuplicateAttribute("x")
    );
}

#[test]
fn the_exchange_shape_owns_the_terminal_row_cardinality() {
    let event_with_exit = ConnectorContract {
        roles: rastok::connector::ConnectorRoles::Inbound,
        exchange: rastok::connector::ConnectorExchange::Event,
        ..sheet(&[], None)
    };
    assert_eq!(
        ContractCatalog::default()
            .try_with(event_with_exit)
            .unwrap_err(),
        SheetDefect::ExchangeTerminalMismatch {
            exchange: rastok::connector::ConnectorExchange::Event,
            exits: 1,
        }
    );

    const EVENT_PORTS: &[PortContract] = &[PortContract {
        name: "out",
        direction: Direction::Output,

        ty: PortTypeRule::Fixed(PortDataType::Variant),
    }];
    let event = ConnectorContract {
        id: "test/event@1",
        name: "EventProbe",
        roles: rastok::connector::ConnectorRoles::Inbound,
        exchange: rastok::connector::ConnectorExchange::Event,
        ports: EVENT_PORTS,
        common: &[],
        selector: None,
    };
    ContractCatalog::default()
        .try_with(event.clone())
        .expect("an Event has no fabricated per-event exit");

    assert_eq!(
        ContractCatalog::default()
            .try_with(ConnectorContract {
                id: "test/call@1",
                name: "CallProbe",
                roles: rastok::connector::ConnectorRoles::Outbound,
                exchange: rastok::connector::ConnectorExchange::Call,
                ..event
            })
            .unwrap_err(),
        SheetDefect::ExchangeTerminalMismatch {
            exchange: rastok::connector::ConnectorExchange::Call,
            exits: 0,
        }
    );

    ContractCatalog::default()
        .try_with(ConnectorContract {
            id: "test/session@1",
            name: "SessionProbe",
            roles: rastok::connector::ConnectorRoles::InboundAndOutbound,
            exchange: rastok::connector::ConnectorExchange::Session,
            ..sheet(&[], None)
        })
        .expect("a Session carries one terminal exit like a Call");
}

#[test]
fn an_event_is_the_one_way_inbound_exchange() {
    const EVENT_PORTS: &[PortContract] = &[PortContract {
        name: "out",
        direction: Direction::Output,

        ty: PortTypeRule::Fixed(PortDataType::Variant),
    }];
    let error = ContractCatalog::default()
        .try_with(ConnectorContract {
            id: "test/outbound-event@1",
            name: "OutboundEvent",
            roles: rastok::connector::ConnectorRoles::Outbound,
            exchange: rastok::connector::ConnectorExchange::Event,
            ports: EVENT_PORTS,
            common: &[],
            selector: None,
        })
        .unwrap_err();
    assert_eq!(
        error,
        SheetDefect::ExchangeRoleMismatch {
            exchange: rastok::connector::ConnectorExchange::Event,
            roles: rastok::connector::ConnectorRoles::Outbound,
        }
    );

    let error = ContractCatalog::default()
        .try_with(ConnectorContract {
            id: "test/inbound-session@1",
            name: "InboundSession",
            roles: rastok::connector::ConnectorRoles::Inbound,
            exchange: rastok::connector::ConnectorExchange::Session,
            ..sheet(&[], None)
        })
        .unwrap_err();
    assert_eq!(
        error,
        SheetDefect::ExchangeRoleMismatch {
            exchange: rastok::connector::ConnectorExchange::Session,
            roles: rastok::connector::ConnectorRoles::Inbound,
        },
        "a Session needs the exchange-specific outbound observation view"
    );
}

#[test]
fn the_catalogue_keys_source_symbols_by_stable_contract_identity() {
    let catalogue = ContractCatalog::try_m0().expect("the standard catalogue holds");
    let id = rastok::connector::ContractId::parse(rastok::connector::PROCESS_CONNECTOR_ID).unwrap();
    let process = catalogue
        .connector_by_id(&id)
        .expect("Process is keyed by its stable id");
    assert_eq!(process.name, "Process");
    assert_eq!(process.roles, rastok::connector::ConnectorRoles::Outbound);

    let invalid = ConnectorContract {
        id: "Process",
        ..sheet(&[], None)
    };
    assert_eq!(
        ContractCatalog::default().try_with(invalid).unwrap_err(),
        SheetDefect::InvalidConnectorId {
            id: "Process",
            reason: rastok::connector::ConnectorIdError::Shape,
        }
    );

    let alias = ConnectorContract {
        id: rastok::connector::PROCESS_CONNECTOR_ID,
        name: "ProcessAlias",
        ..sheet(&[], None)
    };
    assert_eq!(
        catalogue.try_with(alias).unwrap_err(),
        SheetDefect::ConnectorIdCollision {
            id: rastok::connector::PROCESS_CONNECTOR_ID,
            existing: "Process",
            incoming: "ProcessAlias",
        }
    );
}

#[test]
fn source_connector_symbols_disappear_after_contract_resolution() {
    let process_catalogue = ContractCatalog::default()
        .try_with(rastok::stdlib::PROCESS.clone())
        .expect("the Process sheet holds");
    let alias_catalogue = ContractCatalog::default()
        .try_with(ConnectorContract {
            name: "Shell",
            ..rastok::stdlib::PROCESS.clone()
        })
        .expect("one alternate source symbol may resolve to the same stable id");
    let process_source = process_graph(FULL);
    let process = compile(&[process_source.as_str()], &process_catalogue).expect("Process checks");
    let shell_source = process_source.replace(": Process", ": Shell");
    let shell = compile(&[shell_source.as_str()], &alias_catalogue).expect("Shell checks");

    assert_eq!(process.hash, shell.hash);
    assert_eq!(process.plan, shell.plan);
}

#[test]
fn port_type_cycles_are_rejected_even_when_reached_through_a_tail() {
    const CYCLIC: ConnectorContract = ConnectorContract {
        id: "test/cyclic@1",
        name: "Cyclic",
        roles: rastok::connector::ConnectorRoles::Outbound,
        exchange: rastok::connector::ConnectorExchange::Call,
        ports: &[
            PortContract {
                name: "first",
                direction: Direction::Output,

                ty: PortTypeRule::SameAs("second"),
            },
            PortContract {
                name: "second",
                direction: Direction::Output,

                ty: PortTypeRule::SameAs("third"),
            },
            PortContract {
                name: "third",
                direction: Direction::Output,

                ty: PortTypeRule::SameAs("second"),
            },
        ],
        common: &[],
        selector: None,
    };
    assert!(matches!(
        ContractCatalog::try_m0()
            .expect("the standard catalogue holds")
            .try_with(CYCLIC),
        Err(SheetDefect::CyclicPortTypes("first"))
    ));
}

#[test]
fn same_as_and_profile_selected_ports_resolve_without_a_second_type_authority() {
    const PROFILES: &[Profile] = &[
        Profile {
            protocol: None,
            word: "textual",
            support: ProfileSupport::Available,
            attributes: &[],
        },
        Profile {
            protocol: None,
            word: "binary",
            support: ProfileSupport::Available,
            attributes: &[],
        },
    ];
    const SHEET: ConnectorContract = ConnectorContract {
        id: "test/parametric@1",
        name: "Parametric",
        roles: rastok::connector::ConnectorRoles::Outbound,
        exchange: rastok::connector::ConnectorExchange::Call,
        ports: &[
            PortContract {
                name: "in",
                direction: Direction::Input,

                ty: PortTypeRule::AttributeSelected {
                    attribute: "mode",
                    choices: &[
                        PortTypeChoice {
                            word: "textual",
                            ty: PortDataType::Text,
                        },
                        PortTypeChoice {
                            word: "binary",
                            ty: PortDataType::Bytes,
                        },
                    ],
                },
            },
            PortContract {
                name: "out",
                direction: Direction::Output,

                ty: PortTypeRule::SameAs("in"),
            },
            PortContract {
                name: "exit",
                direction: Direction::Output,

                ty: PortTypeRule::SameAs("in"),
            },
        ],
        common: &[],
        selector: Some(ProfileSelector {
            attribute: "mode",
            default: Some("textual"),
            profiles: PROFILES,
        }),
    };
    let registry = ContractCatalog::try_m0()
        .expect("the standard catalogue holds")
        .try_with(SHEET.clone())
        .expect("the parametric sheet is sound");
    assert!(matches!(
        registry.connector("Parametric"),
        rastok::contract::Availability::Available(_)
    ));
    let ports = SHEET
        .resolve_ports(&std::collections::BTreeMap::from([(
            String::from("mode"),
            String::from("binary"),
        )]))
        .expect("the selected table resolves");
    assert!(
        ports
            .iter()
            .all(|port| { port.ty.as_ref() == Some(&rastok::datatype::DataType::BYTES) })
    );
}

#[test]
fn a_profile_local_attribute_cannot_ambiguously_select_a_port_type() {
    const SHAPE_TEXT: &[AttributeSpec] = &[AttributeSpec {
        name: "shape",
        presence: Presence::Optional,
        domain: ValueDomain::Choice(&["text"]),
        latest_site: EvaluationSite::Plan,
    }];
    const SHAPE_BYTES: &[AttributeSpec] = &[AttributeSpec {
        name: "shape",
        presence: Presence::Optional,
        domain: ValueDomain::Choice(&["bytes"]),
        latest_site: EvaluationSite::Plan,
    }];
    const AMBIGUOUS: ConnectorContract = ConnectorContract {
        id: "test/ambiguous@1",
        name: "Ambiguous",
        roles: rastok::connector::ConnectorRoles::Outbound,
        exchange: rastok::connector::ConnectorExchange::Call,
        ports: &[PortContract {
            name: "out",
            direction: Direction::Output,

            ty: PortTypeRule::AttributeSelected {
                attribute: "shape",
                choices: &[PortTypeChoice {
                    word: "text",
                    ty: PortDataType::Text,
                }],
            },
        }],
        common: &[],
        selector: Some(ProfileSelector {
            attribute: "mode",
            default: Some("a"),
            profiles: &[
                Profile {
                    protocol: None,
                    word: "a",
                    support: ProfileSupport::Available,
                    attributes: SHAPE_TEXT,
                },
                Profile {
                    protocol: None,
                    word: "b",
                    support: ProfileSupport::Available,
                    attributes: SHAPE_BYTES,
                },
            ],
        }),
    };
    assert_eq!(
        ContractCatalog::try_m0()
            .expect("the standard catalogue holds")
            .try_with(AMBIGUOUS)
            .unwrap_err(),
        SheetDefect::UnknownPortTypeAttribute {
            port: "out",
            attribute: "shape",
        }
    );
}

#[test]
fn a_port_type_selector_is_structurally_plan_bound() {
    const SHAPE: &[AttributeSpec] = &[AttributeSpec {
        name: "shape",
        presence: Presence::Required,
        domain: ValueDomain::Choice(&["text"]),
        latest_site: EvaluationSite::Instance,
    }];
    const LATE: ConnectorContract = ConnectorContract {
        id: "test/late@1",
        name: "Late",
        roles: rastok::connector::ConnectorRoles::Outbound,
        exchange: rastok::connector::ConnectorExchange::Call,
        ports: &[PortContract {
            name: "out",
            direction: Direction::Output,

            ty: PortTypeRule::AttributeSelected {
                attribute: "shape",
                choices: &[PortTypeChoice {
                    word: "text",
                    ty: PortDataType::Text,
                }],
            },
        }],
        common: SHAPE,
        selector: None,
    };
    assert_eq!(
        ContractCatalog::try_m0()
            .expect("the standard catalogue holds")
            .try_with(LATE)
            .unwrap_err(),
        SheetDefect::LatePortTypeAttribute {
            port: "out",
            attribute: "shape",
        }
    );
}

#[test]
fn a_call_contract_requires_its_explicit_terminal() {
    const INVALID: ConnectorContract = ConnectorContract {
        id: "test/invalid-sink@1",
        name: "InvalidSink",
        roles: rastok::connector::ConnectorRoles::Outbound,
        exchange: rastok::connector::ConnectorExchange::Call,
        ports: &[PortContract {
            name: "in",
            direction: Direction::Input,

            ty: PortTypeRule::Fixed(PortDataType::Variant),
        }],
        common: &[],
        selector: None,
    };
    assert_eq!(
        ContractCatalog::try_m0()
            .expect("the standard catalogue holds")
            .try_with(INVALID)
            .unwrap_err(),
        SheetDefect::ExchangeTerminalMismatch {
            exchange: rastok::connector::ConnectorExchange::Call,
            exits: 0
        }
    );
}

#[test]
fn process_codec_choices_resolve_the_current_execution_pins() {
    for codec in ["lines", "jsonl", "bytes"] {
        for err_codec in ["lines", "bytes"] {
            let ports = rastok::stdlib::PROCESS
                .resolve_ports(&std::collections::BTreeMap::from([
                    (String::from("codec"), String::from(codec)),
                    (String::from("err_codec"), String::from(err_codec)),
                ]))
                .expect("every sheet choice resolves");
            for port in ports {
                let sheet = port.ty.expect("the promised port is typed");
                let core = rastok::boundary::process_port_type(
                    &rastok::connector::ContractId::parse(rastok::connector::PROCESS_CONNECTOR_ID)
                        .unwrap(),
                    port.port.name(),
                    Some(codec),
                    Some(err_codec),
                )
                .expect("the core mirror recognizes every sheet row");
                assert_eq!(sheet, core, "{codec}/{err_codec} on {}", port.port);
            }
        }
    }
}

#[test]
fn file_profiles_resolve_the_same_rows_in_the_sheet_and_core_mirror() {
    let id = rastok::connector::ContractId::parse(rastok::connector::FILE_CONNECTOR_ID)
        .expect("the standard File id");
    for mode in ["read", "replace"] {
        let ports = rastok::stdlib::FILE
            .resolve_ports(&std::collections::BTreeMap::from([(
                String::from("mode"),
                String::from(mode),
            )]))
            .expect("every executable File profile resolves");
        for port in ports {
            assert_eq!(
                port.ty,
                rastok::boundary::file_port_type(&id, port.port.name(), Some(mode)),
                "{mode} on {}",
                port.port
            );
        }
    }
    for attribute in ["mode", "path", "effect", "behavior", "timeout", "max_read"] {
        let profile = rastok::stdlib::FILE
            .profile("read")
            .expect("the read profile");
        let sheet_site = if attribute == "mode" {
            Some(EvaluationSite::Plan)
        } else {
            rastok::stdlib::FILE
                .attribute(Some(profile), attribute)
                .map(|spec| spec.latest_site)
        };
        assert_eq!(
            sheet_site,
            rastok::boundary::file_attribute_site(&id, attribute),
            "the Registry-free phase mirror drifted for `{attribute}`"
        );
    }
}

#[test]
fn a_file_path_closes_at_invocation_and_defaults_travel() {
    let source = file_graph("    mode = read\n    path = data as text");
    let compilation = compile_one(&source).expect("the File read contract checks");
    let spec = compilation.plan.plan().nodes[0]
        .connector()
        .expect("the File connector");
    assert_eq!(spec.contract.as_str(), rastok::connector::FILE_CONNECTOR_ID);
    assert_eq!(
        spec.attribute("max_read"),
        Some(&rastok::plan::AttrSetting::Value(
            rastok::value::Value::Text(String::from("8MiB"))
        ))
    );
    assert!(matches!(
        spec.attribute("path"),
        Some(rastok::plan::AttrSetting::Expr {
            site: EvaluationSite::Invocation,
            expr,
        }) if expr.ty == rastok::datatype::DataType::Text
    ));
    let bytes = rastok::plan::encode(&compilation.plan, &compilation.hash);
    let (decoded, _) = rastok::plan::decode(&bytes).expect("the dynamic path round-trips");
    assert_eq!(*decoded, *compilation.plan);
}

#[test]
fn file_replace_has_bytes_input_and_no_read_budget() {
    let source = file_graph(
        "    mode = replace\n    path = \"artifact.bin\"\n    timeout = duration(\"0s\")",
    );
    let compilation = compile_one(&source).expect("the File replace contract checks");
    let node = &compilation.plan.plan().nodes[0];
    let spec = node.connector().expect("the File connector");
    assert!(spec.attribute("max_read").is_none());
    assert_eq!(
        node.ports
            .iter()
            .find(|port| port.port.name() == "in")
            .and_then(|port| port.ty.as_ref()),
        Some(&rastok::datatype::DataType::BYTES)
    );
}

#[test]
fn unsupported_file_surfaces_refuse_by_name() {
    refusal_with(
        &file_graph("    mode = watch\n    path = \"artifact.bin\""),
        "E-CHECK-DEFERRED",
        "the File watch profile",
    );
    refusal_with(
        &file_graph("    mode = list\n    path = \"artifact.bin\""),
        "E-CONTRACT-ATTR",
        "expected one of",
    );
    for attribute in ["recursive", "create_parents"] {
        refusal_with(
            &file_graph(&format!(
                "    mode = read\n    path = \"artifact.bin\"\n    {attribute} = true"
            )),
            "E-CONTRACT-ATTR",
            "not on the contract sheet",
        );
    }
}

#[test]
fn file_intent_and_budgets_are_never_guessed() {
    refusal_with(
        &file_graph("    path = \"artifact.bin\""),
        "E-CONTRACT-REQUIRED",
        "mode",
    );
    refusal_with(
        &file_graph("    mode = read"),
        "E-CONTRACT-REQUIRED",
        "path",
    );
    refusal_with(
        &file_graph("    mode = read\n    path = 1"),
        "E-PORT-TYPE",
        "`path` requires text",
    );
    refusal_with(
        &file_graph("    mode = read\n    path = \"artifact.bin\"\n    max_read = \"many\""),
        "E-CONTRACT-ATTR",
        "does not fit",
    );
}

#[test]
fn a_forged_file_sheet_never_becomes_a_valid_plan() {
    let source = file_graph("    mode = read\n    path = \"artifact.bin\"");
    let compilation = compile_one(&source).expect("the File read contract checks");
    let mut plan: rastok::plan::ExecutionPlan = (*compilation.plan).clone();
    let rastok::plan::NodeExecution::Connector(spec) = &mut plan.nodes[0].execution else {
        panic!("the compiled node is the File connector")
    };
    spec.attributes.retain(|(name, _)| name != "path");
    assert!(matches!(
        rastok::plan::ValidatedExecutionPlan::try_from(plan),
        Err(
            rastok::plan::PlanValidationError::MissingContractAttribute {
                attribute: "path",
                ..
            }
        )
    ));

    let dynamic = file_graph("    mode = read\n    path = data as text");
    let compilation = compile_one(&dynamic).expect("the dynamic File path checks");
    let mut plan: rastok::plan::ExecutionPlan = (*compilation.plan).clone();
    let rastok::plan::NodeExecution::Connector(spec) = &mut plan.nodes[0].execution else {
        panic!("the compiled node is the File connector")
    };
    let Some(rastok::plan::AttrSetting::Expr { site, .. }) = spec
        .attributes
        .iter_mut()
        .find(|(name, _)| name == "path")
        .map(|(_, setting)| setting)
    else {
        panic!("the dynamic path travels as an expression")
    };
    *site = EvaluationSite::Instance;
    assert!(matches!(
        rastok::plan::ValidatedExecutionPlan::try_from(plan),
        Err(rastok::plan::PlanValidationError::BindingSiteMismatch { name, .. })
            if name == "path"
    ));
}

#[test]
fn a_duplicate_profile_is_a_sheet_defect() {
    const TWICE: ProfileSelector = ProfileSelector {
        attribute: "mode",
        default: Some("a"),
        profiles: &[
            Profile {
                protocol: None,
                word: "a",
                support: ProfileSupport::Available,
                attributes: &[],
            },
            Profile {
                protocol: None,
                word: "a",
                support: ProfileSupport::Available,
                attributes: &[],
            },
        ],
    };
    assert_eq!(
        ContractCatalog::try_m0()
            .expect("the standard catalogue holds")
            .try_with(sheet(&[], Some(TWICE)))
            .unwrap_err(),
        SheetDefect::DuplicateProfile("a")
    );
}

#[test]
fn an_unknown_default_profile_is_a_sheet_defect() {
    const GHOST: ProfileSelector = ProfileSelector {
        attribute: "mode",
        default: Some("ghost"),
        profiles: &[Profile {
            protocol: None,
            word: "a",
            support: ProfileSupport::Available,
            attributes: &[],
        }],
    };
    assert_eq!(
        ContractCatalog::try_m0()
            .expect("the standard catalogue holds")
            .try_with(sheet(&[], Some(GHOST)))
            .unwrap_err(),
        SheetDefect::UnknownDefaultProfile("ghost")
    );
}

#[test]
fn a_selector_colliding_with_an_attribute_is_a_sheet_defect() {
    const MODE: &[AttributeSpec] = &[AttributeSpec {
        name: "mode",
        presence: Presence::Optional,
        domain: ValueDomain::Text,
        latest_site: EvaluationSite::Plan,
    }];
    const SELECTOR: ProfileSelector = ProfileSelector {
        attribute: "mode",
        default: Some("a"),
        profiles: &[Profile {
            protocol: None,
            word: "a",
            support: ProfileSupport::Available,
            attributes: &[],
        }],
    };
    assert_eq!(
        ContractCatalog::try_m0()
            .expect("the standard catalogue holds")
            .try_with(sheet(MODE, Some(SELECTOR)))
            .unwrap_err(),
        SheetDefect::SelectorCollision("mode")
    );
}

#[test]
fn an_uncompilable_expression_default_is_a_sheet_defect() {
    const BAD: &[AttributeSpec] = &[AttributeSpec {
        name: "x",
        presence: Presence::Defaulted(DefaultValue::Expression("] not expr [")),
        domain: ValueDomain::Text,
        latest_site: EvaluationSite::Plan,
    }];
    assert!(matches!(
        ContractCatalog::try_m0()
            .expect("the standard catalogue holds")
            .try_with(sheet(BAD, None))
            .unwrap_err(),
        SheetDefect::InvalidDefault { attribute: "x", .. }
    ));
}

#[test]
fn a_default_outside_its_domain_is_a_sheet_defect() {
    // `1` compiles and reduces — but it is no Text.
    const MISFIT: &[AttributeSpec] = &[AttributeSpec {
        name: "x",
        presence: Presence::Defaulted(DefaultValue::Expression("1")),
        domain: ValueDomain::Text,
        latest_site: EvaluationSite::Plan,
    }];
    assert!(matches!(
        ContractCatalog::try_m0()
            .expect("the standard catalogue holds")
            .try_with(sheet(MISFIT, None))
            .unwrap_err(),
        SheetDefect::InvalidDefault { attribute: "x", .. }
    ));
    // A Choice default must be a word of the domain.
    const OUTSIDE: &[AttributeSpec] = &[AttributeSpec {
        name: "y",
        presence: Presence::Defaulted(DefaultValue::Choice("ghost")),
        domain: ValueDomain::Choice(&["real"]),
        latest_site: EvaluationSite::Plan,
    }];
    assert!(matches!(
        ContractCatalog::try_m0()
            .expect("the standard catalogue holds")
            .try_with(sheet(OUTSIDE, None))
            .unwrap_err(),
        SheetDefect::InvalidDefault { attribute: "y", .. }
    ));
}

#[test]
fn a_duplicate_choice_word_is_a_sheet_defect() {
    const DUP: &[AttributeSpec] = &[AttributeSpec {
        name: "x",
        presence: Presence::Optional,
        domain: ValueDomain::Choice(&["a", "a"]),
        latest_site: EvaluationSite::Plan,
    }];
    assert_eq!(
        ContractCatalog::try_m0()
            .expect("the standard catalogue holds")
            .try_with(sheet(DUP, None))
            .unwrap_err(),
        SheetDefect::DuplicateChoice("x")
    );
}

#[test]
fn an_expression_path_is_an_executable_invocation_domain() {
    const RAW: &[AttributeSpec] = &[AttributeSpec {
        name: "correlate",
        presence: Presence::Optional,
        domain: ValueDomain::ExprPath,
        latest_site: EvaluationSite::Invocation,
    }];
    ContractCatalog::try_m0()
        .expect("the standard catalogue holds")
        .try_with(sheet(RAW, None))
        .expect("a direct invocation path is an implemented domain");
}

// ---------- the dead spellings refuse with their migration ----------

#[test]
fn a_configuration_sub_block_names_its_migration() {
    let source = process_graph(&format!("{FULL}\n    env {{ PATH = \"/bin\" }}"));
    refusal_with(
        &source,
        "E-CONTRACT-ATTR",
        "`env { … }` is no longer a configuration form; write `env = { … }`",
    );
}

#[test]
fn a_tagged_initializer_names_its_migration() {
    let source = process_graph(&format!(
        "{FULL}\n    restart = on_crash {{ backoff = \"1s\" }}"
    ));
    refusal_with(
        &source,
        "E-CONTRACT-ATTR",
        "dead tagged-initializer spelling",
    );
}

#[test]
fn a_dotted_attribute_is_not_a_composite_patch() {
    // `tls.verify` names no owner — it refuses whole, never patches
    // a `tls` composite.
    let source = process_graph(&format!("{FULL}\n    tls.verify = true"));
    refusal_with(&source, "E-CONTRACT-ATTR", "not on the contract sheet");
}

// ---------- the Process profiles ----------

#[test]
fn mode_omitted_and_mode_job_are_one_identity() {
    let omitted = compile_one(&process_graph(FULL)).expect("valid");
    let written = compile_one(&process_graph(&format!("    mode = job\n{FULL}"))).expect("valid");
    assert_eq!(
        omitted.hash.to_string(),
        written.hash.to_string(),
        "omission of a Defaulted selector IS its exact spelling"
    );
    assert_eq!(*omitted.plan, *written.plan, "one effective plan");
}

#[test]
fn every_default_travels_in_the_plan() {
    let compilation = compile_one(&process_graph(FULL)).expect("valid");
    let spec = compilation.plan.plan().nodes[0]
        .connector()
        .expect("a connector");
    let get = |name: &str| spec.attribute(name).cloned();
    assert_eq!(
        get("mode"),
        Some(rastok::plan::AttrSetting::Word(String::from("job")))
    );
    assert_eq!(
        get("on_timeout"),
        Some(rastok::plan::AttrSetting::Word(String::from("kill")))
    );
    assert_eq!(
        get("err_codec"),
        Some(rastok::plan::AttrSetting::Word(String::from("lines")))
    );
    assert_eq!(
        get("max_output"),
        Some(rastok::plan::AttrSetting::Value(
            rastok::value::Value::Text(String::from("8MiB"))
        ))
    );
    assert_eq!(
        get("args"),
        Some(rastok::plan::AttrSetting::Value(
            rastok::value::Value::Array(Vec::new())
        ))
    );
}

#[test]
fn defaults_survive_the_package_round_trip() {
    let compilation = compile_one(&process_graph(FULL)).expect("valid");
    let bytes = rastok::plan::encode(&compilation.plan, &compilation.hash);
    let (decoded, _) = rastok::plan::decode(&bytes).expect("the package decodes");
    let spec = decoded.plan().nodes[0].connector().expect("a connector");
    assert!(
        spec.attribute("max_output").is_some(),
        "the default travels"
    );
    assert!(spec.attribute("args").is_some());
    assert!(spec.attribute("on_timeout").is_some());
    assert!(spec.attribute("err_codec").is_some());
}

#[test]
fn writing_a_default_exactly_is_the_same_identity() {
    let omitted = compile_one(&process_graph(FULL)).expect("valid");
    let written = compile_one(&process_graph(&format!(
        "{FULL}\n    args = []\n    err_codec = lines\n    max_output = \"8MiB\"\n    on_timeout = kill\n    mode = job"
    )))
    .expect("valid");
    assert_eq!(omitted.hash.to_string(), written.hash.to_string());
    assert_eq!(*omitted.plan, *written.plan);
}

#[test]
fn args_null_refuses_as_a_type() {
    // Omission resolves to `[]`; `null` is a real, distinct Rastok expression
    // value and the domain is not nullable.
    let source = process_graph(&format!("{FULL}\n    args = null"));
    refusal_with(
        &source,
        "E-CONTRACT-ATTR",
        "does not fit the TextList domain",
    );
}

#[test]
fn codec_stays_required() {
    let source =
        process_graph("    command = \"cat\"\n    idempotent = true ; behavior = deterministic");
    let bag = compile_one(&source).expect_err("must refuse");
    assert!(
        bag.diagnostics()
            .iter()
            .any(|d| d.code().as_str() == "E-CONTRACT-REQUIRED" && d.message().contains("`codec`")),
        "{bag:?}"
    );
}

#[test]
fn bytes_framing_is_contract_valid_and_binds_in_b7() {
    for (attribute, body) in [
        (
            "codec",
            "    command = \"cat\" ; codec = bytes\n    idempotent = true ; behavior = deterministic"
                .to_string(),
        ),
        ("err_codec", format!("{FULL}\n    err_codec = bytes")),
    ] {
        let compilation = compile_one(&process_graph(&body))
            .unwrap_or_else(|bag| panic!("`{attribute} = bytes` is on the sheet: {bag:?}"));
        rastok::connectors::Process::try_new(&compilation.plan)
            .unwrap_or_else(|error| panic!("`{attribute} = bytes` binds in B7: {error}"));
    }
}

#[test]
fn a_choice_written_as_expression_text_names_the_bare_word() {
    let source = process_graph(
        "    mode = \"job\"\n    command = \"cat\" ; codec = lines\n    idempotent = true ; behavior = deterministic",
    );
    refusal_with(&source, "E-CONTRACT-ATTR", "bare word");
}

#[test]
fn an_expression_domain_refuses_a_bare_word() {
    let source = process_graph(
        "    command = cat ; codec = lines\n    idempotent = true ; behavior = deterministic",
    );
    refusal_with(&source, "E-CONTRACT-ATTR", "`cat` is a bare word");
}

#[test]
fn on_timeout_bypass_says_fixed_to_kill() {
    let source = process_graph(&format!("{FULL}\n    on_timeout = bypass"));
    refusal_with(
        &source,
        "E-CONTRACT-ATTR",
        "`on_timeout` is fixed to `kill` for `Process(mode = job)`",
    );
}

#[test]
fn a_service_attribute_on_a_job_names_the_owning_profile() {
    let source = process_graph(&format!("{FULL}\n    max_frame = \"1MiB\""));
    refusal_with(
        &source,
        "E-CONTRACT-ATTR",
        "`max_frame` belongs to `Process(mode = service)` and is not applicable to `Process(mode = job)`",
    );
}

#[test]
fn mode_service_rejects_retired_rpc_attributes_by_name() {
    let source = process_graph(&format!(
        "    mode = service\n{FULL}\n    correlate = data.id"
    ));
    let bag = compile_one(&source).expect_err("the retired correlate setting refuses");
    assert!(
        bag.diagnostics()
            .iter()
            .any(|d| d.code().as_str() == "E-CONTRACT-ATTR"
                && d.message().contains("`correlate` is retired for Process")),
        "{bag:?}"
    );
    // No whole-profile deferral hides the offending field.
    assert!(
        !bag.diagnostics()
            .iter()
            .any(|d| d.code().as_str() == "E-CHECK-DEFERRED"
                && d.message().contains("the Process service mode")),
        "{bag:?}"
    );
}

/// Mutates the compiled Process sheet. Registry-free plan validation is the
/// first refusal; the Driver keeps the same exhaustive conversion as its belt.
fn process_plan_error(mutate: impl Fn(&mut rastok::plan::ConnectorSpec)) -> String {
    let compilation = compile_one(&process_graph(FULL)).expect("valid");
    let mut plan: rastok::plan::ExecutionPlan = (*compilation.plan).clone();
    for node in &mut plan.nodes {
        if let rastok::plan::NodeExecution::Connector(spec) = &mut node.execution {
            mutate(spec);
        }
    }
    rastok::plan::ValidatedExecutionPlan::try_from(plan)
        .expect_err("the plan validator refuses the forged Process sheet")
        .to_string()
}

#[test]
fn the_plan_gate_refuses_every_stripped_default_individually() {
    // Every effective default is defended before binding or execution.
    for name in ["mode", "on_timeout", "args", "max_output"] {
        let message = process_plan_error(|spec| {
            spec.attributes.retain(|(attr, _)| attr != name);
        });
        assert!(
            message.contains("omits required effective attribute"),
            "`{name}` names the violated invariant: {message}"
        );
    }
}

#[test]
fn the_plan_gate_drops_no_argument_silently() {
    let message = process_plan_error(|spec| {
        for (name, setting) in &mut spec.attributes {
            if name == "args" {
                *setting = rastok::plan::AttrSetting::Value(rastok::value::Value::Array(vec![
                    rastok::value::Value::Text(String::from("ok")),
                    rastok::value::Value::Int(7),
                ]));
            }
        }
    });
    assert!(message.contains("invalid effective `args`"), "{message}");
}

#[test]
fn the_plan_gate_rechecks_optional_process_settings() {
    let bad_cwd = process_plan_error(|spec| {
        spec.attributes.push((
            String::from("cwd"),
            rastok::plan::AttrSetting::Value(rastok::value::Value::Int(7)),
        ));
        spec.attributes.sort_by(|a, b| a.0.cmp(&b.0));
    });
    assert!(bad_cwd.contains("invalid effective `cwd`"), "{bad_cwd}");

    let bad_env = process_plan_error(|spec| {
        spec.attributes.push((
            String::from("env"),
            rastok::plan::AttrSetting::Value(rastok::value::Value::Map(
                [(String::from("COUNT"), rastok::value::Value::Int(7))]
                    .into_iter()
                    .collect(),
            )),
        ));
        spec.attributes.sort_by(|a, b| a.0.cmp(&b.0));
    });
    assert!(bad_env.contains("invalid effective `env`"), "{bad_env}");
}

#[test]
fn an_unreachable_process_still_carries_one_valid_sheet() {
    let compilation = compile_one(&process_graph(FULL)).expect("valid");
    let mut plan: rastok::plan::ExecutionPlan = (*compilation.plan).clone();
    let connector_slot = rastok::plan::Slot::new(
        u32::try_from(
            plan.nodes
                .iter()
                .position(|node| {
                    matches!(node.execution, rastok::plan::NodeExecution::Connector(_))
                })
                .expect("the Process exists"),
        )
        .expect("the plan is small"),
    );
    plan.edges.retain(|edge| {
        edge.to
            .as_ref()
            .is_none_or(|to| to.node != Some(connector_slot))
    });
    plan.boundary.retain(|port| port.port.name() != "in");
    for node in &mut plan.nodes {
        if let rastok::plan::NodeExecution::Connector(spec) = &mut node.execution {
            for (name, setting) in &mut spec.attributes {
                if name == "command" {
                    *setting = rastok::plan::AttrSetting::Value(rastok::value::Value::Int(3));
                }
            }
        }
    }

    assert!(matches!(
        rastok::plan::ValidatedExecutionPlan::try_from(plan),
        Err(rastok::plan::PlanValidationError::ContractAttributeMismatch {
            attribute,
            ..
        }) if attribute == "command"
    ));
}

#[test]
fn a_forged_process_cannot_smuggle_an_unknown_attribute() {
    let compilation = compile_one(&process_graph(FULL)).expect("valid");
    let mut plan: rastok::plan::ExecutionPlan = (*compilation.plan).clone();
    for node in &mut plan.nodes {
        if let rastok::plan::NodeExecution::Connector(spec) = &mut node.execution {
            spec.attributes.push((
                String::from("mystery"),
                rastok::plan::AttrSetting::Value(rastok::value::Value::Text(String::from(
                    "hidden",
                ))),
            ));
            spec.attributes.sort_by(|left, right| left.0.cmp(&right.0));
        }
    }

    assert!(matches!(
        rastok::plan::ValidatedExecutionPlan::try_from(plan),
        Err(rastok::plan::PlanValidationError::ContractAttributeMismatch {
            attribute,
            ..
        }) if attribute == "mystery"
    ));
}

#[test]
fn the_global_binder_requires_callable_connectors_but_skips_pure_outbound_sources() {
    // A callable connector needs its exact Driver family before execution. A
    // pure outbound-only source has no incoming edge and can never open a
    // call, so it creates no runtime obligation.
    let compilation = compile_one(&process_graph(FULL)).expect("valid");
    let mut plan: rastok::plan::ExecutionPlan = (*compilation.plan).clone();
    for node in &mut plan.nodes {
        if let rastok::plan::NodeExecution::Connector(spec) = &mut node.execution {
            spec.contract = rastok::connector::ContractId::parse("test/custom@1").unwrap();
            spec.idempotent = None;
            spec.attributes.retain(|(name, _)| name != "idempotent");
            node.ports = vec![
                rastok::plan::PortDecl {
                    port: rastok::node::Port::new("in").expect("a conventional port"),
                    direction: rastok::node::Direction::Input,

                    ty: Some(rastok::datatype::DataType::Text),
                    annotations: Default::default(),
                },
                rastok::plan::PortDecl {
                    port: rastok::node::Port::new("out").expect("a conventional port"),
                    direction: rastok::node::Direction::Output,

                    ty: Some(rastok::datatype::DataType::Text),
                    annotations: Default::default(),
                },
                rastok::plan::PortDecl {
                    port: rastok::node::Port::new("err").expect("a conventional port"),
                    direction: rastok::node::Direction::Output,

                    ty: Some(rastok::datatype::DataType::Variant),
                    annotations: Default::default(),
                },
                rastok::plan::PortDecl {
                    port: rastok::node::Port::new("exit").expect("a conventional port"),
                    direction: rastok::node::Direction::Output,

                    ty: Some(rastok::datatype::DataType::Variant),
                    annotations: Default::default(),
                },
            ];
            node.ports
                .sort_by(|left, right| left.key().cmp(&right.key()));
        }
    }
    let callable = rastok::plan::ValidatedExecutionPlan::try_from(plan.clone())
        .expect("the foreign connector is structurally typed");
    let registry = rastok::connectors::DriverRegistry::default();
    let message = rastok::connectors::RuntimeBindings::try_new(&callable, &registry)
        .expect_err("the callable foreign connector refuses")
        .to_string();
    assert!(
        message.contains("no Driver is registered for `test/custom@1`"),
        "{message}"
    );
    // Sever every edge DELIVERING to the connector: a pure source.
    // The slot is DERIVED from the node, never hard-coded to the
    // lowering order.
    let slot = rastok::plan::Slot::new(
        u32::try_from(
            plan.nodes
                .iter()
                .position(|node| {
                    matches!(&node.execution, rastok::plan::NodeExecution::Connector(_))
                })
                .expect("the connector exists"),
        )
        .expect("a small plan"),
    );
    plan.edges
        .retain(|edge| edge.to.as_ref().is_none_or(|to| to.node != Some(slot)));
    plan.boundary.retain(|port| port.port.name() != "in");
    let validated = rastok::plan::ValidatedExecutionPlan::try_from(plan)
        .expect("the foreign pure source remains structurally valid");
    assert!(
        rastok::connectors::RuntimeBindings::try_new(&validated, &registry).is_ok(),
        "a pure outbound source requires no binding"
    );
    // An unknown codec cannot even reach the binding: the plan
    // validator refuses it first (the Driver's exhaustive match is
    // the belt BEHIND that door).
    let compilation = compile_one(&process_graph(FULL)).expect("valid");
    let mut plan: rastok::plan::ExecutionPlan = (*compilation.plan).clone();
    for node in &mut plan.nodes {
        if let rastok::plan::NodeExecution::Connector(spec) = &mut node.execution {
            for (name, setting) in &mut spec.attributes {
                if name == "codec" {
                    *setting = rastok::plan::AttrSetting::Word(String::from("bogus"));
                }
            }
        }
    }
    assert!(
        matches!(
            rastok::plan::ValidatedExecutionPlan::try_from(plan),
            Err(rastok::plan::PlanValidationError::UnknownCodec { .. })
        ),
        "the validator speaks before the binding ever could"
    );
}

#[test]
fn a_call_without_its_plan_owned_guarantee_never_validates() {
    let compilation = compile_one(&process_graph(FULL)).expect("valid");
    let mut plan: rastok::plan::ExecutionPlan = (*compilation.plan).clone();
    for node in &mut plan.nodes {
        if let rastok::plan::NodeExecution::Connector(spec) = &mut node.execution {
            spec.idempotent = None;
        }
    }
    assert!(
        matches!(
            rastok::plan::ValidatedExecutionPlan::try_from(plan),
            Err(rastok::plan::PlanValidationError::ContractAttributeMismatch { .. })
        ),
        "a Call needs its re-derived original plan guarantee"
    );
}

#[test]
fn a_delivery_target_requires_outbound_or_an_inbound_call_response_role() {
    let compilation = compile_one(&process_graph(FULL)).expect("valid");
    let mut plan: rastok::plan::ExecutionPlan = (*compilation.plan).clone();
    for node in &mut plan.nodes {
        if let rastok::plan::NodeExecution::Connector(spec) = &mut node.execution {
            spec.contract = rastok::connector::ContractId::parse("test/inbound-only@1")
                .expect("the test id is canonical");
            spec.roles = rastok::connector::ConnectorRoles::Inbound;
            spec.exchange = rastok::connector::ConnectorExchange::Event;
        }
    }
    assert!(matches!(
        rastok::plan::ValidatedExecutionPlan::try_from(plan),
        Err(rastok::plan::PlanValidationError::CallableConnectorWithoutOutboundRole { .. })
    ));
}

#[test]
fn a_forged_event_cannot_claim_an_outbound_effect_role() {
    let compilation = compile_one(&process_graph(FULL)).expect("valid");
    let mut plan: rastok::plan::ExecutionPlan = (*compilation.plan).clone();
    for node in &mut plan.nodes {
        if let rastok::plan::NodeExecution::Connector(spec) = &mut node.execution {
            spec.contract = rastok::connector::ContractId::parse("test/outbound-event@1")
                .expect("the test id is canonical");
            spec.exchange = rastok::connector::ConnectorExchange::Event;
            node.ports.retain(|port| port.port.name() != "exit");
        }
    }
    assert!(matches!(
        rastok::plan::ValidatedExecutionPlan::try_from(plan),
        Err(rastok::plan::PlanValidationError::ExchangeRoleMismatch {
            exchange: rastok::connector::ConnectorExchange::Event,
            roles: rastok::connector::ConnectorRoles::Outbound,
            ..
        })
    ));
}

#[test]
fn byte_size_is_target_independent_and_bounded() {
    use rastok::contract::{MAX_BYTE_SIZE, byte_size};
    assert_eq!(byte_size("4294967295B"), Some(u64::from(u32::MAX)));
    assert_eq!(
        byte_size("4294967296B"),
        Some(4 * 1024 * 1024 * 1024),
        "u32::MAX + 1 fits — the parse is u64 on EVERY machine"
    );
    assert_eq!(byte_size("4GiB"), Some(MAX_BYTE_SIZE), "the bound itself");
    assert_eq!(byte_size("4294967297B"), None, "one beyond refuses");
    assert_eq!(byte_size("5GiB"), None);
    assert_eq!(
        byte_size("999999999999999999999GiB"),
        None,
        "overflow refuses, never wraps"
    );
    assert_eq!(
        byte_size("8 MiB"),
        Some(8 * 1024 * 1024),
        "interior space tolerated"
    );
    assert_eq!(byte_size("0B"), Some(0), "a zero budget is declarable");
}

#[test]
fn an_unsatisfiable_sheet_never_enters_the_registry() {
    // An empty choice, a selector with no profile, a `nothing`
    // choice word, an inexpressible contract name — each refuses at
    // the door.
    const EMPTY: &[AttributeSpec] = &[AttributeSpec {
        name: "x",
        presence: Presence::Optional,
        domain: ValueDomain::Choice(&[]),
        latest_site: EvaluationSite::Plan,
    }];
    assert_eq!(
        ContractCatalog::try_m0()
            .expect("the standard catalogue holds")
            .try_with(sheet(EMPTY, None))
            .unwrap_err(),
        SheetDefect::EmptyChoice("x")
    );
    const NOBODY: ProfileSelector = ProfileSelector {
        attribute: "mode",
        default: None,
        profiles: &[],
    };
    assert_eq!(
        ContractCatalog::try_m0()
            .expect("the standard catalogue holds")
            .try_with(sheet(&[], Some(NOBODY)))
            .unwrap_err(),
        SheetDefect::NoProfiles("mode")
    );
    const SENTINEL: &[AttributeSpec] = &[AttributeSpec {
        name: "x",
        presence: Presence::Optional,
        domain: ValueDomain::Choice(&["nothing"]),
        latest_site: EvaluationSite::Plan,
    }];
    assert_eq!(
        ContractCatalog::try_m0()
            .expect("the standard catalogue holds")
            .try_with(sheet(SENTINEL, None))
            .unwrap_err(),
        SheetDefect::InexpressibleName("nothing"),
        "the parser turns `nothing` into the no-emission sentinel"
    );
    let unnameable = ConnectorContract {
        name: "Not A Name",
        ..sheet(&[], None)
    };
    assert_eq!(
        ContractCatalog::try_m0()
            .expect("the standard catalogue holds")
            .try_with(unnameable)
            .unwrap_err(),
        SheetDefect::InexpressibleName("Not A Name")
    );
}

#[test]
fn the_new_discriminant_words_parse_as_words() {
    // The review's blocker: `exponential` INSIDE a struct is a bare
    // word, never a resurrected dead-initializer refusal — the form
    // (word alone vs word + `(`/`{`) decides.
    let source = process_graph(&format!("{FULL}\n    bogus = {{ curve = exponential }}"));
    let bag = compile_one(&source).expect_err("bogus is not on the sheet");
    assert!(
        !bag.diagnostics()
            .iter()
            .any(|d| d.code().as_str() == "E-PARSE-EXPECTED"),
        "the parser preempts nothing: {bag:?}"
    );
    assert!(
        bag.diagnostics()
            .iter()
            .any(|d| d.code().as_str() == "E-CONTRACT-ATTR"
                && d.message().contains("not on the contract sheet")),
        "{bag:?}"
    );
}

#[test]
fn a_bare_tag_word_is_judged_by_the_contract() {
    // `codec = json`: `json` is a RESERVED word, but the word ALONE is a
    // choice for the contract to judge — it must never eat the next
    // line as a JSON value.
    let source = process_graph(
        "    command = \"cat\" ; codec = json\n    idempotent = true ; behavior = deterministic",
    );
    refusal_with(
        &source,
        "E-CONTRACT-ATTR",
        "expected one of [\"bytes\", \"jsonl\", \"lines\"], got `json`",
    );
}

#[test]
fn tag_words_serve_as_choice_words_on_another_sheet() {
    // A future sheet may spell `text`, `json` or `unbounded` as
    // closed choices — the parser leaves the words to the contract.
    const CHOICES: &[AttributeSpec] = &[
        AttributeSpec {
            name: "behavior",
            presence: Presence::Required,
            domain: ValueDomain::Choice(&["deterministic"]),
            latest_site: EvaluationSite::Plan,
        },
        AttributeSpec {
            name: "framing",
            presence: Presence::Required,
            domain: ValueDomain::Choice(&["text", "json", "unbounded"]),
            latest_site: EvaluationSite::Plan,
        },
    ];
    let registry = ContractCatalog::try_m0()
        .expect("the standard catalogue holds")
        .try_with(sheet(CHOICES, None))
        .expect("a valid sheet");
    for word in ["text", "json", "unbounded"] {
        let source = format!(
            "graph g : App {{\n  input in: variant\n  output out: variant\n  connector p : Probe {{\n    framing = {word}\n    behavior = deterministic\n  }}\n  in -> p -> out\n  p.err -> nothing\n  p.exit -> nothing\n}}\n"
        );
        let result = compile(&[source.as_str()], &registry);
        assert!(result.is_ok(), "`framing = {word}` compiles: {result:?}");
    }
}

#[test]
fn max_output_is_judged_at_check_not_at_spawn() {
    // The ByteSize domain closes the review's false green: a
    // non-size refuses at CHECK, through the same parser the
    // Driver enforces the budget with.
    let source = process_graph(&format!("{FULL}\n    max_output = \"banana\""));
    refusal_with(&source, "E-CONTRACT-ATTR", "ByteSize");
    assert_eq!(rastok::contract::byte_size("8MiB"), Some(8 * 1024 * 1024));
    assert_eq!(rastok::contract::byte_size("banana"), None);
    assert_eq!(
        rastok::contract::byte_size("1024"),
        None,
        "an explicit unit is required"
    );
}

#[test]
fn process_declares_the_phase_ceiling_of_every_setting() {
    use rastok::stdlib::PROCESS;

    for name in [
        "codec",
        "err_codec",
        "idempotent",
        "behavior",
        "max_output",
        "on_timeout",
    ] {
        assert_eq!(
            PROCESS
                .attribute(PROCESS.profile("job"), name)
                .expect("the Process row")
                .latest_site,
            EvaluationSite::Plan,
            "{name} shapes the plan"
        );
    }
    for name in ["command", "args", "cwd", "timeout", "env"] {
        assert_eq!(
            PROCESS
                .attribute(PROCESS.profile("job"), name)
                .expect("the Process row")
                .latest_site,
            EvaluationSite::Instance,
            "{name} may consume per-instance immutable context"
        );
    }
    assert!(EvaluationSite::Instance.admits(EvaluationSite::Plan));
    assert!(!EvaluationSite::Plan.admits(EvaluationSite::Instance));
}

#[test]
fn the_core_phase_mirror_recognizes_every_process_sheet_row() {
    use rastok::stdlib::PROCESS;

    for spec in PROCESS.common {
        assert_eq!(
            rastok::boundary::process_attribute_site(
                &rastok::connector::ContractId::parse(rastok::connector::PROCESS_CONNECTOR_ID,)
                    .unwrap(),
                spec.name,
            ),
            Some(spec.latest_site),
            "common attribute {}",
            spec.name
        );
    }
    let selector = PROCESS.selector.expect("Process has profiles");
    assert_eq!(
        rastok::boundary::process_attribute_site(
            &rastok::connector::ContractId::parse(rastok::connector::PROCESS_CONNECTOR_ID).unwrap(),
            selector.attribute,
        ),
        Some(EvaluationSite::Plan),
        "the profile selector shapes the plan"
    );
    for profile in selector.profiles {
        for spec in profile.attributes {
            assert_eq!(
                rastok::boundary::process_attribute_site(
                    &rastok::connector::ContractId::parse(rastok::connector::PROCESS_CONNECTOR_ID,)
                        .unwrap(),
                    spec.name,
                ),
                Some(spec.latest_site),
                "{} attribute {}",
                profile.word,
                spec.name
            );
        }
    }
    assert_eq!(
        rastok::boundary::process_attribute_site(
            &rastok::connector::ContractId::parse(rastok::connector::PROCESS_CONNECTOR_ID).unwrap(),
            "unknown",
        ),
        None
    );
    assert_eq!(
        rastok::boundary::process_attribute_site(
            &rastok::connector::ContractId::parse("test/other@1").unwrap(),
            "command",
        ),
        None
    );
}

#[test]
fn constant_process_cwd_and_env_are_effective_plan_attributes() {
    let source = process_graph(&format!(
        "{FULL}\n    cwd = \"/\"\n    env = {{ \"RASTOK_PHASE_PROBE\": \"bound\" }}"
    ));
    let compilation = compile_one(&source).expect("constant operational settings compile");
    let shell = compilation
        .plan
        .nodes
        .iter()
        .find(|node| node.path.to_string() == "shell")
        .expect("the Process node");
    let rastok::plan::NodeExecution::Connector(spec) = &shell.execution else {
        panic!("the Process node remains a connector")
    };
    assert_eq!(
        spec.attribute("cwd"),
        Some(&rastok::plan::AttrSetting::Value(
            rastok::value::Value::Text(String::from("/"))
        ))
    );
    assert!(matches!(
        spec.attribute("env"),
        Some(rastok::plan::AttrSetting::Value(rastok::value::Value::Map(entries)))
            if entries.get("RASTOK_PHASE_PROBE")
                == Some(&rastok::value::Value::Text(String::from("bound")))
    ));

    let invalid = process_graph(&format!("{FULL}\n    env = {{ \"COUNT\": 2 }}"));
    refusal_with(&invalid, "E-CONTRACT-ATTR", "TextMap");
}

#[test]
fn the_service_profile_still_owes_the_common_sheet() {
    let source = process_graph("    mode = service");
    let bag = compile_one(&source).expect_err("must refuse");
    for name in ["`command`", "`codec`", "`behavior`"] {
        assert!(
            bag.diagnostics()
                .iter()
                .any(|d| d.code().as_str() == "E-CONTRACT-REQUIRED" && d.message().contains(name)),
            "missing required refusal for {name}: {bag:?}"
        );
    }
}

#[test]
fn sibling_profiles_may_share_an_attribute_name() {
    // The selected profile disambiguates — only common-vs-profile
    // and within-scope duplicates are defects.
    const SHARED: &AttributeSpec = &AttributeSpec {
        name: "shared",
        presence: Presence::Optional,
        domain: ValueDomain::Text,
        latest_site: EvaluationSite::Plan,
    };
    const SELECTOR: ProfileSelector = ProfileSelector {
        attribute: "mode",
        default: Some("a"),
        profiles: &[
            Profile {
                protocol: None,
                word: "a",
                support: ProfileSupport::Available,
                attributes: std::slice::from_ref(SHARED),
            },
            Profile {
                protocol: None,
                word: "b",
                support: ProfileSupport::Available,
                attributes: std::slice::from_ref(SHARED),
            },
        ],
    };
    assert!(
        ContractCatalog::try_m0()
            .expect("the standard catalogue holds")
            .try_with(sheet(&[], Some(SELECTOR)))
            .is_ok()
    );
}

#[test]
fn an_inexpressible_sheet_name_is_a_sheet_defect() {
    let bad_port = ConnectorContract {
        id: "test/probe@1",
        name: "Probe",
        roles: rastok::connector::ConnectorRoles::Outbound,
        exchange: rastok::connector::ConnectorExchange::Call,
        ports: &[PortContract {
            name: "bad port",
            direction: Direction::Input,

            ty: PortTypeRule::Fixed(PortDataType::Variant),
        }],
        common: &[],
        selector: None,
    };
    assert_eq!(
        ContractCatalog::try_m0()
            .expect("the standard catalogue holds")
            .try_with(bad_port)
            .unwrap_err(),
        SheetDefect::InvalidPortName("bad port")
    );
    const BAD_ATTR: &[AttributeSpec] = &[AttributeSpec {
        name: "not a name",
        presence: Presence::Optional,
        domain: ValueDomain::Text,
        latest_site: EvaluationSite::Plan,
    }];
    assert_eq!(
        ContractCatalog::try_m0()
            .expect("the standard catalogue holds")
            .try_with(sheet(BAD_ATTR, None))
            .unwrap_err(),
        SheetDefect::InexpressibleName("not a name")
    );
}

#[test]
fn a_deferred_domain_carries_no_default() {
    const LIE: ProfileSelector = ProfileSelector {
        attribute: "mode",
        default: Some("a"),
        profiles: &[Profile {
            protocol: None,
            word: "a",
            support: ProfileSupport::Deferred("someday"),
            attributes: &[AttributeSpec {
                name: "x",
                presence: Presence::Defaulted(DefaultValue::Expression("[]")),
                domain: ValueDomain::Struct,
                latest_site: EvaluationSite::Instance,
            }],
        }],
    };
    assert!(matches!(
        ContractCatalog::try_m0()
            .expect("the standard catalogue holds")
            .try_with(sheet(&[], Some(LIE)))
            .unwrap_err(),
        SheetDefect::InvalidDefault { attribute: "x", .. }
    ));
}

#[test]
fn a_nested_block_entry_names_its_migration() {
    let source = "graph g : App {\n  operator o {\n    on in { out = { parent { child = 1 } } }\n  }\n  in -> o -> out\n}\n";
    refusal_with(
        source,
        "E-CONTRACT-ATTR",
        "`parent { … }` is no longer a form; write `parent = { … }`",
    );
}

#[test]
fn an_obsolete_initializer_has_one_migration_voice_in_every_value_position() {
    // B6 makes `IDENT { ... }` one real syntax production. When the name is
    // an obsolete initializer rather than a genuine union case, CHECK owns
    // the one migration voice in every value position.
    for (label, source) in [
        (
            "var initializer",
            "graph g : App {\n  operator o {\n    var v : int = measure { x = 1 }\n    on in { out = data }\n  }\n  in -> o -> out\n}\n",
        ),
        (
            "local val",
            "graph g : App {\n  operator o {\n    on in {\n      val x = on_crash { y = 1 }\n      out = data\n    }\n  }\n  in -> o -> out\n}\n",
        ),
        (
            "state write",
            "graph g : App {\n  operator o {\n    var v : int = 0\n    on in {\n      v = exponential { y = 1 }\n      out = data\n    }\n  }\n  in -> o -> out\n}\n",
        ),
    ] {
        let bag = compile_one(source).expect_err(label);
        assert_eq!(bag.diagnostics().len(), 1, "{label}: {bag:?}");
        assert_eq!(
            bag.diagnostics()[0].code().as_str(),
            "E-CONTRACT-ATTR",
            "{label}: the migration voice alone"
        );
    }
}

#[test]
fn structural_words_are_reserved_but_frontend_names_are_bindable() {
    // B4: `json` is an ordinary name; only the adjacent `!` selects
    // its frontend. Structural/literal words remain unmaskable.
    for (label, source) in [
        (
            "operator val nothing",
            "graph g : App {\n  operator o {\n    val nothing = 42\n    on in { out = data }\n  }\n  in -> o -> out\n}\n",
        ),
        (
            "operator var nothing",
            "graph g : App {\n  operator o {\n    var nothing : int = 0\n    on in { out = data }\n  }\n  in -> o -> out\n}\n",
        ),
        (
            "node named nothing",
            "graph g : App {\n  operator nothing {\n    on in { out = data }\n  }\n  in -> nothing -> out\n}\n",
        ),
        (
            "graph named nothing",
            "graph nothing : App {\n  operator o {\n    on in { out = data }\n  }\n  in -> o -> out\n}\n",
        ),
        (
            "boundary input nothing",
            "graph g : App {\n  input nothing\n  operator o {\n    on in { out = data }\n  }\n  in -> o -> out\n}\n",
        ),
        // The expression literals: a masked one is a shadow Rastok expression can
        // never read (review round 3).
        (
            "val true",
            "graph g : App {\n  operator o {\n    val true = 1\n    on in { out = data }\n  }\n  in -> o -> out\n}\n",
        ),
        (
            "var false",
            "graph g : App {\n  operator o {\n    var false : int = 0\n    on in { out = data }\n  }\n  in -> o -> out\n}\n",
        ),
        (
            "val null",
            "graph g : App {\n  operator o {\n    val null = 1\n    on in { out = data }\n  }\n  in -> o -> out\n}\n",
        ),
        (
            "val now",
            "graph g : App {\n  operator o {\n    val now = 1\n    on in { out = data }\n  }\n  in -> o -> out\n}\n",
        ),
        // A user type may not SPELL a builtin: the builtin resolves
        // first and the declaration is silently dead (proved by the
        // identical hash before the guard).
        (
            "type spelling a builtin",
            "type text = int\ngraph g : App {\n  operator o {\n    on in { out = data }\n  }\n  in -> o -> out\n}\n",
        ),
    ] {
        let bag = compile_one(source).expect_err(label);
        assert!(
            bag.diagnostics().iter().any(|d| {
                d.code().as_str() == "E-NAME-RESERVED"
                    && (d.message().contains("reserved word")
                        || d.message().contains("builtin type"))
            }),
            "{label}: {bag:?}"
        );
    }
    compile_one(
        "graph g : App {\n  input in: int\n  output out: int\n  operator o {\n    val json = 1\n    on in { out = json }\n  }\n  in -> o -> out\n}\n",
    )
    .expect("a frontend name remains an ordinary binding");
    compile_one(
        "graph g : App {\n  input in: variant\n  output out: variant\n  operator json {\n    on in { out = data }\n  }\n  in -> json -> out\n}\n",
    )
    .expect("a frontend name remains an ordinary node name");
    // The value forms themselves stay whole.
    compile_one("graph g : App {\n  input in: variant\n  output out: variant\n  operator o {\n    on in { out = json! {\"a\": 1} }\n  }\n  in -> o -> out\n}\n")
        .expect("json object lives");
    compile_one(
        "graph g : App {\n  input in: variant\n  output out: variant\n  operator o {\n    on in { out = json! [1, 2] }\n  }\n  in -> o -> out\n}\n",
    )
    .expect("json array lives");
    compile_one(
        "graph g : App {\n  input in: variant\n  output out: variant\n  operator o {\n    on in { out = nothing }\n  }\n  in -> o -> out\n}\n",
    )
    .expect("nothing lives");
    // With no maskable binding, `json![0]` IS the document `[0]` —
    // the same program as its spaced twin, and never the Rastok expression list.
    let hash = |source: &str| compile_one(source).expect("valid").hash.to_string();
    assert_eq!(
        hash(
            "graph g : App {\n  input in: variant\n  output out: variant\n  operator o {\n    on in { out = json![0] }\n  }\n  in -> o -> out\n}\n"
        ),
        hash(
            "graph g : App {\n  input in: variant\n  output out: variant\n  operator o {\n    on in { out = json! [0] }\n  }\n  in -> o -> out\n}\n"
        ),
        "spacing is not identity"
    );
    assert_ne!(
        hash(
            "graph g : App {\n  input in: variant\n  output out: variant\n  operator o {\n    on in { out = json![0] }\n  }\n  in -> o -> out\n}\n"
        ),
        hash(
            "graph g : App {\n  input in: variant\n  output out: variant\n  operator o {\n    on in { out = [0] }\n  }\n  in -> o -> out\n}\n"
        ),
        "a variant document is not the Rastok expression list"
    );
}

#[test]
fn a_registered_frontend_owns_framing_without_becoming_a_keyword() {
    assert_eq!(
        ContractCatalog::default().try_with_frontend(FrontendContract {
            name: "bad name",
            reader: FrontendReader::Text,
        }),
        Err(SheetDefect::InexpressibleName("bad name")),
        "an admitted frontend contract must be writable before `!`"
    );
    let registry = ContractCatalog::try_m0()
        .expect("the standard catalogue holds")
        .try_with_frontend(FrontendContract {
            name: "starlark",
            reader: FrontendReader::Deferred {
                framing: FrontendFraming::QuotedBraced,
                construct: "the application Starlark frontend",
            },
        })
        .expect("the registered name is expressible");
    let source = "graph g : App {\n  input in: variant\n  output out: variant\n  operator o {\n    on in { out = starlark! { x = '@'; nested { y = \"}\" } } }\n  }\n  in -> o -> out\n}\n";
    let bag = compile(&[source], &registry).expect_err("the compiler is deliberately deferred");
    assert_eq!(bag.diagnostics().len(), 1, "{bag:?}");
    assert!(
        bag.diagnostics()[0]
            .message()
            .contains("the application Starlark frontend"),
        "{bag:?}"
    );

    let unknown = compile_one(
        "graph g : App {\n  input in: variant\n  output out: variant\n  operator o {\n    on in { out = starlark! { x } }\n  }\n  in -> o -> out\n}\n",
    )
    .expect_err("an unregistered frontend refuses");
    assert_eq!(unknown.diagnostics().len(), 1, "{unknown:?}");
    assert_eq!(
        unknown.diagnostics()[0].code().as_str(),
        "E-RESOLVE-UNKNOWN"
    );
}

#[test]
fn a_frontend_fills_one_host_value_and_never_an_expression_atom() {
    let embedded = compile_one(
        "graph g : App {\n  input in: int\n  output out: int\n  operator o {\n    on in { out = 1 + json! 2 }\n  }\n  in -> o -> out\n}\n",
    )
    .expect_err("a frontend result must be bound before expression use");
    assert_eq!(embedded.diagnostics().len(), 1, "{embedded:?}");
    assert_eq!(embedded.diagnostics()[0].code().as_str(), "E-EXPR");

    let spaced = compile_one(
        "graph g : App {\n  input in: int\n  output out: int\n  operator o {\n    val json = 2\n    on in { out = json ! 1 }\n  }\n  in -> o -> out\n}\n",
    )
    .expect_err("trivia between the name and bang is not frontend syntax");
    assert_eq!(spaced.diagnostics().len(), 1, "{spaced:?}");
    assert_eq!(spaced.diagnostics()[0].code().as_str(), "E-EXPR");

    compile_one(
        "graph g : App {\n  input in: int\n  output out: bool\n  operator o {\n    val json = 2\n    on in { out = json!=data }\n  }\n  in -> o -> out\n}\n",
    )
    .expect("maximal munch keeps `!=` an expression operator");
}

#[test]
fn a_json_recovery_follows_the_position_terminators() {
    // The review's blocker: `;`-separated statements, struct
    // fields, paren tuples, and MULTILINE faulty json all recover
    // at their own terminator — one voice, siblings preserved.
    for (label, source) in [
        (
            "semicolon statement",
            "graph g : App {\n  operator o {\n    on in { out = json! {\"a\": ???}; out = data }\n  }\n  in -> o -> out\n}\n",
        ),
        (
            "struct field",
            "graph g : App {\n  operator o {\n    on in { out = { bad = json! {\"a\": ???}, ok = 1 } }\n  }\n  in -> o -> out\n}\n",
        ),
        (
            "multiline json",
            "graph g : App {\n  operator o {\n    on in {\n      out = json! {\n        \"a\": ???\n      }\n      out = data\n    }\n  }\n  in -> o -> out\n}\n",
        ),
        // Any non-JSON follower after the explicit frontend sigil is
        // one reader-owned refusal with forward host recovery.
        (
            "host-alien first byte",
            "graph g : App {\n  operator o {\n    on in { out = json! @; out = data }\n  }\n  in -> o -> out\n}\n",
        ),
        (
            "quoted Rastok expression follower",
            "graph g : App {\n  operator o {\n    on in { out = json! 'x'; out = data }\n  }\n  in -> o -> out\n}\n",
        ),
    ] {
        let bag = compile_one(source).expect_err(label);
        assert_eq!(bag.diagnostics().len(), 1, "{label}: {bag:?}");
        assert_eq!(
            bag.diagnostics()[0].code().as_str(),
            "E-JSON-VALUE",
            "{label}"
        );
    }
    // The positive twin proves the SIBLING really lives there: with
    // the fault healed, the handler carries BOTH emissions.
    let healed = "graph g : App {\n  operator o {\n    on in { out = json! {\"a\": 1}; out = data }\n  }\n  in -> o -> out\n}\n";
    let compilation = compile_one(healed).expect("valid");
    let handler = &compilation.plan.plan().handlers[0];
    let emissions = handler
        .dispatch
        .steps()
        .filter(|step| matches!(step, rastok::plan::DispatchStep::Emit { .. }))
        .count();
    assert_eq!(emissions, 2, "both emissions live in the tree");

    for scalar in ["19.99", "\"s\"", "true", "false", "null"] {
        compile_one(&format!(
            "graph g : App {{\n  input in: variant\n  output out: variant\n  operator o {{\n    on in {{ out = json! {scalar} }}\n  }}\n  in -> o -> out\n}}\n"
        ))
        .unwrap_or_else(|bag| panic!("scalar JSON `{scalar}` lives: {bag:?}"));
    }
}

#[test]
fn a_semicolon_at_json_depth_is_a_strong_boundary() {
    // `;` cannot appear in JSON outside a string: the walk stops at
    // ANY depth, the sibling survives, and the two residual closer
    // errors are the file's REAL brace imbalance (the unterminated
    // json `{` — accepted as necessarily ambiguous).
    let source = "graph g : App {\n  operator o {\n    on in { out = json! { \"a\": ???; out = data } }\n  }\n  in -> o -> out\n}\n";
    let bag = compile_one(source).expect_err("must refuse");
    let errors: Vec<_> = bag
        .diagnostics()
        .iter()
        .filter(|d| d.code().as_str().starts_with("E-"))
        .collect();
    assert_eq!(errors.len(), 3, "{bag:?}");
    assert_eq!(errors[0].code().as_str(), "E-JSON-VALUE");
    assert!(
        errors[1..]
            .iter()
            .all(|d| d.code().as_str() == "E-PARSE-EXPECTED"),
        "the tail is the real imbalance: {bag:?}"
    );
}

#[test]
fn json_trivia_belongs_to_the_host() {
    // A same-line comment between the sigil and the payload is HOST
    // trivia — the reader never sees it, and the identity is the
    // same as the uncommented spelling.
    let commented = "graph g : App {\n  input in: variant\n  output out: variant\n  operator o {\n    on in { out = json! /* c */ {\"a\": 1} }\n  }\n  in -> o -> out\n}\n";
    let plain = "graph g : App {\n  input in: variant\n  output out: variant\n  operator o {\n    on in { out = json! {\"a\": 1} }\n  }\n  in -> o -> out\n}\n";
    let a = compile_one(commented).expect("valid");
    let b = compile_one(plain).expect("valid");
    assert_eq!(
        a.hash.to_string(),
        b.hash.to_string(),
        "host trivia is not identity"
    );
    // An NBSP is NOT host trivia: it opens no document and is not
    // the position's boundary, so the json form is invalid — the
    // boundary never depends on Unicode trim_start
    // (D-2026-08-13-1, review round 3).
    let nbsp = "graph g : App {\n  operator o {\n    on in { out = json!\u{a0}1 }\n  }\n  in -> o -> out\n}\n";
    let bag = compile_one(nbsp).expect_err("must refuse");
    assert_eq!(bag.diagnostics().len(), 1, "{bag:?}");
    assert_eq!(bag.diagnostics()[0].code().as_str(), "E-JSON-VALUE");
    // Host trivia may cross lines after the explicit sigil; it is
    // still not part of the JSON value or its identity.
    let crossing = "graph g : App {\n  input in: variant\n  output out: variant\n  operator o {\n    on in { out = json! /* c\n*/ 1 }\n  }\n  in -> o -> out\n}\n";
    let scalar = "graph g : App {\n  input in: variant\n  output out: variant\n  operator o {\n    on in { out = json! 1 }\n  }\n  in -> o -> out\n}\n";
    assert_eq!(
        compile_one(crossing)
            .expect("cross-line trivia is valid")
            .hash,
        compile_one(scalar).expect("scalar JSON is valid").hash,
    );

    let unterminated =
        "graph g : App {\n  operator o {\n    on in { out = json! /* never\n  }\n}\n";
    let bag = compile_one(unterminated).expect_err("the host comment is unterminated");
    assert_eq!(
        bag.diagnostics()[0].code().as_str(),
        "E-LEX-UNTERMINATED-COMMENT",
        "the host trivia authority speaks first: {bag:?}"
    );
    assert!(
        bag.diagnostics()
            .iter()
            .all(|diagnostic| diagnostic.code().as_str() != "E-JSON-VALUE"),
        "the frontend reader must not diagnose the same missing payload: {bag:?}"
    );
}

#[test]
fn frontend_leading_trivia_has_one_lexical_authority() {
    for frontend in ["text", "jq"] {
        let source = format!(
            "graph g : App {{\n  operator o {{\n    on in {{ out = {frontend}! /* never\n  }}\n}}\n"
        );
        let bag = compile_one(&source).expect_err("the host comment is unterminated");
        assert_eq!(
            bag.diagnostics()[0].code().as_str(),
            "E-LEX-UNTERMINATED-COMMENT",
            "{frontend}: the trivia authority speaks first: {bag:?}"
        );
        assert!(
            bag.diagnostics()
                .iter()
                .all(|diagnostic| !diagnostic.message().contains("a braced frontend payload")),
            "{frontend}: the parser must not rediagnose the missing opener: {bag:?}"
        );
    }
}

#[test]
fn dead_and_reserved_parens_are_opaque_too() {
    // Including DELIMITERS inside both quote styles: a quoted paren
    // is content, never structure (the review's probe).
    for (label, source) in [
        (
            "dead initializer with alien parens",
            "graph g : App {\n  operator o {\n    on in { out = on_crash('@') { weird = '@' } }\n  }\n  in -> o -> out\n}\n",
        ),
        (
            "reserved format with alien parens",
            "graph g : App {\n  operator o {\n    on in { out = protobuf!('@') { message X {} } }\n  }\n  in -> o -> out\n}\n",
        ),
        (
            "single-quoted open paren",
            "graph g : App {\n  operator o {\n    on in { out = on_crash('(') { weird = '@' }; out = data }\n  }\n  in -> o -> out\n}\n",
        ),
        (
            "single-quoted close paren",
            "graph g : App {\n  operator o {\n    on in { out = protobuf!(')') { message X {} }; out = data }\n  }\n  in -> o -> out\n}\n",
        ),
        (
            "double-quoted open paren",
            "graph g : App {\n  operator o {\n    on in { out = on_crash(\"(\") { weird = '@' }; out = data }\n  }\n  in -> o -> out\n}\n",
        ),
        (
            "double-quoted close paren",
            "graph g : App {\n  operator o {\n    on in { out = protobuf!(\")\") { message X {} }; out = data }\n  }\n  in -> o -> out\n}\n",
        ),
    ] {
        let bag = compile_one(source).expect_err(label);
        assert_eq!(bag.diagnostics().len(), 1, "{label}: {bag:?}");
    }
}

#[test]
fn an_invalid_selector_selects_no_implicit_profile() {
    // `mode =` is PresentInvalid — the compiler never pretends
    // `mode = job` about a mode the author wrote and got wrong: one
    // parser voice, no belongs-to-job claim.
    let source = "graph g : App {\n  connector shell : Process {\n    mode =\n    restart = whatever\n    command = \"printf\"\n    codec = lines\n    idempotent = true\n    behavior = deterministic\n  }\n  in -> shell -> out\n}\n";
    let bag = compile_one(source).expect_err("must refuse");
    assert_eq!(bag.diagnostics().len(), 1, "{bag:?}");
    assert_eq!(bag.diagnostics()[0].code().as_str(), "E-PARSE-EXPECTED");
}

#[test]
fn a_duplicate_attribute_is_syntactic() {
    // `command =` then `command = "ok"`: the first value failed, the
    // NAME was still written — the duplicate refuses.
    let source = "graph g : App {\n  connector shell : Process {\n    command =\n    command = \"printf\"\n    codec = lines\n    idempotent = true; behavior = deterministic\n  }\n  in -> shell -> out\n}\n";
    let bag = compile_one(source).expect_err("must refuse");
    assert!(
        bag.diagnostics()
            .iter()
            .any(|d| d.message().contains("declared twice")),
        "{bag:?}"
    );
}

#[test]
fn an_omitted_value_never_absorbs_the_next_line() {
    // The review's blocker: `command =` must diagnose EXACTLY ONCE,
    // at the empty position, and the NEXT attribute must survive —
    // written-but-invalid is neither "missing" nor defaultable, so
    // no Required cascade and no MissingCodec artifact.
    let source = "graph g : App {\n  connector shell : Process {\n    command =\n    codec = lines\n    idempotent = true\n    behavior = deterministic\n  }\n  in -> shell -> out\n}\n";
    let bag = compile_one(source).expect_err("must refuse");
    assert_eq!(bag.diagnostics().len(), 1, "{bag:?}");
    let diagnostic = &bag.diagnostics()[0];
    assert_eq!(diagnostic.code().as_str(), "E-PARSE-EXPECTED");
    assert!(
        diagnostic
            .message()
            .contains("expected a value before the end of the line"),
        "{diagnostic:?}"
    );
    // The span sits on the `command =` line (line 3 starts at 39).
    let span = diagnostic.spans()[0].range();
    let line = source[..span.start].matches('\n').count() + 1;
    assert_eq!(line, 3, "the diagnostic points at the EMPTY position");
    // In a handler: `val x =` keeps the following emission.
    let source = "graph g : App {\n  operator o {\n    on in {\n      val x =\n      out = data\n    }\n  }\n  in -> o -> out\n}\n";
    let bag = compile_one(source).expect_err("must refuse");
    assert_eq!(bag.diagnostics().len(), 1, "{bag:?}");
    assert_eq!(bag.diagnostics()[0].code().as_str(), "E-PARSE-EXPECTED");
}

#[test]
fn a_json_error_resynchronizes_at_the_line_boundary() {
    // The faulty value is consumed to its line's end — one voice,
    // and the next emission survives in the tree.
    let source = "graph g : App {\n  operator o {\n    on in {\n      out = json! {\"a\": ???}\n      out = data\n    }\n  }\n  in -> o -> out\n}\n";
    let bag = compile_one(source).expect_err("must refuse");
    assert_eq!(bag.diagnostics().len(), 1, "{bag:?}");
    assert!(
        bag.diagnostics()[0]
            .message()
            .contains("the JSON value does not read"),
        "{bag:?}"
    );
}

#[test]
fn an_omitted_expression_always_diagnoses() {
    // The review's blocker: the recovery form is Value::Error, never
    // an empty ExprSource a user could also produce — a genuinely
    // omitted expression must diagnose, everywhere.
    for (label, source) in [
        (
            "out =",
            "graph g : App {\n  operator o {\n    on in {\n      out =\n    }\n  }\n  in -> o -> out\n}\n",
        ),
        (
            "val x =",
            "graph g : App {\n  val x =\n  operator o {\n    on in { out = data }\n  }\n  in -> o -> out\n}\n",
        ),
        (
            "attribute without a value",
            "graph g : App {\n  connector shell : Process {\n    command =\n    codec = lines\n    idempotent = true; behavior = deterministic\n  }\n  in -> shell -> out\n}\n",
        ),
        (
            "guard without an expression",
            "graph g : App {\n  operator o {\n    on in where {\n      out = data\n    }\n  }\n  in -> o -> out\n}\n",
        ),
    ] {
        let bag = compile_one(source).expect_err(label);
        assert!(
            !bag.diagnostics().is_empty(),
            "`{label}` must speak: {bag:?}"
        );
    }
}

#[test]
fn a_reserved_format_body_defers_once_and_opaquely() {
    // `avro! { … }` selects its registered, currently deferred
    // frontend: one named deferral and an opaque body.
    let source = "graph g : App {\n  operator o {\n    on in { out = avro! { \"type\": \"record\", \"name\": \"X\" } }\n  }\n  in -> o -> out\n}\n";
    let bag = compile_one(source).expect_err("reserved defers");
    assert_eq!(bag.diagnostics().len(), 1, "{bag:?}");
    assert!(
        bag.diagnostics()[0]
            .message()
            .contains("the Avro data frontend"),
        "{bag:?}"
    );
}

#[test]
fn a_reserved_format_body_is_lexically_opaque() {
    // The body never meets the host lexer: characters RKG cannot
    // tokenize, braces inside quotes, and nesting all pass under
    // the ONE deferral; an unterminated block is the raw lexer's
    // own fault.
    for (label, body) in [
        ("host-unknown characters", "option strange = '@';"),
        ("braces inside strings", r#"x = "{{{" ; y = '}'"#),
        ("nested blocks", "outer { inner { '@' } }"),
    ] {
        let source = format!(
            "graph g : App {{\n  operator o {{\n    on in {{ out = protobuf! {{ {body} }} }}\n  }}\n  in -> o -> out\n}}\n"
        );
        let bag = compile_one(&source).expect_err(label);
        assert_eq!(bag.diagnostics().len(), 1, "{label}: {bag:?}");
        assert!(
            bag.diagnostics()[0]
                .message()
                .contains("the Protobuf data frontend"),
            "{label}: {bag:?}"
        );
    }
    // Unterminated: framing fails before a complete frontend value
    // exists, so the framer's lexical cause speaks first.
    let source = "graph g : App {\n  operator o {\n    on in { out = avro! { never closed\n";
    let bag = compile_one(source).expect_err("unterminated refuses");
    assert_eq!(
        bag.diagnostics()[0].code().as_str(),
        "E-LEX-UNTERMINATED-BLOCK",
        "{bag:?}"
    );
}

#[test]
fn gauge_without_a_connector_is_immobile() {
    // The witness that the identity move is SCOPED: no connector,
    // no injected attribute, no movement.
    let source = std::fs::read_to_string(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/gauge.rkg"),
    )
    .expect("the example reads");
    let compilation = compile_one(&source).expect("valid");
    assert_eq!(
        compilation.hash.to_string(),
        "0d3ad43d9fb09c8626146342f6a50d1f4c387a8de52a47a3134a408cf74cb22e",
        "gauge moved — the injection leaked outside connectors"
    );
}
