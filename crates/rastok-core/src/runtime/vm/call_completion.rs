//! Contract production guarantees apply before publication, with or without Loop.

use super::*;

impl Vm<'_> {
    pub(super) fn validate_call_completion<'record>(
        &self,
        slot: Slot,
        record: &'record crate::journal::TransactionRecord,
    ) -> Result<Option<&'record Emission>, VmError> {
        let node = &self.plan.nodes[slot.index()];
        let Some(call) = node
            .connector()
            .and_then(crate::plan::ConnectorSpec::call_semantics)
        else {
            return Ok(None);
        };
        let violation = |reason| VmError::ContractViolation {
            node: node.path.clone(),
            reason,
        };
        let mut terminals = record
            .emissions
            .iter()
            .filter(|emission| emission.port.name() == call.terminal());
        let terminal = terminals
            .next()
            .ok_or_else(|| violation("a Call completion has no terminal"))?;
        if terminals.next().is_some() {
            return Err(violation("a Call completion has more than one terminal"));
        }
        let Data::Materialized(crate::value::Value::TaggedUnion(value)) = terminal.envelope.data()
        else {
            return Err(violation(
                "the Call terminal is not its closed tagged union",
            ));
        };
        let terminal_case = value.case.as_str();
        if !call
            .terminal_cases()
            .iter()
            .any(|(case, _)| *case == terminal_case)
        {
            return Err(violation("the Call terminal is outside its Contract"));
        }
        for port in &node.ports {
            use crate::connector::FrameProduction;
            let Some(production) = call.frame_production(port.port.name()) else {
                continue;
            };
            let count = record
                .emissions
                .iter()
                .filter(|emission| emission.port == port.port)
                .count();
            let valid = match production {
                FrameProduction::Never => count == 0,
                FrameProduction::ZeroOrMore => true,
                FrameProduction::ExactlyOneOnExit(case) => {
                    count == usize::from(case == terminal_case)
                }
            };
            if !valid {
                return Err(violation(
                    "Call frames contradict the Contract's terminal cardinality",
                ));
            }
        }
        for emission in &record.emissions {
            if emission.port.name() != call.terminal()
                && call.frame_production(emission.port.name()).is_none()
            {
                return Err(violation("a Call produced a port outside its Contract"));
            }
        }
        Ok(Some(terminal))
    }
}
