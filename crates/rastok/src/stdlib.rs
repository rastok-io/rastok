//! The hosted standard catalogue, frontends and annotation declarations.
//!
//! Governing rule: [`stdlib.md`] — the ten-case Connector template and
//! the standard contracts. The compiler and independent package validator
//! read the same definitions in [`crate::connector::stdlib`], without linking
//! any physical Driver. This module adds the compiler's frontend and
//! annotation declarations and explicitly deferred Connector names.
//!
//! [`stdlib.md`]: https://github.com/rastok-io/rastok/blob/main/docs/spec/stdlib.md

use crate::contract::{ContractCatalog, FrontendContract, FrontendFraming, FrontendReader};

/// Hosted declarations; runtime interpretation is retained in the resolved plan.
pub(crate) const ANNOTATION_DEFINITIONS: &str = include_str!("stdlib/annotations.rkg");

/// One completion candidate for the open `media_type` logical scalar.
///
/// This is deliberately a catalogue, not an enum: registered and vendor
/// media types remain valid when they parse canonically even when they are
/// absent here. Tooling may offer these stable names without turning today's
/// list into tomorrow's compatibility constraint.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MediaTypeCatalogEntry {
    /// Completion-friendly Rastok spelling.
    pub name: &'static str,
    /// Canonical concrete media type.
    pub value: &'static str,
}

/// Common Internet media types offered by standard tooling.
pub const MEDIA_TYPE_CATALOG: &[MediaTypeCatalogEntry] = &[
    MediaTypeCatalogEntry {
        name: "application_cbor",
        value: "application/cbor",
    },
    MediaTypeCatalogEntry {
        name: "application_form_urlencoded",
        value: "application/x-www-form-urlencoded",
    },
    MediaTypeCatalogEntry {
        name: "application_json",
        value: "application/json",
    },
    MediaTypeCatalogEntry {
        name: "application_ndjson",
        value: "application/x-ndjson",
    },
    MediaTypeCatalogEntry {
        name: "application_octet_stream",
        value: "application/octet-stream",
    },
    MediaTypeCatalogEntry {
        name: "application_pdf",
        value: "application/pdf",
    },
    MediaTypeCatalogEntry {
        name: "application_problem_json",
        value: "application/problem+json",
    },
    MediaTypeCatalogEntry {
        name: "application_protobuf",
        value: "application/protobuf",
    },
    MediaTypeCatalogEntry {
        name: "application_xml",
        value: "application/xml",
    },
    MediaTypeCatalogEntry {
        name: "image_jpeg",
        value: "image/jpeg",
    },
    MediaTypeCatalogEntry {
        name: "image_png",
        value: "image/png",
    },
    MediaTypeCatalogEntry {
        name: "multipart_form_data",
        value: "multipart/form-data",
    },
    MediaTypeCatalogEntry {
        name: "text_csv",
        value: "text/csv",
    },
    MediaTypeCatalogEntry {
        name: "text_html",
        value: "text/html",
    },
    MediaTypeCatalogEntry {
        name: "text_plain_utf8",
        value: "text/plain; charset=utf-8",
    },
];

use crate::connector::stdlib::{
    FILE, HTTP_CLIENT, HTTP_SERVER, PROCESS, WEBSOCKET_CLIENT, WEBSOCKET_SERVER,
};

const DECLARED: &[&str] = &["Timer", "Calendar", "Memory"];

const M0_FRONTENDS: &[FrontendContract] = &[
    FrontendContract {
        name: "json",
        reader: FrontendReader::Json,
    },
    FrontendContract {
        name: "text",
        reader: FrontendReader::Text,
    },
    FrontendContract {
        name: "base64",
        reader: FrontendReader::Base64,
    },
    FrontendContract {
        name: "jq",
        reader: FrontendReader::Deferred {
            framing: FrontendFraming::QuotedBraced,
            construct: "the jq guest frontend",
        },
    },
    FrontendContract {
        name: "sql",
        reader: FrontendReader::Deferred {
            framing: FrontendFraming::QuotedBraced,
            construct: "the SQL guest frontend",
        },
    },
    FrontendContract {
        name: "bash",
        reader: FrontendReader::Deferred {
            framing: FrontendFraming::QuotedBraced,
            construct: "the bash guest frontend",
        },
    },
    FrontendContract {
        name: "python",
        reader: FrontendReader::Deferred {
            framing: FrontendFraming::QuotedBraced,
            construct: "the Python guest frontend",
        },
    },
    FrontendContract {
        name: "rust",
        reader: FrontendReader::Deferred {
            framing: FrontendFraming::QuotedBraced,
            construct: "the Rust guest frontend",
        },
    },
    FrontendContract {
        name: "avro",
        reader: FrontendReader::Deferred {
            framing: FrontendFraming::QuotedBraced,
            construct: "the Avro data frontend",
        },
    },
    FrontendContract {
        name: "protobuf",
        reader: FrontendReader::Deferred {
            framing: FrontendFraming::QuotedBraced,
            construct: "the Protobuf data frontend",
        },
    },
    FrontendContract {
        name: "parquet",
        reader: FrontendReader::Deferred {
            framing: FrontendFraming::QuotedBraced,
            construct: "the Parquet data frontend",
        },
    },
    FrontendContract {
        name: "msgpack",
        reader: FrontendReader::Deferred {
            framing: FrontendFraming::QuotedBraced,
            construct: "the MessagePack data frontend",
        },
    },
    FrontendContract {
        name: "csv",
        reader: FrontendReader::Deferred {
            framing: FrontendFraming::QuotedBraced,
            construct: "the CSV data frontend",
        },
    },
];

impl ContractCatalog {
    /// The M0 registry through its fallible door — the same
    /// validation any extension walks.
    ///
    /// # Errors
    ///
    /// The first [`crate::contract::ContractValidationError`] of the standard
    /// catalogue.
    pub fn try_m0() -> Result<Self, crate::contract::ContractValidationError> {
        let base = Self {
            connectors: std::collections::BTreeMap::new(),
            declared: DECLARED.to_vec(),
            frontends: Vec::new(),
        };
        // The standard catalogue walks through the SAME door as any
        // extension — never an unvalidated authority.
        M0_FRONTENDS
            .iter()
            .cloned()
            .try_fold(base, Self::try_with_frontend)?
            .try_with(PROCESS)?
            .try_with(FILE)?
            .try_with(HTTP_CLIENT)?
            .try_with(HTTP_SERVER)?
            .try_with(WEBSOCKET_CLIENT)?
            .try_with(WEBSOCKET_SERVER)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connector::contract::{DefaultValue, EvaluationSite, Presence};
    use crate::connector::stdlib::HTTP_SERVER_CONNECTOR_ID;
    use crate::contract::Availability;

    #[test]
    fn the_catalogue_resolves_the_three_availabilities() {
        let catalogue = ContractCatalog::try_m0().expect("the standard catalogue holds");
        assert!(matches!(
            catalogue.connector("Process"),
            Availability::Available(contract) if contract.name == "Process"
        ));
        assert!(matches!(
            catalogue.connector("HttpServer"),
            Availability::Available(contract) if contract.id == HTTP_SERVER_CONNECTOR_ID
        ));
        assert_eq!(catalogue.connector("Timer"), Availability::Declared);
        assert_eq!(catalogue.connector("Postgres"), Availability::Unknown);
    }

    #[test]
    fn the_process_sheet_holds_its_own_door() {
        // The sheet validates at the registry door — defaults
        // compiled, profiles coherent.
        assert!(crate::contract::validate_sheet(&PROCESS).is_ok());
        let process = PROCESS;
        let job = process.profile("job").unwrap();
        let guarantee = process.attribute(Some(job), "idempotent").unwrap();
        assert_eq!(
            guarantee.presence,
            Presence::Defaulted(DefaultValue::Expression("false"))
        );
        assert!(process.attribute(Some(job), "effect").is_none());
        assert!(
            process
                .attribute(process.profile("service"), "idempotent")
                .is_none()
        );
        assert!(process.attribute(Some(job), "max_output").is_some());
        assert!(process.attribute(Some(job), "restart").is_none());
        assert!(process.owning_profile("restart").is_none());
        assert_eq!(
            process.owning_profile("max_frame").map(|p| p.word),
            Some("service")
        );
        let ports = PROCESS
            .resolve_ports(&std::collections::BTreeMap::from([
                (String::from("codec"), String::from("lines")),
                (String::from("err_codec"), String::from("lines")),
            ]))
            .expect("the effective Process sheet resolves");
        assert_eq!(ports.len(), 4);
        let process_exit =
            crate::connector::stdlib::process_exit_type().expect("the target type is valid");
        assert!(
            ports
                .iter()
                .any(|port| port.port.name() == "exit" && port.ty.as_ref() == Some(&process_exit))
        );
    }

    #[test]
    fn the_file_sheet_holds_its_own_door() {
        assert!(crate::contract::validate_sheet(&FILE).is_ok());
        let read = FILE.profile("read").expect("the read profile");
        assert!(FILE.attribute(Some(read), "max_read").is_some());
        assert_eq!(
            FILE.attribute(Some(read), "path")
                .expect("the common path")
                .latest_site,
            EvaluationSite::Invocation
        );
        let ports = FILE
            .resolve_ports(&std::collections::BTreeMap::from([(
                String::from("mode"),
                String::from("read"),
            )]))
            .expect("the effective File sheet resolves");
        assert_eq!(ports.len(), 3);
    }

    #[test]
    fn every_network_sheet_row_agrees_with_the_core_mirror() {
        let sheets = [
            &HTTP_CLIENT,
            &HTTP_SERVER,
            &WEBSOCKET_CLIENT,
            &WEBSOCKET_SERVER,
        ];
        for sheet in sheets {
            crate::contract::validate_sheet(sheet).expect("the network sheet holds its door");
            let id = crate::connector::ContractId::parse(sheet.id).expect("a canonical id");
            for attribute in sheet.common {
                assert_eq!(
                    crate::connector::stdlib::network_attribute_site(&id, attribute.name),
                    Some(attribute.latest_site),
                    "{}.{}",
                    sheet.name,
                    attribute.name
                );
            }
            if let Some(selector) = &sheet.selector {
                assert_eq!(
                    crate::connector::stdlib::network_attribute_site(&id, selector.attribute),
                    Some(EvaluationSite::Plan)
                );
                for profile in selector.profiles {
                    for attribute in profile.attributes {
                        assert_eq!(
                            crate::connector::stdlib::network_attribute_site(&id, attribute.name),
                            Some(attribute.latest_site),
                            "{}({}).{}",
                            sheet.name,
                            profile.word,
                            attribute.name
                        );
                    }
                }
            }

            for codec in ["bytes", "json", "text"] {
                let mut words = std::collections::BTreeMap::from([(
                    String::from("codec"),
                    String::from(codec),
                )]);
                if sheet.id == HTTP_SERVER_CONNECTOR_ID {
                    words.insert(String::from("mode"), String::from("ack"));
                }
                let ports = sheet
                    .resolve_ports(&words)
                    .expect("the effective network ports resolve");
                for port in ports {
                    assert_eq!(
                        port.ty,
                        crate::connector::stdlib::network_port_type(
                            &id,
                            port.port.name(),
                            Some(codec)
                        ),
                        "{}.{} under {codec}",
                        sheet.name,
                        port.port
                    );
                }
            }
        }
    }

    #[test]
    fn the_network_metadata_and_terminal_unions_are_closed() {
        assert_eq!(
            crate::plan::standard_ext_field_type("http_status"),
            Some(crate::datatype::DataType::INT)
        );
        assert_eq!(
            crate::plan::standard_ext_field_type("http_headers"),
            Some(crate::plan::http_headers_type())
        );
        let canonical_headers = crate::value::Value::Map(std::collections::BTreeMap::from([(
            String::from("set-cookie"),
            crate::value::Value::Array(vec![
                crate::value::Value::Bytes(b"a=1".to_vec()),
                crate::value::Value::Bytes(b"b=2".to_vec()),
            ]),
        )]));
        assert!(crate::plan::http_headers_value_holds(&canonical_headers));
        let uppercase = crate::value::Value::Map(std::collections::BTreeMap::from([(
            String::from("Content-Type"),
            crate::value::Value::Array(vec![crate::value::Value::Bytes(
                b"application/json".to_vec(),
            )]),
        )]));
        assert!(!crate::plan::http_headers_value_holds(&uppercase));
        let empty_values = crate::value::Value::Map(std::collections::BTreeMap::from([(
            String::from("accept"),
            crate::value::Value::Array(Vec::new()),
        )]));
        assert!(!crate::plan::http_headers_value_holds(&empty_values));
        let pseudo_field = crate::value::Value::Map(std::collections::BTreeMap::from([(
            String::from(":status"),
            crate::value::Value::Array(vec![crate::value::Value::Bytes(b"200".to_vec())]),
        )]));
        assert!(!crate::plan::http_headers_value_holds(&pseudo_field));
        let typed = crate::value::Value::Struct(vec![(
            String::from("content_type"),
            crate::value::Value::Text(String::from("application/problem+json")),
        )]);
        let custom = crate::value::Value::Map(std::collections::BTreeMap::from([(
            String::from("X-Trace-Id"),
            crate::value::Value::Array(vec![crate::value::Value::Text(String::from("one"))]),
        )]));
        assert_eq!(
            crate::plan::canonical_request_headers(&typed, &custom),
            Some(crate::value::Value::Map(std::collections::BTreeMap::from(
                [
                    (
                        String::from("content-type"),
                        crate::value::Value::Array(vec![crate::value::Value::Bytes(
                            b"application/problem+json".to_vec(),
                        )]),
                    ),
                    (
                        String::from("x-trace-id"),
                        crate::value::Value::Array(vec![crate::value::Value::Bytes(
                            b"one".to_vec(),
                        )]),
                    ),
                ]
            )))
        );
        let managed_custom = crate::value::Value::Map(std::collections::BTreeMap::from([(
            String::from("Content-Type"),
            crate::value::Value::Array(vec![crate::value::Value::Text(String::from(
                "application/json",
            ))]),
        )]));
        assert!(!crate::plan::custom_http_headers_value_holds(
            &managed_custom
        ));
        let client =
            crate::connector::stdlib::http_client_exit_type().expect("the client exit union");
        let server =
            crate::connector::stdlib::http_server_exit_type().expect("the server exit union");
        let websocket =
            crate::connector::stdlib::websocket_exit_type().expect("the session exit union");
        let case_names = |ty: crate::datatype::DataType| match ty {
            crate::datatype::DataType::TaggedUnion(union) => union
                .cases()
                .iter()
                .map(|case| case.name().to_string())
                .collect::<Vec<_>>(),
            other => panic!("expected a tagged union, got {other:?}"),
        };
        assert_eq!(
            case_names(client),
            [
                "BodyTooLarge",
                "Completed",
                "ConnectFailed",
                "DecodeFailed",
                "DnsFailed",
                "OutcomeUnknown",
                "ProtocolFailed",
                "ServerError",
                "TimeoutExpired",
                "TlsFailed",
            ]
        );
        assert_eq!(
            case_names(server),
            [
                "Completed",
                "OutcomeUnknown",
                "Overloaded",
                "PeerDisconnected",
                "RequestExpired",
            ]
        );
        assert_eq!(
            case_names(websocket),
            [
                "Closed",
                "ConnectFailed",
                "DecodeFailed",
                "DnsFailed",
                "HandshakeRejected",
                "MessageTooLarge",
                "OutcomeUnknown",
                "ProtocolFailed",
                "TimeoutExpired",
                "TlsFailed",
                "TransportLost",
            ]
        );
    }

    #[test]
    fn the_media_type_catalog_is_open_but_every_completion_is_canonical() {
        let mut names = std::collections::BTreeSet::new();
        let mut values = std::collections::BTreeSet::new();
        for entry in MEDIA_TYPE_CATALOG {
            assert!(names.insert(entry.name), "duplicate catalogue name");
            assert!(values.insert(entry.value), "duplicate catalogue value");
            assert_eq!(
                crate::datatype::canonical_media_type(entry.value),
                Ok(entry.value.to_string())
            );
        }
        assert_eq!(
            crate::datatype::canonical_media_type("application/vnd.example+json"),
            Ok(String::from("application/vnd.example+json")),
            "a valid value outside the completion catalogue remains admitted"
        );
    }

    #[test]
    fn frontends_share_one_contract_registry() {
        let registry = ContractCatalog::try_m0().expect("the standard catalogue holds");
        assert_eq!(
            registry.frontend("json").map(|frontend| frontend.reader),
            Some(FrontendReader::Json)
        );
        assert!(matches!(
            registry.frontend("jq").map(|frontend| frontend.reader),
            Some(FrontendReader::Deferred { .. })
        ));
        assert_eq!(
            registry
                .frontend("text")
                .map(|frontend| frontend.reader.result_type()),
            Some(Some(crate::datatype::DataType::Text))
        );
        assert_eq!(registry.frontend("starlark"), None);
    }
}
