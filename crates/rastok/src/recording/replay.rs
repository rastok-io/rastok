use super::Error;
use crate::boundary::*;
use crate::connectors::{ScriptedConnectorProtocol, scripted_protocol_binders};
use crate::deployment::BootBindings;
use crate::envelope::{Data, Envelope};
use crate::node;
use crate::plan::ValidatedExecutionPlan;
use crate::value::Value;
use std::collections::BTreeMap;

/// Only pure scheduler projections are reachable. No physical registry,
/// Process object, socket, File root or secret provider is ever constructed.
pub(super) struct Contracts {
    protocols: BTreeMap<node::Path, Box<dyn ScriptedConnectorProtocol>>,
}
impl Contracts {
    pub(super) fn bind(plan: &ValidatedExecutionPlan) -> Result<Self, Error> {
        let mut protocols = BTreeMap::new();
        for node in &plan.nodes {
            let Some(contract) = node.connector() else {
                continue;
            };
            if let Some((_, binder)) = scripted_protocol_binders()
                .into_iter()
                .find(|(id, _)| *id == contract.contract.as_str())
            {
                let protocol = binder(
                    plan,
                    std::slice::from_ref(&node.path),
                    &BootBindings::empty(),
                )
                .map_err(|error| Error::Contract(error.to_string()))?;
                protocols.insert(node.path.clone(), protocol);
            }
        }
        Ok(Self { protocols })
    }
}
impl OutboundConnectorBindings for Contracts {
    fn scheduler_completion(
        &mut self,
        connector: &node::Path,
        completion: SchedulerCompletion,
    ) -> Result<Completion, DriverRouteError> {
        self.protocols
            .get(connector)
            .map(|protocol| protocol.scheduler_completion(completion))
            .ok_or_else(|| DriverRouteError::UnboundOutbound(connector.clone()))
    }
    fn prepare_request(
        &mut self,
        connector: &node::Path,
        _: &Data,
        _: &BTreeMap<String, Value>,
    ) -> Result<RequestPreparation, DriverRouteError> {
        Err(DriverRouteError::UnboundOutbound(connector.clone()))
    }
    fn cancel(&mut self, connector: &node::Path, _: &CallRef) -> Result<(), DriverRouteError> {
        Err(DriverRouteError::UnboundOutbound(connector.clone()))
    }
    fn start(&mut self, intent: &Intent, _: &Envelope) -> Result<CallStart, DriverRouteError> {
        Err(DriverRouteError::UnboundOutbound(
            intent.connector_path.clone(),
        ))
    }
    fn events(&mut self) -> Result<Vec<Response>, DriverRouteError> {
        Ok(Vec::new())
    }
}
