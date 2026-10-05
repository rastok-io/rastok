//! Canonical decision and routing fields, independently checked at transport.
//!
//! These are fields, not executable packages. Their local proofs take an
//! independently established expression environment or source-type lookup.
//! Owner, port, effect, capacity and execution-capability proofs belong to the
//! enclosing plan. Decoding a field never grants permission to execute it.
//! Package validation must also count its enclosing table/handler framing
//! against the total nesting budget.

use alloc::{boxed::Box, string::String, vec, vec::Vec};
use core::fmt;

use super::{Cbor, array_of, emission_from, err, step_from, step_tree, tree_from, tree_tree};
use crate::plan::{
    ClosedArm, DecodeError, Dispatch, DispatchStep, DispatchValidationError, Endpoint,
    EndpointOwner, InstanceStopTarget, LoopCommand, LoopCommandKind, LoopCommandTarget,
    MAX_DISPATCH_DEPTH, MultiEmitValues, OpenArm, RouteArm, RouteDestination, RouteRule,
    RouteSelection, RouteValidationError, validate_dispatch, validate_routing,
};
use crate::{canonical_cbor, datatype::DataType, expr, node};

/// A field is unreadable, noncanonical, or contradicts its independent context.
#[derive(Debug, Clone, PartialEq)]
pub enum FieldError {
    /// The canonical CBOR reader or a typed field reader refused.
    Decode(DecodeError),
    /// The decision's expressions, partitions, locals or statements refuse.
    Dispatch(DispatchValidationError),
    /// A routing selection or its additive inventory refuses.
    Routing(RouteValidationError),
    /// The inventory's row order is not canonical.
    RouteOrder,
    /// Combined nesting of decisions, expressions, types and values is too deep.
    Nesting,
}

impl fmt::Display for FieldError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Decode(error) => error.fmt(f),
            Self::Dispatch(error) => error.fmt(f),
            Self::Routing(error) => error.fmt(f),
            Self::RouteOrder => f.write_str("routing rows are not in canonical encoded order"),
            Self::Nesting => f.write_str("combined plan-field nesting exceeds the CBOR budget"),
        }
    }
}

impl core::error::Error for FieldError {}

impl From<DecodeError> for FieldError {
    fn from(error: DecodeError) -> Self {
        Self::Decode(error)
    }
}

/// Encode the whole decision tree, preserving authored statement/open-arm order.
///
/// # Errors
/// Refuses invalid local proofs and the combined transport nesting budget.
pub fn encode_dispatch(
    dispatch: &Dispatch,
    signature: &expr::Signature,
) -> Result<Vec<u8>, FieldError> {
    validate_dispatch(dispatch, signature).map_err(FieldError::Dispatch)?;
    encode_field(&dispatch_tree(dispatch))
}

/// Decode and independently re-derive a decision field without repairing it.
///
/// # Errors
/// Refuses trailing bytes, unknown forms, noncanonical encoding and invalid
/// expression/partition/local-binding claims against `signature`.
pub fn decode_dispatch(bytes: &[u8], signature: &expr::Signature) -> Result<Dispatch, FieldError> {
    let tree = read_field(bytes)?;
    let dispatch = dispatch_from(&tree, 0)?;
    validate_dispatch(&dispatch, signature).map_err(FieldError::Dispatch)?;
    canonical(bytes, &dispatch_tree(&dispatch))?;
    Ok(dispatch)
}

/// Encode a complete routing inventory in strictly increasing encoded-row order.
///
/// # Errors
/// Refuses noncanonical rows or any selection/additive conflict. The lookup
/// supplies actual source types, never types asserted by the route itself.
pub fn encode_routing(
    rules: &[RouteRule],
    source_type: impl FnMut(&Endpoint) -> Option<DataType>,
) -> Result<Vec<u8>, FieldError> {
    validate_routing(rules, source_type).map_err(FieldError::Routing)?;
    routing_order_holds(rules)?;
    encode_field(&routing_tree(rules))
}

/// Decode a routing field; manager commands remain disjoint from spatial ports.
///
/// # Errors
/// Refuses malformed/cross-typed endpoints, unknown command tags, noncanonical
/// rows/sets, incomplete case partitions and additive conflicts.
pub fn decode_routing(
    bytes: &[u8],
    source_type: impl FnMut(&Endpoint) -> Option<DataType>,
) -> Result<Vec<RouteRule>, FieldError> {
    let tree = read_field(bytes)?;
    let rules = array_of(&tree, "routing inventory")?
        .iter()
        .map(route_from)
        .collect::<Result<Vec<_>, _>>()?;
    validate_routing(&rules, source_type).map_err(FieldError::Routing)?;
    routing_order_holds(&rules)?;
    canonical(bytes, &routing_tree(&rules))?;
    Ok(rules)
}

fn encode_field(tree: &Cbor) -> Result<Vec<u8>, FieldError> {
    // The component proofs bound recursive construction; this walk additionally
    // accounts for the *combined* decision/expression/type transport overhead.
    let mut pending = vec![(tree, 0usize)];
    while let Some((value, depth)) = pending.pop() {
        if depth >= canonical_cbor::MAX_NESTING {
            return Err(FieldError::Nesting);
        }
        match value {
            Cbor::Array(values) => pending.extend(values.iter().map(|value| (value, depth + 1))),
            Cbor::Map(values) => pending.extend(values.values().map(|value| (value, depth + 1))),
            _ => {}
        }
    }
    Ok(canonical_cbor::encode(tree))
}

fn read_field(bytes: &[u8]) -> Result<Cbor, FieldError> {
    let (tree, used) = canonical_cbor::decode(bytes).map_err(DecodeError::Read)?;
    if used != bytes.len() {
        return Err(err("trailing plan-field bytes").into());
    }
    Ok(tree)
}

fn canonical(bytes: &[u8], tree: &Cbor) -> Result<(), FieldError> {
    if encode_field(tree)? != bytes {
        return Err(DecodeError::NonCanonical("plan field").into());
    }
    Ok(())
}

fn steps_tree(steps: &[DispatchStep]) -> Cbor {
    Cbor::Array(steps.iter().map(step_tree).collect())
}

pub(super) fn dispatch_tree(dispatch: &Dispatch) -> Cbor {
    match dispatch {
        Dispatch::Linear(steps) => Cbor::Array(vec![Cbor::Uint(1), steps_tree(steps)]),
        Dispatch::ClosedMatch {
            prefix,
            scrutinee,
            arms,
        } => Cbor::Array(vec![
            Cbor::Uint(2),
            steps_tree(prefix),
            tree_tree(scrutinee),
            Cbor::Array(
                arms.iter()
                    .map(|arm| {
                        Cbor::Array(vec![strings_tree(&arm.cases), dispatch_tree(&arm.dispatch)])
                    })
                    .collect(),
            ),
        ]),
        Dispatch::OpenMatch {
            prefix,
            arms,
            fallback,
        } => Cbor::Array(vec![
            Cbor::Uint(3),
            steps_tree(prefix),
            Cbor::Array(
                arms.iter()
                    .map(|arm| {
                        Cbor::Array(vec![
                            tree_tree(&arm.predicate),
                            dispatch_tree(&arm.dispatch),
                        ])
                    })
                    .collect(),
            ),
            dispatch_tree(fallback),
        ]),
    }
}

pub(super) fn dispatch_from(tree: &Cbor, depth: usize) -> Result<Dispatch, DecodeError> {
    // Check before allocating children, even when called on an in-memory tree
    // which did not pass through the byte reader's independent nesting limit.
    if depth > MAX_DISPATCH_DEPTH {
        return Err(err("Dispatch decision depth"));
    }
    match array_of(tree, "Dispatch")? {
        [Cbor::Uint(1), steps] => Ok(Dispatch::Linear(steps_from(steps)?)),
        [Cbor::Uint(2), prefix, scrutinee, arms] if depth < MAX_DISPATCH_DEPTH => {
            Ok(Dispatch::ClosedMatch {
                prefix: steps_from(prefix)?,
                scrutinee: tree_from(scrutinee)?,
                arms: array_of(arms, "closed arms")?
                    .iter()
                    .map(|arm| {
                        let [cases, dispatch] = array_of(arm, "closed arm")? else {
                            return Err(err("closed arm"));
                        };
                        Ok(ClosedArm {
                            cases: strings_from(cases)?,
                            dispatch: Box::new(dispatch_from(dispatch, depth + 1)?),
                        })
                    })
                    .collect::<Result<_, DecodeError>>()?,
            })
        }
        [Cbor::Uint(3), prefix, arms, fallback] if depth < MAX_DISPATCH_DEPTH => {
            Ok(Dispatch::OpenMatch {
                prefix: steps_from(prefix)?,
                arms: array_of(arms, "open arms")?
                    .iter()
                    .map(|arm| {
                        let [predicate, dispatch] = array_of(arm, "open arm")? else {
                            return Err(err("open arm"));
                        };
                        Ok(OpenArm {
                            predicate: tree_from(predicate)?,
                            dispatch: Box::new(dispatch_from(dispatch, depth + 1)?),
                        })
                    })
                    .collect::<Result<_, DecodeError>>()?,
                fallback: Box::new(dispatch_from(fallback, depth + 1)?),
            })
        }
        _ => Err(err("Dispatch tag, arity or depth")),
    }
}

fn steps_from(tree: &Cbor) -> Result<Vec<DispatchStep>, DecodeError> {
    array_of(tree, "Dispatch statements")?
        .iter()
        .map(|tree| match array_of(tree, "statement")? {
            [Cbor::Uint(5), ports, values] => Ok(DispatchStep::MultiEmit {
                ports: strings_from(ports)?
                    .into_iter()
                    .map(|port| node::Port::new(port).map_err(|_| err("multi-emission port")))
                    .collect::<Result<_, _>>()?,
                values: match array_of(values, "multi-emission values")? {
                    [Cbor::Uint(1), value] => MultiEmitValues::Broadcast(emission_from(value)?),
                    [Cbor::Uint(2), values] => MultiEmitValues::Tuple(
                        array_of(values, "tuple emissions")?
                            .iter()
                            .map(emission_from)
                            .collect::<Result<_, _>>()?,
                    ),
                    _ => return Err(err("multi-emission values")),
                },
            }),
            [Cbor::Uint(6), command] => {
                let [Cbor::Uint(tag), payload @ ..] = array_of(command, "Loop command")? else {
                    return Err(err("Loop command"));
                };
                Ok(DispatchStep::LoopCommand(
                    match (command_kind_from(*tag)?, payload) {
                        (LoopCommandKind::Next, [value]) => {
                            LoopCommand::Next(emission_from(value)?)
                        }
                        (LoopCommandKind::Again, []) => LoopCommand::Again,
                        (LoopCommandKind::Done, []) => LoopCommand::Done,
                        _ => return Err(err("Loop command payload")),
                    },
                ))
            }
            _ => step_from(tree),
        })
        .collect()
}

fn strings_tree(values: &[String]) -> Cbor {
    Cbor::Array(values.iter().cloned().map(Cbor::Text).collect())
}

fn command_kind_from(tag: u64) -> Result<LoopCommandKind, DecodeError> {
    u8::try_from(tag)
        .ok()
        .and_then(LoopCommandKind::from_tag)
        .ok_or_else(|| err("Loop command kind"))
}

fn strings_from(tree: &Cbor) -> Result<Vec<String>, DecodeError> {
    array_of(tree, "names")?
        .iter()
        .map(|tree| match tree {
            Cbor::Text(value) => Ok(value.clone()),
            _ => Err(err("name")),
        })
        .collect()
}

fn routing_order_holds(rules: &[RouteRule]) -> Result<(), FieldError> {
    let mut previous = None;
    for rule in rules {
        let bytes = canonical_cbor::encode(&route_tree(rule));
        if previous.as_ref().is_some_and(|previous| previous >= &bytes) {
            return Err(FieldError::RouteOrder);
        }
        previous = Some(bytes);
    }
    Ok(())
}

pub(super) fn routing_tree(rules: &[RouteRule]) -> Cbor {
    Cbor::Array(rules.iter().map(route_tree).collect())
}

pub(super) fn route_tree(rule: &RouteRule) -> Cbor {
    Cbor::Array(vec![
        endpoint_tree(&rule.source),
        match &rule.selection {
            RouteSelection::All(destinations) => {
                Cbor::Array(vec![Cbor::Uint(1), destinations_tree(destinations)])
            }
            RouteSelection::Cases(arms) => Cbor::Array(vec![
                Cbor::Uint(2),
                Cbor::Array(
                    arms.iter()
                        .map(|arm| {
                            Cbor::Array(vec![
                                strings_tree(&arm.cases),
                                destinations_tree(&arm.destinations),
                            ])
                        })
                        .collect(),
                ),
            ]),
        },
    ])
}

fn destinations_tree(destinations: &[RouteDestination]) -> Cbor {
    Cbor::Array(
        destinations
            .iter()
            .map(|destination| match destination {
                RouteDestination::Port(endpoint) => {
                    Cbor::Array(vec![Cbor::Uint(1), endpoint_tree(endpoint)])
                }
                RouteDestination::Discard => Cbor::Array(vec![Cbor::Uint(2)]),
                RouteDestination::LoopCommand(command) => Cbor::Array(vec![
                    Cbor::Uint(3),
                    owner_tree(&command.owner),
                    Cbor::Uint(u64::from(command.kind.tag())),
                ]),
                RouteDestination::InstanceStop(target) => Cbor::Array(vec![
                    Cbor::Uint(4),
                    owner_tree(&EndpointOwner::Graph(target.scope.clone())),
                    owner_tree(&target.owner),
                ]),
            })
            .collect(),
    )
}

fn owner_tree(owner: &EndpointOwner) -> Cbor {
    let (tag, path) = match owner {
        EndpointOwner::Node(path) => (1, alloc::string::ToString::to_string(path)),
        EndpointOwner::Graph(path) => (2, alloc::string::ToString::to_string(path)),
    };
    Cbor::Array(vec![Cbor::Uint(tag), Cbor::Text(path)])
}

fn endpoint_tree(endpoint: &Endpoint) -> Cbor {
    Cbor::Array(vec![
        owner_tree(&endpoint.owner),
        Cbor::Text(endpoint.port.name().into()),
    ])
}

fn owner_from(tree: &Cbor) -> Result<EndpointOwner, DecodeError> {
    let parts = array_of(tree, "endpoint owner")?;
    match parts {
        [Cbor::Uint(1), Cbor::Text(path)] => {
            node::Path::new(path.split('.').map(String::from).collect::<Vec<_>>())
                .map(EndpointOwner::Node)
                .map_err(|_| err("Node owner path"))
        }
        [Cbor::Uint(2), Cbor::Text(path)] => {
            let segments = if path.is_empty() {
                Vec::new()
            } else {
                path.split('.').map(String::from).collect()
            };
            node::OwnerPath::new(segments)
                .map(EndpointOwner::Graph)
                .map_err(|_| err("Graph owner path"))
        }
        _ => Err(err("endpoint owner")),
    }
}

fn endpoint_from(tree: &Cbor) -> Result<Endpoint, DecodeError> {
    match array_of(tree, "route endpoint")? {
        [owner, Cbor::Text(port)] => Ok(Endpoint {
            owner: owner_from(owner)?,
            port: node::Port::new(port.clone()).map_err(|_| err("route port"))?,
        }),
        _ => Err(err("route endpoint")),
    }
}

pub(super) fn route_from(tree: &Cbor) -> Result<RouteRule, DecodeError> {
    let [source, selection] = array_of(tree, "route")? else {
        return Err(err("route"));
    };
    Ok(RouteRule {
        source: endpoint_from(source)?,
        selection: match array_of(selection, "route selection")? {
            [Cbor::Uint(1), destinations] => RouteSelection::All(destinations_from(destinations)?),
            [Cbor::Uint(2), arms] => RouteSelection::Cases(
                array_of(arms, "route arms")?
                    .iter()
                    .map(|arm| {
                        let [cases, destinations] = array_of(arm, "route arm")? else {
                            return Err(err("route arm"));
                        };
                        Ok(RouteArm {
                            cases: strings_from(cases)?,
                            destinations: destinations_from(destinations)?,
                        })
                    })
                    .collect::<Result<_, DecodeError>>()?,
            ),
            _ => return Err(err("route selection")),
        },
    })
}

fn destinations_from(tree: &Cbor) -> Result<Vec<RouteDestination>, DecodeError> {
    array_of(tree, "route destinations")?
        .iter()
        .map(
            |destination| match array_of(destination, "route destination")? {
                [Cbor::Uint(1), endpoint] => Ok(RouteDestination::Port(endpoint_from(endpoint)?)),
                [Cbor::Uint(2)] => Ok(RouteDestination::Discard),
                [Cbor::Uint(3), owner, Cbor::Uint(kind)] => {
                    Ok(RouteDestination::LoopCommand(LoopCommandTarget {
                        owner: owner_from(owner)?,
                        kind: command_kind_from(*kind)?,
                    }))
                }
                [Cbor::Uint(4), scope, owner] => {
                    let EndpointOwner::Graph(scope) = owner_from(scope)? else {
                        return Err(err("instance closure requires a Graph wiring scope"));
                    };
                    Ok(RouteDestination::InstanceStop(Box::new(
                        InstanceStopTarget {
                            scope,
                            owner: owner_from(owner)?,
                        },
                    )))
                }
                _ => Err(err("route destination")),
            },
        )
        .collect()
}

#[cfg(test)]
mod tests;
