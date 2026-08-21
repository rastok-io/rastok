//! Identity machinery — the registries of `identity.md`, applied.

pub mod encoding;

use crate::canonical_cbor::{Value as Cbor, encode};
use crate::datatype::DataType;
use crate::plan::OwnerPath;
use crate::value::{Value, ValueError};
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec;
use core::cmp::Ordering;
use core::fmt;
use core::hash::{Hash, Hasher};
use core::num::NonZeroU32;

/// One dynamic component of a logical instance address.
///
/// Frames are semantic identity. Their order is fixed outermost-to-innermost;
/// when an owner contributes both kinds, its selected key precedes its
/// processing-time box.
#[derive(Debug, Clone, PartialEq)]
pub enum InstanceFrame {
    /// A key selected from the envelope entering this owner.
    SelectedKey {
        /// The owner which selected the key.
        owner: OwnerPath,
        /// The resolved type required to interpret the materialized value.
        ty: DataType,
        /// The selected canonical value.
        value: Value,
    },
    /// One epoch-aligned processing-time box.
    ProcessingTimeBox {
        /// The owner whose lifetime selected the box.
        owner: OwnerPath,
        /// Epoch-aligned box start in signed nanoseconds.
        start_ns: i64,
    },
}

impl InstanceFrame {
    fn owner(&self) -> &OwnerPath {
        match self {
            Self::SelectedKey { owner, .. } | Self::ProcessingTimeBox { owner, .. } => owner,
        }
    }

    const fn kind(&self) -> u64 {
        match self {
            Self::SelectedKey { .. } => 1,
            Self::ProcessingTimeBox { .. } => 2,
        }
    }

    fn tree(&self) -> Cbor {
        match self {
            Self::SelectedKey { owner, ty, value } => Cbor::Array(vec![
                Cbor::Text(owner.to_string()),
                Cbor::Uint(1),
                Cbor::Array(vec![
                    crate::plan::type_tree(ty),
                    encoding::value_tagged(value),
                ]),
            ]),
            Self::ProcessingTimeBox { owner, start_ns } => Cbor::Array(vec![
                Cbor::Text(owner.to_string()),
                Cbor::Uint(2),
                signed(*start_ns),
            ]),
        }
    }
}

/// Why a logical instance address cannot be sealed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstanceAddressError {
    /// A selected value itself is malformed.
    InvalidValue(ValueError),
    /// The declared key type is outside the closed instance-key domain.
    InvalidKeyType(DataType),
    /// The selected value does not inhabit its resolved key type.
    KeyTypeMismatch(String),
    /// A frame is not owned by the target owner or one of its ancestors.
    ForeignFrame {
        /// The target owner.
        owner: OwnerPath,
        /// The frame owner.
        frame: OwnerPath,
    },
    /// Frames are not ordered outermost-to-innermost, or two frame kinds of
    /// one owner are reversed.
    NonCanonicalFrames,
}

impl fmt::Display for InstanceAddressError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidValue(error) => error.fmt(f),
            Self::InvalidKeyType(ty) => {
                write!(f, "`{ty}` is outside the canonical instance-key domain")
            }
            Self::KeyTypeMismatch(reason) => f.write_str(reason),
            Self::ForeignFrame { owner, frame } => write!(
                f,
                "instance owner `{owner}` cannot contain a frame owned by `{frame}`"
            ),
            Self::NonCanonicalFrames => {
                f.write_str("instance address frames are not in canonical owner/kind order")
            }
        }
    }
}

impl core::error::Error for InstanceAddressError {}

/// Canonical logical address of one owner instance.
///
/// The byte carrier is deliberately opaque to routing consumers. It is the
/// exact canonical-CBOR form specified by `identity.md`; comparing or hashing
/// it never depends on a Rust enum order or a host word size.
#[derive(Clone)]
pub struct InstanceAddress {
    owner: Arc<OwnerPath>,
    bytes: Arc<[u8]>,
}

impl PartialEq for InstanceAddress {
    fn eq(&self, other: &Self) -> bool {
        self.bytes == other.bytes
    }
}

impl Eq for InstanceAddress {}

impl PartialOrd for InstanceAddress {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for InstanceAddress {
    fn cmp(&self, other: &Self) -> Ordering {
        self.bytes.cmp(&other.bytes)
    }
}

impl Hash for InstanceAddress {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.bytes.hash(state);
    }
}

impl fmt::Debug for InstanceAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InstanceAddress")
            .field("owner", &self.owner)
            .field("bytes", &self.bytes)
            .finish()
    }
}

impl InstanceAddress {
    fn seal(owner: &OwnerPath, frames: &[InstanceFrame]) -> Self {
        let tree = Cbor::Array(vec![
            Cbor::Text(owner.to_string()),
            Cbor::Array(frames.iter().map(InstanceFrame::tree).collect()),
        ]);
        Self {
            owner: Arc::new(owner.clone()),
            bytes: Arc::from(encode(&tree)),
        }
    }

    /// Seals an owner path and its dynamic frames.
    ///
    /// # Errors
    ///
    /// Refuses a malformed key, a key outside the closed address domain, or
    /// frames which are not an ancestor-ordered canonical chain.
    pub fn new(owner: &OwnerPath, frames: &[InstanceFrame]) -> Result<Self, InstanceAddressError> {
        for frame in frames {
            if !owner.segments().starts_with(frame.owner().segments()) {
                return Err(InstanceAddressError::ForeignFrame {
                    owner: owner.clone(),
                    frame: frame.owner().clone(),
                });
            }
            if let InstanceFrame::SelectedKey { ty, value, .. } = frame {
                if !ty.is_instance_key() {
                    return Err(InstanceAddressError::InvalidKeyType(ty.clone()));
                }
                value
                    .validate()
                    .map_err(InstanceAddressError::InvalidValue)?;
                if let Some(violation) = ty.violation("instance.key", value) {
                    return Err(InstanceAddressError::KeyTypeMismatch(violation.to_string()));
                }
            }
        }
        for pair in frames.windows(2) {
            let left = pair[0].owner();
            let right = pair[1].owner();
            let same_owner = left == right;
            let nested = right.segments().starts_with(left.segments())
                && right.segments().len() > left.segments().len();
            if (!same_owner && !nested) || (same_owner && pair[0].kind() >= pair[1].kind()) {
                return Err(InstanceAddressError::NonCanonicalFrames);
            }
        }
        Ok(Self::seal(owner, frames))
    }

    /// The persistent singleton address of one owner.
    #[must_use]
    pub fn singleton(owner: &OwnerPath) -> Self {
        // With no dynamic frame there is no key or ordering invariant to
        // reject. Keeping this path infallible also keeps the ubiquitous
        // singleton from acquiring a fake runtime failure mode.
        Self::seal(owner, &[])
    }

    /// The exact canonical bytes sealed by causal records and hashed for
    /// placement.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        self.bytes.as_ref()
    }

    /// Static owner sealed into these bytes. Kept crate-private because
    /// routing treats the address as opaque; the journal uses it only to
    /// enforce the structural parent field of lifecycle records.
    pub(crate) fn owner(&self) -> &OwnerPath {
        self.owner.as_ref()
    }

    /// Maps this logical address onto a stable virtual shard.
    ///
    /// The first eight BLAKE3 bytes are interpreted big-endian before the
    /// modulus. This definition is architecture-neutral; Worker count never
    /// participates.
    #[must_use]
    pub fn virtual_shard(&self, count: NonZeroU32) -> u32 {
        let digest = blake3::hash(self.bytes.as_ref());
        let mut prefix = [0_u8; 8];
        prefix.copy_from_slice(&digest.as_bytes()[..8]);
        let shard = u64::from_be_bytes(prefix) % u64::from(count.get());
        // The modulus is a non-zero u32 widened to u64, so the result is
        // mathematically within u32 without a host-width assumption.
        shard as u32
    }
}

/// Complete reference to one live generation of a logical address.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InstanceRef {
    address: InstanceAddress,
    generation: u64,
}

impl InstanceRef {
    /// Couples an address with its journal-derived generation.
    #[must_use]
    pub const fn new(address: InstanceAddress, generation: u64) -> Self {
        Self {
            address,
            generation,
        }
    }

    /// The stable logical address; generation is deliberately excluded.
    #[must_use]
    pub const fn address(&self) -> &InstanceAddress {
        &self.address
    }

    /// Zero-based ordinal of `l:start` for this address.
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    /// The fixed causal-record sub-form `[instance_address, generation]`.
    #[must_use]
    pub fn tree(&self) -> Cbor {
        Cbor::Array(vec![
            Cbor::Bytes(self.address.bytes.to_vec()),
            Cbor::Uint(self.generation),
        ])
    }
}

fn signed(value: i64) -> Cbor {
    if value >= 0 {
        Cbor::Uint(value as u64)
    } else {
        Cbor::Negative(value.unsigned_abs() - 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::datatype::{DataType, DateUnit, EnumType, StructType, TemporalType, TypeField};

    #[test]
    fn instance_addresses_seal_type_value_and_owner_order() {
        let root = OwnerPath::root();
        let child = OwnerPath::new(["session"]).expect("valid owner");
        let key_type = DataType::Struct(
            StructType::new(vec![
                TypeField::new("tenant", DataType::Text),
                TypeField::new("year", DataType::INT),
            ])
            .expect("valid key type"),
        );
        let key = Value::Struct(vec![
            (String::from("year"), Value::Int(2026)),
            (String::from("tenant"), Value::Text(String::from("acme"))),
        ]);
        let frames = vec![
            InstanceFrame::SelectedKey {
                owner: root,
                ty: DataType::Text,
                value: Value::Text(String::from("eu")),
            },
            InstanceFrame::SelectedKey {
                owner: child.clone(),
                ty: key_type,
                value: key,
            },
        ];
        let address = InstanceAddress::new(&child, &frames).expect("canonical address");
        assert_eq!(
            address.as_bytes(),
            InstanceAddress::new(&child, &frames)
                .expect("same address")
                .as_bytes()
        );
        assert_eq!(address.virtual_shard(NonZeroU32::new(257).unwrap()), 212);
        assert_eq!(InstanceRef::new(address, 7).generation(), 7);
    }

    #[test]
    fn one_selected_text_key_has_architecture_neutral_address_bytes() {
        let owner = OwnerPath::new(["node"]).expect("valid owner");
        let address = InstanceAddress::new(
            &owner,
            &[InstanceFrame::SelectedKey {
                owner: owner.clone(),
                ty: DataType::Text,
                value: Value::Text(String::from("alice")),
            }],
        )
        .expect("the text key is canonical");

        assert_eq!(
            address.as_bytes(),
            &[
                0x82, 0x64, b'n', b'o', b'd', b'e', 0x81, 0x83, 0x64, b'n', b'o', b'd', b'e', 0x01,
                0x82, 0x07, 0x82, 0x08, 0x65, b'a', b'l', b'i', b'c', b'e',
            ]
        );
    }

    #[test]
    fn composite_key_bytes_cover_binary_temporal_enum_and_nested_struct_leaves() {
        let owner = OwnerPath::new(["keyed"]).expect("valid owner");
        let status = DataType::Enum(
            EnumType::new(vec![String::from("active"), String::from("idle")]).expect("valid enum"),
        );
        let inner = DataType::Struct(
            StructType::new(vec![
                TypeField::new("at", DataType::Temporal(TemporalType::Date(DateUnit::Day))),
                TypeField::new("raw", DataType::BYTES),
                TypeField::new("status", status),
            ])
            .expect("valid nested key type"),
        );
        let ty = DataType::Struct(
            StructType::new(vec![
                TypeField::new("tenant", DataType::Text),
                TypeField::new("token", inner),
            ])
            .expect("valid composite key type"),
        );
        let token = Value::Struct(vec![
            (String::from("status"), Value::Enum(String::from("active"))),
            (String::from("raw"), Value::Bytes(vec![0, 1, 255])),
            (String::from("at"), Value::Date(20_326)),
        ]);
        let left = Value::Struct(vec![
            (String::from("token"), token.clone()),
            (String::from("tenant"), Value::Text(String::from("acme"))),
        ]);
        let right = Value::Struct(vec![
            (String::from("tenant"), Value::Text(String::from("acme"))),
            (String::from("token"), token),
        ]);
        let address = |value| {
            InstanceAddress::new(
                &owner,
                &[InstanceFrame::SelectedKey {
                    owner: owner.clone(),
                    ty: ty.clone(),
                    value,
                }],
            )
            .expect("the closed composite belongs to the instance-key domain")
        };

        let left = address(left);
        let right = address(right);
        assert_eq!(left, right, "source field order is not address identity");
        assert_eq!(
            left.virtual_shard(NonZeroU32::new(4093).expect("non-zero")),
            right.virtual_shard(NonZeroU32::new(4093).expect("non-zero"))
        );
    }

    #[test]
    fn frame_order_and_key_conformance_are_constructor_invariants() {
        let owner = OwnerPath::new(["session"]).expect("valid owner");
        let reversed = vec![
            InstanceFrame::ProcessingTimeBox {
                owner: owner.clone(),
                start_ns: 0,
            },
            InstanceFrame::SelectedKey {
                owner: owner.clone(),
                ty: DataType::Text,
                value: Value::Text(String::from("alice")),
            },
        ];
        assert_eq!(
            InstanceAddress::new(&owner, &reversed),
            Err(InstanceAddressError::NonCanonicalFrames)
        );
        let wrong = vec![InstanceFrame::SelectedKey {
            owner: owner.clone(),
            ty: DataType::INT,
            value: Value::Text(String::from("alice")),
        }];
        assert!(matches!(
            InstanceAddress::new(&owner, &wrong),
            Err(InstanceAddressError::KeyTypeMismatch(_))
        ));
    }
}
