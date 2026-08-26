//! HttpServer's effectless Lab Driver.
//!
//! Scenario injections enter through the ordinary Lab boundary. The hosted
//! listener lands in H5 without sharing a source module with HttpClient.

use std::collections::BTreeSet;

use crate::boundary::{
    InboundConnectorDriver, InboundDecision, InboundProposal, InboundProposalRef,
};
use crate::connector::{ConnectorId, ConnectorRoles};
use crate::plan::ValidatedExecutionPlan;

#[derive(Default)]
struct PassiveInbound;

impl InboundConnectorDriver for PassiveInbound {
    fn proposal(&mut self) -> Option<InboundProposal> {
        None
    }

    fn decide(&mut self, _: InboundProposalRef, _: InboundDecision) {}
}

struct HttpServerLabFactory {
    id: ConnectorId,
}

impl super::ConnectorDriverFactory for HttpServerLabFactory {
    fn connector_id(&self) -> &ConnectorId {
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
                "the HttpServer Lab binder received an incomplete family",
            ));
        }
        Ok(Box::new(PassiveInbound))
    }
}

impl super::BoundConnectorDriver for PassiveInbound {
    fn inbound(&mut self) -> Option<&mut dyn InboundConnectorDriver> {
        Some(self)
    }

    fn outbound(&mut self) -> Option<&mut dyn crate::boundary::OutboundConnectorDriver> {
        None
    }
}

pub(crate) fn lab_factory()
-> Result<Box<dyn super::ConnectorDriverFactory>, super::DriverFactoryError> {
    let id = ConnectorId::parse(crate::connector::HTTP_SERVER_CONNECTOR_ID)
        .map_err(|error| super::DriverFactoryError::new(error.to_string()))?;
    Ok(Box::new(HttpServerLabFactory { id }))
}
