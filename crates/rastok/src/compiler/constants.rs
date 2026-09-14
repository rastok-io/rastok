//! One capture-free expression substitution for checking and lowering.

use crate::expr;

/// A folded value keeps the type established before substitution. Going via
/// variant also preserves checked, but unselected, narrowed branches: their
/// casts remain lazy instead of rejecting an unreachable value at plan check.
pub(super) fn typed_literal(
    value: crate::value::Value,
    ty: crate::datatype::DataType,
) -> expr::Expr {
    let widen = expr::Expr::As {
        expr: Box::new(expr::Expr::Literal(value)),
        target: Box::new(expr::NarrowTarget::Type(Box::new(expr::NarrowType::new(
            crate::datatype::DataType::Variant,
        )))),
    };
    expr::Expr::As {
        expr: Box::new(widen),
        target: Box::new(expr::NarrowTarget::Type(Box::new(expr::NarrowType::new(
            ty,
        )))),
    }
}

pub(super) fn substitute(
    expr: &expr::Expr,
    resolve: &impl Fn(&expr::Expr) -> Option<expr::Expr>,
) -> expr::Expr {
    if let Some(replacement) = resolve(expr) {
        return replacement;
    }
    let recurse = |inner: &expr::Expr| Box::new(substitute(inner, resolve));
    match expr {
        expr::Expr::Asset(_) => expr.clone(),
        expr::Expr::Path(_) => expr.clone(),
        expr::Expr::Literal(_) => expr.clone(),
        expr::Expr::Unary { op, expr } => expr::Expr::Unary {
            op: *op,
            expr: recurse(expr),
        },
        expr::Expr::Binary { op, left, right } => expr::Expr::Binary {
            op: *op,
            left: recurse(left),
            right: recurse(right),
        },
        expr::Expr::Ternary {
            cond,
            then,
            otherwise,
        } => expr::Expr::Ternary {
            cond: recurse(cond),
            then: recurse(then),
            otherwise: recurse(otherwise),
        },
        expr::Expr::Index { base, index } => expr::Expr::Index {
            base: recurse(base),
            index: recurse(index),
        },
        expr::Expr::Call { function, args } => expr::Expr::Call {
            function: *function,
            args: args.iter().map(|arg| substitute(arg, resolve)).collect(),
        },
        expr::Expr::List(items) => {
            expr::Expr::List(items.iter().map(|item| substitute(item, resolve)).collect())
        }
        expr::Expr::MapLit(entries) => expr::Expr::MapLit(
            entries
                .iter()
                .map(|(key, value)| (key.clone(), substitute(value, resolve)))
                .collect(),
        ),
        expr::Expr::Struct(fields) => expr::Expr::Struct(
            fields
                .iter()
                .map(|(name, value)| (name.clone(), substitute(value, resolve)))
                .collect(),
        ),
        expr::Expr::Tagged {
            union,
            case,
            fields,
        } => expr::Expr::Tagged {
            union: union.clone(),
            case: case.clone(),
            fields: fields
                .iter()
                .map(|(name, value)| (name.clone(), substitute(value, resolve)))
                .collect(),
        },
        expr::Expr::Is { expr, target } => expr::Expr::Is {
            expr: recurse(expr),
            target: target.clone(),
        },
        expr::Expr::As { expr, target } => expr::Expr::As {
            expr: recurse(expr),
            target: target.clone(),
        },
    }
}
