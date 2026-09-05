//! Checked Operator decision algebra, shared by the source HIR and core proof.
//!
//! Source spans and compiler inference state never enter the tree.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

use super::Emission;
use crate::schema_annotation::AnnotationTable;
use crate::{expr, node};

/// Maximum nested decisions on a Dispatch path, including a desugared `where`.
/// The parser and independent core proof enforce the same resource bound.
pub const MAX_DISPATCH_DEPTH: usize = 32;

/// Recursive, typed Operator decision tree.
#[derive(Debug, Clone, PartialEq)]
pub enum Dispatch {
    /// Rectilinear statements in semantic source order.
    Linear(Vec<DispatchStep>),
    /// Closed selection over a tagged union, enum, or bool. Arms are sorted
    /// by their canonical case sets; their source order is erased.
    ClosedMatch {
        /// Pure local bindings evaluated before the scrutinee.
        prefix: Vec<DispatchStep>,
        /// Checked closed-domain expression.
        scrutinee: expr::Tree,
        /// Pairwise-disjoint, exhaustive arms.
        arms: Vec<ClosedArm>,
    },
    /// Ordered predicate selection; first true wins and fallback is required.
    OpenMatch {
        /// Pure local bindings evaluated before the predicates.
        prefix: Vec<DispatchStep>,
        /// Predicates and nested dispatches in source order.
        arms: Vec<OpenArm>,
        /// Exact `_` fallback.
        fallback: Box<Dispatch>,
    },
}

impl Dispatch {
    /// A consuming filter in its canonical decision form: one predicate,
    /// no prefix, and an empty fallback. The accepted branch may be recursive.
    #[must_use]
    pub fn filter_parts(&self) -> Option<(&expr::Tree, &Self)> {
        let Self::OpenMatch {
            prefix,
            arms,
            fallback,
        } = self
        else {
            return None;
        };
        if !prefix.is_empty() || arms.len() != 1 || !fallback.as_linear()?.is_empty() {
            return None;
        }
        Some((&arms[0].predicate, &arms[0].dispatch))
    }

    /// The statements of a linear leaf.
    #[must_use]
    pub fn as_linear(&self) -> Option<&[DispatchStep]> {
        match self {
            Self::Linear(statements) => Some(statements),
            Self::ClosedMatch { .. } | Self::OpenMatch { .. } => None,
        }
    }
}

/// One canonical closed-match arm.
#[derive(Debug, Clone, PartialEq)]
pub struct ClosedArm {
    /// Sorted, non-empty symbolic cases covered by this arm.
    pub cases: Vec<String>,
    /// Nested dispatch for those cases.
    pub dispatch: Box<Dispatch>,
}

/// One ordered open-match arm.
#[derive(Debug, Clone, PartialEq)]
pub struct OpenArm {
    /// Boolean predicate.
    pub predicate: expr::Tree,
    /// Nested dispatch when it is the first true predicate.
    pub dispatch: Box<Dispatch>,
}

/// One checked statement.
#[derive(Debug, Clone, PartialEq)]
pub enum DispatchStep {
    /// Handler-local constant binding.
    Let {
        /// The binding name.
        name: String,
        /// The compiled expression.
        value: expr::Tree,
        /// Sparse policy table beside the local binding's explicit type.
        annotations: AnnotationTable,
    },
    /// Emission on a port.
    Emit {
        /// The emission port.
        port: node::Port,
        /// What is emitted.
        value: Emission,
    },
    /// Ordered atomic production of several distinct envelopes.
    MultiEmit {
        /// Output targets in authored order.
        ports: Vec<node::Port>,
        /// Broadcast or positionally paired values.
        values: MultiEmitValues,
    },
    /// Write to a declared state cell.
    SetVar {
        /// The cell name.
        name: String,
        /// The compiled expression.
        value: expr::Tree,
    },
    /// Bare `stop`: close this Node's current logical instance.
    Stop,
    /// A durable command produced for the owner's Loop manager.
    LoopCommand(LoopCommand),
}

impl DispatchStep {
    /// Business emissions in authored order. A broadcast retains one builder
    /// with several targets; it is not expanded into repeated evaluations.
    pub fn emissions(&self) -> impl Iterator<Item = (&node::Port, &Emission)> {
        let mut index = 0;
        core::iter::from_fn(move || {
            let current = index;
            let result = match self {
                Self::Emit { port, value } if current == 0 => Some((port, value)),
                Self::MultiEmit { ports, values } => {
                    let port = ports.get(current)?;
                    let value = match values {
                        MultiEmitValues::Broadcast(value) => value,
                        MultiEmitValues::Tuple(values) => values.get(current)?,
                    };
                    Some((port, value))
                }
                Self::Let { .. }
                | Self::Emit { .. }
                | Self::SetVar { .. }
                | Self::Stop
                | Self::LoopCommand(_) => None,
            };
            if result.is_some() {
                index += 1;
            }
            result
        })
    }

    /// Every expression which can influence this statement. A broadcast's
    /// builder is visited once, while tuple builders retain their order.
    pub(crate) fn expressions(&self) -> impl Iterator<Item = &expr::Tree> {
        let scalar = match self {
            Self::Let { value, .. } | Self::SetVar { value, .. } => Some(value),
            Self::Emit { .. } | Self::MultiEmit { .. } | Self::Stop | Self::LoopCommand(_) => None,
        };
        let single = match self {
            Self::Emit { value, .. }
            | Self::MultiEmit {
                values: MultiEmitValues::Broadcast(value),
                ..
            }
            | Self::LoopCommand(LoopCommand::Next(value)) => Some(value),
            Self::Let { .. }
            | Self::SetVar { .. }
            | Self::Stop
            | Self::MultiEmit {
                values: MultiEmitValues::Tuple(_),
                ..
            }
            | Self::LoopCommand(LoopCommand::Again | LoopCommand::Done) => None,
        };
        let tuple = match self {
            Self::MultiEmit {
                values: MultiEmitValues::Tuple(values),
                ..
            } => values.as_slice(),
            Self::Let { .. }
            | Self::Emit { .. }
            | Self::SetVar { .. }
            | Self::Stop
            | Self::MultiEmit {
                values: MultiEmitValues::Broadcast(_),
                ..
            }
            | Self::LoopCommand(_) => &[],
        };
        scalar.into_iter().chain(
            single
                .into_iter()
                .chain(tuple)
                .flat_map(Emission::expressions),
        )
    }
}

/// Right-hand side shape of an atomic multi-emission.
#[derive(Debug, Clone, PartialEq)]
pub enum MultiEmitValues {
    /// Evaluate once, then build one envelope for every target.
    Broadcast(Emission),
    /// Evaluate and pair values with targets from left to right.
    Tuple(Vec<Emission>),
}

/// Loop manager commands. They are neither ports nor spatial emissions.
#[derive(Debug, Clone, PartialEq)]
pub enum LoopCommand {
    /// Supply the next entry envelope.
    Next(Emission),
    /// Retain the current `Loop.state`.
    Again,
    /// Close without a business envelope.
    Done,
}

impl LoopCommand {
    /// Discriminant independent of the next-state envelope builder.
    #[must_use]
    pub const fn kind(&self) -> super::LoopCommandKind {
        match self {
            Self::Next(_) => super::LoopCommandKind::Next,
            Self::Again => super::LoopCommandKind::Again,
            Self::Done => super::LoopCommandKind::Done,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        datatype::DataType,
        expr::{Expr, Tree},
        value::Value,
    };
    use alloc::vec;

    fn value(number: i64) -> Emission {
        Emission::Data(Tree {
            root: Expr::Literal(Value::Int(number)),
            ty: DataType::INT,
        })
    }

    #[test]
    fn broadcast_visits_one_builder_but_preserves_every_authored_target() {
        let step = DispatchStep::MultiEmit {
            ports: vec![node::Port::new("z").unwrap(), node::Port::new("a").unwrap()],
            values: MultiEmitValues::Broadcast(value(7)),
        };
        let outputs: Vec<_> = step.emissions().collect();
        assert_eq!(
            outputs
                .iter()
                .map(|(port, _)| port.name())
                .collect::<Vec<_>>(),
            vec!["z", "a"]
        );
        assert!(core::ptr::eq(outputs[0].1, outputs[1].1));
        assert_eq!(step.expressions().count(), 1);
    }

    #[test]
    fn tuple_visits_values_in_emission_order_without_sorting_targets() {
        let step = DispatchStep::MultiEmit {
            ports: vec![node::Port::new("z").unwrap(), node::Port::new("a").unwrap()],
            values: MultiEmitValues::Tuple(vec![value(2), value(1)]),
        };
        assert_eq!(
            step.emissions()
                .map(|(port, value)| (port.name(), value.clone()))
                .collect::<Vec<_>>(),
            vec![("z", value(2)), ("a", value(1))]
        );
        assert_eq!(
            step.expressions()
                .map(|tree| tree.root.clone())
                .collect::<Vec<_>>(),
            vec![Expr::Literal(Value::Int(2)), Expr::Literal(Value::Int(1))]
        );
    }

    #[test]
    fn commands_are_not_business_emissions_and_next_keeps_its_expression_dependency() {
        let next = DispatchStep::LoopCommand(LoopCommand::Next(value(9)));
        assert_eq!(next.emissions().count(), 0);
        assert_eq!(next.expressions().count(), 1);
        for command in [LoopCommand::Again, LoopCommand::Done] {
            let step = DispatchStep::LoopCommand(command);
            assert_eq!(step.emissions().count(), 0);
            assert_eq!(step.expressions().count(), 0);
        }
    }
}
