//! Hosted recording-frame/source helpers. Retained records use the core codec.

use super::Error;
use crate::canonical_cbor::Value as Cbor;
pub(super) use crate::journal::codec::{Decoder, encode};

pub(super) fn list(items: impl IntoIterator<Item = Cbor>) -> Cbor {
    Cbor::Array(items.into_iter().collect())
}
pub(super) fn word(value: impl ToString) -> Cbor {
    Cbor::Text(value.to_string())
}
pub(super) fn integer(value: i64) -> Cbor {
    if value >= 0 {
        Cbor::Uint(value as u64)
    } else {
        Cbor::Negative(value.unsigned_abs() - 1)
    }
}
pub(super) fn array<const N: usize>(value: &Cbor) -> Result<&[Cbor; N], Error> {
    items(value)?
        .try_into()
        .map_err(|_| Error::Invalid("row arity"))
}
pub(super) fn items(value: &Cbor) -> Result<&[Cbor], Error> {
    match value {
        Cbor::Array(items) => Ok(items),
        _ => Err(Error::Invalid("array")),
    }
}
pub(super) fn text(value: &Cbor) -> Result<&str, Error> {
    match value {
        Cbor::Text(value) => Ok(value),
        _ => Err(Error::Invalid("text")),
    }
}
pub(super) fn bytes(value: &Cbor) -> Result<&[u8], Error> {
    match value {
        Cbor::Bytes(value) => Ok(value),
        _ => Err(Error::Invalid("bytes")),
    }
}
pub(super) fn uint(value: &Cbor) -> Result<u64, Error> {
    match value {
        Cbor::Uint(value) => Ok(*value),
        _ => Err(Error::Invalid("unsigned integer")),
    }
}
pub(super) fn signed(value: &Cbor) -> Result<i64, Error> {
    match value {
        Cbor::Uint(value) => i64::try_from(*value).map_err(|_| Error::Invalid("integer range")),
        Cbor::Negative(value) => i64::try_from(*value)
            .map(|value| -1 - value)
            .map_err(|_| Error::Invalid("integer range")),
        _ => Err(Error::Invalid("integer")),
    }
}
