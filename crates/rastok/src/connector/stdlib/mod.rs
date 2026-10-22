//! Hosted access to the portable standard Contracts and checked compiler catalogue.
//!
//! These are the same definitions consumed by independent core validation.
//! Physical Driver availability is reported separately by the Driver registry.

use super::contract::ContractCatalog;
use crate::stdlib::M0_FRONTENDS;
pub use rastok_core::connector::stdlib::*;

const DECLARED: &[&str] = &["Timer", "Calendar", "Memory"];

impl ContractCatalog {
    /// Builds the standard catalogue using the validation applied to every
    /// registered declaration.
    ///
    /// # Errors
    ///
    /// The first [`crate::connector::contract::ContractValidationError`] of the standard
    /// catalogue.
    pub fn try_m0() -> Result<Self, crate::connector::contract::ContractValidationError> {
        let base = Self {
            connectors: std::collections::BTreeMap::new(),
            declared: DECLARED.to_vec(),
            frontends: Vec::new(),
        };
        // Standard definitions must satisfy the same checks as additions.
        let base = M0_FRONTENDS
            .iter()
            .cloned()
            .try_fold(base, Self::try_with_frontend)?;
        CONTRACTS
            .iter()
            .copied()
            .try_fold(base, |catalogue, contract| {
                catalogue.try_with(contract.clone())
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connector::contract::Availability;
    use crate::connector::contract::{DefaultValue, EvaluationSite, Presence};
    use crate::connector::stdlib::HTTP_SERVER_CONNECTOR_ID;

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
        // compiled, modes coherent.
        assert!(crate::connector::contract::validate_sheet(&PROCESS).is_ok());
        let process = PROCESS;
        let job = process.mode("job").unwrap();
        let guarantee = process.attribute(Some(job), "idempotent").unwrap();
        assert_eq!(
            guarantee.presence,
            Presence::Defaulted(DefaultValue::Expression("false"))
        );
        assert!(process.attribute(Some(job), "effect").is_none());
        assert!(
            process
                .attribute(process.mode("service"), "idempotent")
                .is_none()
        );
        assert!(process.attribute(Some(job), "max_output").is_some());
        assert!(process.attribute(Some(job), "restart").is_none());
        assert!(process.owning_mode("restart").is_none());
        assert_eq!(
            process.owning_mode("max_frame").map(|p| p.word),
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
        assert!(crate::connector::contract::validate_sheet(&FILE).is_ok());
        let read = FILE.mode("read").expect("the read mode");
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
    fn network_ports_and_sites_come_from_the_shared_definitions() {
        let sheets = [
            &HTTP_CLIENT,
            &HTTP_SERVER,
            &WEBSOCKET_CLIENT,
            &WEBSOCKET_SERVER,
        ];
        for sheet in sheets {
            crate::connector::contract::validate_sheet(sheet)
                .expect("the network sheet holds its door");
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
                for mode in selector.modes {
                    for attribute in mode.attributes {
                        assert_eq!(
                            crate::connector::stdlib::network_attribute_site(&id, attribute.name),
                            Some(attribute.latest_site),
                            "{}({}).{}",
                            sheet.name,
                            mode.word,
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
}
