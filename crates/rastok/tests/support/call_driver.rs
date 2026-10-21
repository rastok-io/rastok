//! Explicit single-Driver routing for tests that do not exercise the hosted registry.

use rastok::connector::{
    driver::{DriverRouteError, OutboundCall, OutboundDrivers},
    exchange::{
        CallId, CallOpen, CallResponse, CallStart, Completion, RequestPreparation,
        SchedulerCompletion,
    },
};
use rastok::{
    envelope::{Data, Envelope},
    node,
    value::Value,
};
use std::collections::BTreeMap;

/// Borrows one test Driver without making every Call Driver a routing table.
pub(crate) struct CallDriver<'a>(pub(crate) &'a mut dyn OutboundCall);

impl OutboundDrivers for CallDriver<'_> {
    fn prepare_request(
        &mut self,
        connector: &node::Path,
        data: &Data,
        settings: &BTreeMap<String, Value>,
    ) -> Result<RequestPreparation, DriverRouteError> {
        Ok(self.0.prepare_request(connector, data, settings))
    }

    fn scheduler_completion(
        &mut self,
        connector: &node::Path,
        completion: SchedulerCompletion,
    ) -> Result<Completion, DriverRouteError> {
        Ok(self.0.scheduler_completion(connector, completion))
    }

    fn cancel(&mut self, _: &node::Path, call: &CallId) -> Result<(), DriverRouteError> {
        self.0.cancel(call);
        Ok(())
    }

    fn start(
        &mut self,
        call: &CallOpen,
        request: &Envelope,
    ) -> Result<CallStart, DriverRouteError> {
        Ok(self.0.start(call, request))
    }

    fn poll_responses(&mut self) -> Result<Vec<CallResponse>, DriverRouteError> {
        Ok(self.0.poll_responses())
    }
}
