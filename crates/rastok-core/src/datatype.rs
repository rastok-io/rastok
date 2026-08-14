//! Logical payload types owned by the Rastok core.
//!
//! Expression engines, ports, state, metadata and codecs consume this
//! vocabulary; none of them defines a parallel type system.  This is the
//! executable subset admitted before the M1.3 categorical type-registry
//! landing.  New logical families extend this authority rather than being
//! added to an expression-specific enum.

use alloc::boxed::Box;

/// The currently executable subset of Rastok's logical [`DataType`] tree.
///
/// `Variant` is the explicit dynamic value type; the type algebra deliberately
/// contains no inference-hole or recovery variant.  Some pre-M1.3 compiler
/// paths still collapse an unresolved constraint into `Variant`; the B2
/// migration removes those callers without adding such a case here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DataType {
    /// Boolean.
    Bool,
    /// The currently admitted signed-integer domain.
    Int,
    /// The currently admitted unsigned-integer domain.
    Uint,
    /// IEEE-754 binary64.
    Double,
    /// UTF-8 text.
    Text,
    /// Opaque bytes.
    Bytes,
    /// Nanosecond-resolution duration.
    Duration,
    /// UTC instant at nanosecond resolution.
    Timestamp,
    /// The null type.
    Null,
    /// The explicitly dynamic, self-describing value type.
    Variant,
    /// Homogeneous ordered values.
    Array(Box<DataType>),
    /// Text-keyed homogeneous values in the current executable subset.
    Map(Box<DataType>),
}
