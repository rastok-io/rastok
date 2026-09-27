//! HttpServer contract binding and effectless Lab Driver.
//!
//! Decoded scenario requests use the same two-phase admission as the hosted
//! listener. The effectless model needs no listener or networking dependency.

use std::cell::RefCell;
#[cfg(feature = "hosted-http12")]
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::rc::Rc;

use crate::boundary::{
    Completion, HttpServerExit, InboundCallConnectorDriver, InboundCallEvent, InboundCallResponse,
    InboundCallResponseStart, InboundCallTimeout, InboundCallTimeoutStart, InboundConnectorDriver,
    InboundDecision, InboundExchangeRef, InboundProposal, InboundProposalHandle, InboundSubmission,
};
use crate::connector::{ConnectorRoles, ContractId};
use crate::plan::ValidatedExecutionPlan;
#[cfg(feature = "hosted-http12")]
use crate::plan::{AttrSetting, NodeExecution};
#[cfg(feature = "hosted-http12")]
use crate::value::Value;

#[cfg(feature = "hosted-http12")]
pub use crate::connectors::network::http_server::{HttpServerConfig, HttpServerFactory};

#[cfg(feature = "hosted-http12")]
use super::http_shared::HttpVersion;

/// Protocol behavior selected by the semantic HttpServer profile.
#[cfg(feature = "hosted-http12")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HttpServerMode {
    Ack,
    Respond,
}

/// One statically bindable route of the hosted server family.
#[cfg(feature = "hosted-http12")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct HttpServerRouteBinding {
    pub(crate) auth_bearer: Option<String>,
    pub(crate) codec: String,
    pub(crate) listen: String,
    pub(crate) max_body: u64,
    pub(crate) mode: HttpServerMode,
    pub(crate) node: crate::node::Path,
    pub(crate) path: String,
    pub(crate) response_headers: BTreeMap<String, Vec<Vec<u8>>>,
    pub(crate) tls_certificate: Option<Vec<u8>>,
    pub(crate) tls_private_key: Option<Vec<u8>>,
    pub(crate) versions: Vec<HttpVersion>,
}

#[cfg(feature = "hosted-http12")]
fn static_value(
    spec: &crate::plan::ConnectorSpec,
    name: &str,
) -> Result<Option<Value>, super::DriverFactoryError> {
    match spec.attribute(name) {
        Some(AttrSetting::Value(value)) => Ok(Some(value.clone())),
        Some(AttrSetting::Expr { .. }) => Err(super::DriverFactoryError::new(format!(
            "the hosted HttpServer setting `{name}` at `{}` must resolve before listener binding",
            spec.contract
        ))),
        Some(AttrSetting::Word(_)) => Err(super::DriverFactoryError::new(format!(
            "the hosted HttpServer setting `{name}` has the wrong value form"
        ))),
        None => Ok(None),
    }
}

#[cfg(feature = "hosted-http12")]
fn required_text(
    spec: &crate::plan::ConnectorSpec,
    name: &str,
) -> Result<String, super::DriverFactoryError> {
    match static_value(spec, name)? {
        Some(Value::Text(value)) => Ok(value),
        _ => Err(super::DriverFactoryError::new(format!(
            "the hosted HttpServer requires a resolved text `{name}` setting"
        ))),
    }
}

#[cfg(feature = "hosted-http12")]
fn optional_text(
    spec: &crate::plan::ConnectorSpec,
    name: &str,
) -> Result<Option<String>, super::DriverFactoryError> {
    match static_value(spec, name)? {
        Some(Value::Text(value)) => Ok(Some(value)),
        None => Ok(None),
        _ => Err(super::DriverFactoryError::new(format!(
            "the hosted HttpServer `{name}` setting is not text"
        ))),
    }
}

#[cfg(feature = "hosted-http12")]
fn optional_bytes(
    spec: &crate::plan::ConnectorSpec,
    name: &str,
) -> Result<Option<Vec<u8>>, super::DriverFactoryError> {
    match static_value(spec, name)? {
        Some(Value::Bytes(value)) => Ok(Some(value)),
        None => Ok(None),
        _ => Err(super::DriverFactoryError::new(format!(
            "the hosted HttpServer `{name}` setting is not bytes"
        ))),
    }
}

#[cfg(feature = "hosted-http12")]
fn response_headers(
    spec: &crate::plan::ConnectorSpec,
) -> Result<BTreeMap<String, Vec<Vec<u8>>>, super::DriverFactoryError> {
    let typed = static_value(spec, "headers")?.unwrap_or_else(|| Value::Struct(Vec::new()));
    let custom =
        static_value(spec, "custom_headers")?.unwrap_or_else(|| Value::Map(BTreeMap::new()));
    let Some(Value::Map(headers)) = crate::plan::canonical_request_headers(&typed, &custom) else {
        return Err(super::DriverFactoryError::new(
            "the hosted HttpServer response headers are not canonical",
        ));
    };
    headers
        .into_iter()
        .map(|(name, values)| {
            let Value::Array(values) = values else {
                return Err(super::DriverFactoryError::new(
                    "the hosted HttpServer response-header row is not an array",
                ));
            };
            let values = values
                .into_iter()
                .map(|value| match value {
                    Value::Bytes(value) => Ok(value),
                    _ => Err(super::DriverFactoryError::new(
                        "the hosted HttpServer response-header value is not bytes",
                    )),
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok((name, values))
        })
        .collect()
}

/// Resolves every static listener and route row before any socket opens.
#[cfg(feature = "hosted-http12")]
pub(crate) fn bind_routes(
    plan: &ValidatedExecutionPlan,
    nodes: &[crate::node::Path],
) -> Result<Vec<HttpServerRouteBinding>, super::DriverFactoryError> {
    let expected: BTreeSet<_> = nodes.iter().cloned().collect();
    let mut routes = Vec::new();
    for path in nodes {
        let spec = plan
            .plan()
            .nodes
            .iter()
            .find(|node| &node.path == path)
            .and_then(|node| match &node.execution {
                NodeExecution::Connector(spec) => Some(spec),
                NodeExecution::Operator => None,
            })
            .ok_or_else(|| {
                super::DriverFactoryError::new(format!(
                    "the HttpServer binding path `{path}` is not a connector"
                ))
            })?;
        if spec.contract.as_str() != crate::connector::HTTP_SERVER_CONNECTOR_ID {
            return Err(super::DriverFactoryError::new(format!(
                "the HttpServer binder received `{}` at `{path}`",
                spec.contract
            )));
        }
        let mode = match spec.attribute("mode") {
            Some(AttrSetting::Word(mode)) if mode == "ack" => HttpServerMode::Ack,
            Some(AttrSetting::Word(mode)) if mode == "respond" => HttpServerMode::Respond,
            _ => {
                return Err(super::DriverFactoryError::new(format!(
                    "the hosted HttpServer mode at `{path}` is unresolved"
                )));
            }
        };
        let codec = match spec.attribute("codec") {
            Some(AttrSetting::Word(codec))
                if matches!(codec.as_str(), "bytes" | "json" | "text") =>
            {
                codec.clone()
            }
            _ => {
                return Err(super::DriverFactoryError::new(format!(
                    "the hosted HttpServer codec at `{path}` is unresolved"
                )));
            }
        };
        let listen = required_text(spec, "listen")?;
        let route_path = required_text(spec, "path")?;
        if !route_path.starts_with('/') || route_path.contains(['?', '#']) {
            return Err(super::DriverFactoryError::new(format!(
                "the hosted HttpServer path `{route_path}` at `{path}` is not an absolute path"
            )));
        }
        let max_body = match static_value(spec, "max_body")? {
            Some(Value::Text(value)) => crate::connector::byte_size(&value),
            _ => None,
        }
        .ok_or_else(|| {
            super::DriverFactoryError::new(format!(
                "the hosted HttpServer body bound at `{path}` is unresolved"
            ))
        })?;
        let versions = match static_value(spec, "versions")? {
            Some(Value::Array(values)) => values
                .into_iter()
                .map(|value| match value {
                    Value::Text(value) | Value::Enum(value) => HttpVersion::parse(&value),
                    _ => None,
                })
                .collect::<Option<Vec<_>>>(),
            _ => None,
        }
        .filter(|versions| !versions.is_empty())
        .ok_or_else(|| {
            super::DriverFactoryError::new(format!(
                "the hosted HttpServer version preference at `{path}` is unresolved"
            ))
        })?;
        routes.push(HttpServerRouteBinding {
            auth_bearer: optional_text(spec, "auth_bearer")?,
            codec,
            listen,
            max_body,
            mode,
            node: path.clone(),
            path: route_path,
            response_headers: response_headers(spec)?,
            tls_certificate: optional_bytes(spec, "tls_certificate")?,
            tls_private_key: optional_bytes(spec, "tls_private_key")?,
            versions,
        });
    }
    let actual: BTreeSet<_> = routes.iter().map(|route| route.node.clone()).collect();
    if expected != actual {
        return Err(super::DriverFactoryError::new(
            "the HttpServer binder received an incomplete connector family",
        ));
    }
    Ok(routes)
}

/// One authored arrival and one post-admission acknowledgement, both bounded.
/// The scenario and the bound Lab Driver share this world-side slot; it never
/// owns envelopes, sequence allocation or journal records.
#[derive(Debug, Default)]
pub(crate) struct HttpServerLabInput {
    pending: Option<InboundProposal>,
    terminal: Option<InboundCallEvent>,
    fault: Option<String>,
}

impl HttpServerLabInput {
    pub(crate) fn offer(
        &mut self,
        submission: InboundSubmission,
        action: u64,
    ) -> Result<(), String> {
        if self.pending.is_some() || self.terminal.is_some() {
            return Err(String::from(
                "the preceding HttpServer Lab request has not settled",
            ));
        }
        self.pending = Some(InboundProposal {
            reference: InboundProposalHandle::new(action),
            submission,
            session: None,
        });
        Ok(())
    }

    pub(crate) fn is_pending(&self) -> bool {
        self.pending.is_some()
    }

    pub(crate) fn take_fault(&mut self) -> Option<String> {
        self.fault.take()
    }
}

struct HttpServerLabDriver {
    input: Rc<RefCell<HttpServerLabInput>>,
}

impl InboundConnectorDriver for HttpServerLabDriver {
    fn proposal(&mut self) -> Option<InboundProposal> {
        self.input.borrow().pending.clone()
    }

    fn decide(&mut self, reference: InboundProposalHandle, decision: InboundDecision) {
        let mut input = self.input.borrow_mut();
        if input
            .pending
            .as_ref()
            .is_none_or(|proposal| proposal.reference != reference)
        {
            input.fault = Some(String::from(
                "the HttpServer Lab received an unmatched admission decision",
            ));
            return;
        }
        input.pending = None;
        if let InboundDecision::Accepted(accepted) = decision {
            match accepted.exchange {
                Some(InboundExchangeRef::Call(call_id)) if input.terminal.is_none() => {
                    // A fixture scripts a decoded request and successful fixed
                    // ACK. Only the accepting commit supplies its Call identity.
                    input.terminal = Some(InboundCallEvent {
                        call_id,
                        completion: Completion::http_server_exit(HttpServerExit::Completed {
                            status: 202,
                        }),
                    });
                }
                _ => {
                    input.fault = Some(String::from(
                        "the HttpServer Lab acknowledgement has no admitted Call or free terminal slot",
                    ))
                }
            }
        }
    }

    fn call(&mut self) -> Option<&mut dyn InboundCallConnectorDriver> {
        Some(self)
    }
}

impl InboundCallConnectorDriver for HttpServerLabDriver {
    fn start_response(&mut self, _: &InboundCallResponse) -> InboundCallResponseStart {
        InboundCallResponseStart::Pending
    }

    fn start_timeout(&mut self, _: &InboundCallTimeout) -> InboundCallTimeoutStart {
        InboundCallTimeoutStart::Pending
    }

    fn events(&mut self) -> Vec<InboundCallEvent> {
        self.input
            .borrow_mut()
            .terminal
            .take()
            .into_iter()
            .collect()
    }
}

struct HttpServerLabFactory {
    id: ContractId,
    input: Rc<RefCell<HttpServerLabInput>>,
}

impl super::ConnectorDriverFactory for HttpServerLabFactory {
    fn connector_id(&self) -> &ContractId {
        &self.id
    }

    fn roles(&self) -> ConnectorRoles {
        ConnectorRoles::Inbound
    }

    fn bind<'plan>(
        &self,
        plan: &'plan ValidatedExecutionPlan,
        nodes: &[crate::node::Path],
    ) -> Result<Box<dyn super::BoundConnectorDriver + 'plan>, super::DriverFactoryError> {
        let expected: BTreeSet<_> = nodes.iter().collect();
        let actual: BTreeSet<_> = plan
            .plan()
            .nodes
            .iter()
            .filter(|node| {
                node.connector().is_some_and(|spec| {
                    spec.contract.as_str() == crate::connector::HTTP_SERVER_CONNECTOR_ID
                })
            })
            .map(|node| &node.path)
            .collect();
        if expected != actual {
            return Err(super::DriverFactoryError::new(
                "the HttpServer Lab binder received an incomplete Connector set",
            ));
        }
        Ok(Box::new(HttpServerLabDriver {
            input: Rc::clone(&self.input),
        }))
    }
}

impl super::BoundConnectorDriver for HttpServerLabDriver {
    fn inbound(&mut self) -> Option<&mut dyn InboundConnectorDriver> {
        Some(self)
    }

    fn outbound(&mut self) -> Option<&mut dyn crate::boundary::OutboundConnectorDriver> {
        None
    }
}

pub(crate) fn lab_factory(
    input: Rc<RefCell<HttpServerLabInput>>,
) -> Result<Box<dyn super::ConnectorDriverFactory>, super::DriverFactoryError> {
    let id = ContractId::parse(crate::connector::HTTP_SERVER_CONNECTOR_ID)
        .map_err(|error| super::DriverFactoryError::new(error.to_string()))?;
    Ok(Box::new(HttpServerLabFactory { id, input }))
}

#[cfg(test)]
mod lab_tests {
    use super::*;
    use crate::boundary::{
        Draft, InboundAdmission, InboundEnvelopeProvenance, InboundRejection, SubmissionSequence,
    };
    use crate::connectors::{DriverRegistry, RuntimeBindings};
    use crate::envelope::{Data, Value};
    use crate::journal::{Journal, MemJournal, Record};
    use crate::plan::GraphVersion;
    use crate::runtime::vm::Vm;

    fn submission(value: Value) -> InboundSubmission {
        InboundSubmission {
            provenance: InboundEnvelopeProvenance::single(
                crate::node::Path::new(["listener"]).unwrap(),
            ),
            port: crate::node::Port::new("out").unwrap(),
            draft: Draft {
                data: Data::materialized(value),
                ext: Default::default(),
            },
            sequence: SubmissionSequence::Next,
        }
    }

    fn compile_source() -> crate::compiler::Compilation {
        crate::compiler::compile(
            &[include_str!("../../tests/fixtures/lab-http-source.rkg")],
            &crate::contract::ContractCatalog::try_m0().unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn the_lab_source_never_acknowledges_before_commit_or_after_rejection() {
        let compilation = compile_source();
        let input = Rc::new(RefCell::new(HttpServerLabInput::default()));
        let registry = DriverRegistry::default()
            .try_with(lab_factory(Rc::clone(&input)).unwrap())
            .unwrap();
        let mut bindings = RuntimeBindings::try_new(&compilation.plan, &registry).unwrap();
        let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
        let mut journal = MemJournal::new(GraphVersion(1));
        input
            .borrow_mut()
            .offer(submission(Value::Int(17)), 0)
            .unwrap();
        assert_eq!(
            bindings.pump_inbound_call_events(&mut vm).unwrap().queued,
            0
        );
        assert_eq!(
            bindings.pump_inbound(&mut vm, &mut journal).unwrap(),
            Some(InboundAdmission::Rejected(InboundRejection::InvalidPayload))
        );
        assert!(journal.records().is_empty());
        assert!(!input.borrow().is_pending());
        assert_eq!(
            bindings.pump_inbound_call_events(&mut vm).unwrap().queued,
            0
        );

        input
            .borrow_mut()
            .offer(submission(Value::Text("accepted".into())), 1)
            .unwrap();
        let proposed = bindings.next_inbound().unwrap().unwrap();
        assert_eq!(
            bindings.next_inbound().unwrap().unwrap().proposal(),
            proposed.proposal()
        );
        assert!(journal.records().is_empty());
        assert_eq!(
            bindings.pump_inbound_call_events(&mut vm).unwrap().queued,
            0
        );
        let Some(InboundAdmission::Accepted(accepted)) =
            bindings.pump_inbound(&mut vm, &mut journal).unwrap()
        else {
            panic!("accepted source")
        };
        assert!(journal.find_envelope(accepted.envelope).is_some());
        assert!(matches!(journal.records()[0], Record::Inbound(_)));
        assert!(
            input
                .borrow_mut()
                .offer(submission(Value::Text("cannot overwrite an ACK".into())), 2)
                .is_err()
        );
        assert_eq!(
            bindings.pump_inbound_call_events(&mut vm).unwrap().queued,
            1
        );
        assert_eq!(
            bindings.pump_inbound_call_events(&mut vm).unwrap().queued,
            0,
            "one ACK only"
        );
        vm.drain_with_bindings(&mut journal, &mut bindings).unwrap();
        assert!(journal.open_inbound_calls().is_empty());
        assert!(input.borrow_mut().take_fault().is_none());
    }

    #[test]
    fn a_full_mailbox_retains_the_same_source_proposal_without_an_ack() {
        let compilation = compile_source();
        let input = Rc::new(RefCell::new(HttpServerLabInput::default()));
        let registry = DriverRegistry::default()
            .try_with(lab_factory(Rc::clone(&input)).unwrap())
            .unwrap();
        let mut bindings = RuntimeBindings::try_new(&compilation.plan, &registry).unwrap();
        let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
        let mut journal = MemJournal::new(GraphVersion(1));
        let capacity = vm.inspection().mailbox_capacity;
        for action in 0..capacity {
            input
                .borrow_mut()
                .offer(submission(Value::Text("queued".into())), action as u64)
                .unwrap();
            assert!(matches!(
                bindings.pump_inbound(&mut vm, &mut journal).unwrap(),
                Some(InboundAdmission::Accepted(_))
            ));
            assert_eq!(
                bindings.pump_inbound_call_events(&mut vm).unwrap().queued,
                1
            );
        }
        input
            .borrow_mut()
            .offer(submission(Value::Text("waiting".into())), capacity as u64)
            .unwrap();
        let records_before = journal.records().len();
        let retained = bindings.next_inbound().unwrap().unwrap();
        for _ in 0..2 {
            assert_eq!(
                bindings.pump_inbound(&mut vm, &mut journal).unwrap(),
                Some(InboundAdmission::Pending)
            );
            assert_eq!(journal.records().len(), records_before);
            assert_eq!(
                bindings.next_inbound().unwrap().unwrap().proposal(),
                retained.proposal()
            );
            assert_eq!(
                bindings.pump_inbound_call_events(&mut vm).unwrap().queued,
                0
            );
        }
        assert!(
            input
                .borrow_mut()
                .offer(submission(Value::Text("replacement".into())), 9999)
                .is_err()
        );
        vm.drain_with_bindings(&mut journal, &mut bindings).unwrap();
        assert!(matches!(
            bindings.pump_inbound(&mut vm, &mut journal).unwrap(),
            Some(InboundAdmission::Accepted(_))
        ));
        assert_eq!(
            bindings.pump_inbound_call_events(&mut vm).unwrap().queued,
            1
        );
        assert!(!input.borrow().is_pending());
        assert!(input.borrow_mut().take_fault().is_none());
    }
}
