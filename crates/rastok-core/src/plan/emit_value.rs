//! Envelope-construction expressions shared by checking, transport, proofs and execution.

use alloc::vec::Vec;

use crate::{envelope::WriteTarget, expr};

/// The expression evaluated to build an envelope, not a computed value.
/// Field writes retain their authored evaluation order.
#[derive(Debug, Clone, PartialEq)]
pub enum EmitValue {
    /// Re-emit the input envelope, preserving its payload representation.
    Identity,
    /// Inherit the input and apply the listed data/metadata writes.
    Patch(Vec<FieldWrite>),
    /// Build fresh data while inheriting the input's heritable metadata.
    /// A whole-data write may replace the initial struct with a scalar or
    /// carry an existing payload witness; explicit metadata writes win.
    FreshData(Vec<FieldWrite>),
    /// A computed value becoming the whole `data`.
    Data(expr::Tree),
    /// No emission on this port in this transaction.
    Nothing,
}

impl EmitValue {
    /// Expressions in the order the envelope builder evaluates them.
    pub(crate) fn expressions(&self) -> impl Iterator<Item = &expr::Tree> {
        let data = match self {
            Self::Data(value) => Some(value),
            Self::Identity | Self::Nothing | Self::Patch(_) | Self::FreshData(_) => None,
        };
        let writes = match self {
            Self::Patch(writes) | Self::FreshData(writes) => writes.as_slice(),
            Self::Identity | Self::Nothing | Self::Data(_) => &[],
        };
        data.into_iter()
            .chain(writes.iter().map(|write| &write.value))
    }
}

/// One resolved write in an envelope builder. Source spelling and diagnostic
/// spans are absent: all consumers use the same canonical landing site.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldWrite {
    /// The checked data or metadata destination.
    pub target: WriteTarget,
    /// The checked expression evaluated at this position in the builder.
    pub value: expr::Tree,
}
