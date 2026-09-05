//! Pure selection of one rectilinear handler branch. Only the chosen path's
//! prefixes and decisions run; no state write, emission or command runs here.

use alloc::collections::BTreeMap;
use alloc::string::String;

use crate::datatype::DataType;
use crate::expr::{self, Bindings, EvalValue};
use crate::plan::{Dispatch, DispatchStep, MAX_DISPATCH_DEPTH};

/// The selected branch and its evaluated, transaction-local pure prefixes.
#[derive(Debug)]
pub struct DispatchSelection<'a> {
    /// Statements to prepare atomically, in authored order.
    pub statements: &'a [DispatchStep],
    /// Locals visible only along the selected decision path. Payload witnesses
    /// remain lazy and retain the supplied environment's provenance.
    pub locals: BTreeMap<String, EvalValue>,
}

/// Selects exactly one branch against explicit bindings. The caller supplies
/// payloads, assets and logical time through the ordinary expression boundary.
/// It must then prepare all selected statements before committing any effect.
///
/// The checked Dispatch must have passed the independent structural/type
/// proof. This evaluator additionally refuses malformed prefixes, depth,
/// non-boolean predicates and missing/ambiguous closed selections. It never
/// evaluates another branch after an evaluation fault.
///
/// # Errors
/// Returns an expression fault without changing the input environment.
pub fn select_dispatch<'a>(
    dispatch: &'a Dispatch,
    bindings: &dyn Bindings,
) -> Result<DispatchSelection<'a>, expr::Error> {
    let mut current = dispatch;
    let mut locals = BTreeMap::new();
    let mut depth = 0;
    loop {
        let prefix = match current {
            Dispatch::Linear(statements) => {
                return Ok(DispatchSelection { statements, locals });
            }
            Dispatch::ClosedMatch { prefix, .. } | Dispatch::OpenMatch { prefix, .. } => prefix,
        };
        if depth >= MAX_DISPATCH_DEPTH {
            return Err(fault("Dispatch exceeds its decision depth budget"));
        }
        depth += 1;
        for step in prefix {
            let DispatchStep::Let { name, value, .. } = step else {
                return Err(fault("only pure local bindings may precede a match"));
            };
            if locals.contains_key(name) || bindings.get(name).is_some() {
                return Err(fault("a Dispatch prefix shadows a visible binding"));
            }
            let value = expr::eval(
                value,
                &LocalBindings {
                    bindings,
                    locals: &locals,
                },
            )?;
            locals.insert(name.clone(), value);
        }
        let scope = LocalBindings {
            bindings,
            locals: &locals,
        };
        current = match current {
            Dispatch::ClosedMatch {
                scrutinee, arms, ..
            } => {
                let value = decision_value(scrutinee, &scope)?;
                let case = match (&scrutinee.ty, &value) {
                    (DataType::Bool, EvalValue::Bool(true)) => "true",
                    (DataType::Bool, EvalValue::Bool(false)) => "false",
                    (DataType::Enum(_), EvalValue::Enum(case))
                    | (DataType::TaggedUnion(_), EvalValue::TaggedUnion { case, .. }) => case,
                    _ => return Err(fault("a closed match received a value outside its domain")),
                };
                let mut selected = None;
                for arm in arms {
                    if arm.cases.iter().any(|candidate| candidate == case) {
                        if selected.is_some() {
                            return Err(fault("a closed match selected more than one arm"));
                        }
                        selected = Some(arm.dispatch.as_ref());
                    }
                }
                selected.ok_or_else(|| fault("a closed match selected no arm"))?
            }
            Dispatch::OpenMatch { arms, fallback, .. } => {
                let mut selected = fallback.as_ref();
                for arm in arms {
                    match decision_value(&arm.predicate, &scope)? {
                        EvalValue::Bool(true) => {
                            selected = &arm.dispatch;
                            break;
                        }
                        EvalValue::Bool(false) => {}
                        _ => return Err(fault("a match predicate did not evaluate to bool")),
                    }
                }
                selected
            }
            Dispatch::Linear(_) => return Err(fault("a linear Dispatch has no decision")),
        };
    }
}

fn decision_value(tree: &expr::Tree, bindings: &dyn Bindings) -> Result<EvalValue, expr::Error> {
    match expr::eval(tree, bindings)? {
        EvalValue::Payload(reference) => bindings.payload(&reference),
        value => Ok(value),
    }
}

struct LocalBindings<'a> {
    bindings: &'a dyn Bindings,
    locals: &'a BTreeMap<String, EvalValue>,
}

impl Bindings for LocalBindings<'_> {
    fn get(&self, name: &str) -> Option<EvalValue> {
        self.locals
            .get(name)
            .cloned()
            .or_else(|| self.bindings.get(name))
    }

    fn payload(&self, reference: &expr::PayloadRef) -> Result<EvalValue, expr::Error> {
        self.bindings.payload(reference)
    }

    fn asset(&self, index: u32) -> Result<EvalValue, expr::Error> {
        self.bindings.asset(index)
    }
}

fn fault(message: &str) -> expr::Error {
    expr::Error {
        kind: expr::ErrorKind::Evaluation,
        message: message.into(),
    }
}

#[cfg(test)]
mod tests;
