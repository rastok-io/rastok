//! Close source inference placeholders after port resolution. Package readers
//! never call this pass: a transported decision must already carry its proof.

use super::{Dispatch, DispatchStep, EmitValue, LoopCommand, MultiEmitValues};
use crate::{datatype::DataType, expr};
use alloc::vec;

fn resolve_tree(tree: &mut expr::Tree, scope: &expr::Signature) {
    if tree.ty == DataType::Variant
        && let Some(ty) = expr::rederive_type(&tree.root, scope)
    {
        tree.ty = ty;
    }
}

fn resolve_emission(value: &mut EmitValue, scope: &expr::Signature) {
    match value {
        EmitValue::Identity | EmitValue::Nothing => {}
        EmitValue::Data(tree) => resolve_tree(tree, scope),
        EmitValue::Patch(writes) | EmitValue::FreshData(writes) => {
            for write in writes {
                resolve_tree(&mut write.value, scope);
            }
        }
    }
}

pub(super) fn resolve(dispatch: &mut Dispatch, signature: expr::Signature) {
    let mut pending = vec![(dispatch, signature)];
    while let Some((dispatch, mut scope)) = pending.pop() {
        let steps = match dispatch {
            Dispatch::Linear(steps)
            | Dispatch::ClosedMatch { prefix: steps, .. }
            | Dispatch::OpenMatch { prefix: steps, .. } => steps,
        };
        for step in steps {
            match step {
                DispatchStep::Let { name, value, .. } => {
                    resolve_tree(value, &scope);
                    scope = scope.with(name.as_str(), value.ty.clone());
                }
                DispatchStep::SetVar { value, .. } => resolve_tree(value, &scope),
                DispatchStep::Emit { value, .. }
                | DispatchStep::LoopCommand(LoopCommand::Next(value)) => {
                    resolve_emission(value, &scope)
                }
                DispatchStep::MultiEmit { values, .. } => match values {
                    MultiEmitValues::Broadcast(value) => resolve_emission(value, &scope),
                    MultiEmitValues::Tuple(values) => {
                        for value in values {
                            resolve_emission(value, &scope);
                        }
                    }
                },
                DispatchStep::Stop
                | DispatchStep::LoopCommand(LoopCommand::Again | LoopCommand::Done) => {}
            }
        }
        match dispatch {
            Dispatch::Linear(_) => {}
            Dispatch::OpenMatch { arms, fallback, .. } => {
                for arm in arms {
                    resolve_tree(&mut arm.predicate, &scope);
                    pending.push((&mut arm.dispatch, scope.narrowed_by(&arm.predicate.root)));
                }
                pending.push((fallback, scope));
            }
            Dispatch::ClosedMatch {
                scrutinee, arms, ..
            } => {
                resolve_tree(scrutinee, &scope);
                for arm in arms {
                    let mut branch = scope.clone();
                    if matches!(&scrutinee.root, expr::Expr::Path(path) if path.len() == 1 && path[0] == "data")
                        && let Some(ty) =
                            super::routing::selected_case_type(&scrutinee.ty, &arm.cases)
                    {
                        branch = branch.with("data", ty);
                    }
                    pending.push((&mut arm.dispatch, branch));
                }
            }
        }
    }
}
