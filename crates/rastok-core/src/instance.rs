//! One instance: declared policy, unique identity and live generation.
//!
//! - [`InstanceSpec`] declares the key, [`Lifetime`] and bounded [`Closing`] policy.
//! - [`InstanceAddress`] seals the static scope and dynamic [`Segment`] chain;
//!   [`Generation`] fences successive lifetimes at that same identity.
//! - [`crate::runtime::State`] holds private `var` cells per generation.
//!   Its plan-checked accessors remain in the runtime, not in this module.
//! - `LiveInstance` is the crate-private runtime state reconstructed by the Execution.
//! - [`crate::journal::instance_stop`] records exact-generation closure.
//!
//! ```text
//! app / sessions                       static scope path (the Node that declared `instance = { … }`)
//!   [sessions: key = "alice"]          segment: the selected key
//!   [sessions: box = 2026-09-24T10:00] segment: the processing-time box, tumbling lifetime only
//!   [dialogue: key = 42]               segment: a nested keyed Node
//! ```
//!
//! The journal, not this live bundle, is the authority for generations and
//! lifecycle. The governing rules are [instances.md] and [identity.md].
//!
//! [instances.md]: https://github.com/rastok-io/rastok/blob/main/docs/spec/instances.md
//! [identity.md]: https://github.com/rastok-io/rastok/blob/main/docs/spec/identity.md

use crate::canonical;
use crate::canonical_cbor::{Value as Cbor, encode};
use crate::datatype::DataType;
use crate::envelope::{self, Envelope};
use crate::expr;
use crate::node;
use crate::value::{Value, ValueError};
use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use core::cmp::Ordering;
use core::fmt;
use core::hash::{Hash, Hasher};
use core::num::NonZeroU32;

/// One dynamic component of a logical instance address.
///
/// Segments are semantic identity. Their order is fixed outermost-to-innermost;
/// when an owner contributes both kinds, its selected key precedes its
/// processing-time box.
#[derive(Debug, Clone, PartialEq)]
pub enum Segment {
    /// A key selected from the envelope entering this owner.
    SelectedKey {
        /// The owner which selected the key.
        scope: node::OwnerPath,
        /// The resolved type required to interpret the materialized value.
        ty: DataType,
        /// The selected canonical value.
        value: Value,
    },
    /// One epoch-aligned processing-time box.
    ProcessingTimeBox {
        /// The owner whose lifetime selected the box.
        scope: node::OwnerPath,
        /// Civil box start: the Unix epoch plus an integral number of periods.
        start: crate::time::Timestamp,
    },
}

impl Segment {
    fn owner(&self) -> &node::OwnerPath {
        match self {
            Self::SelectedKey { scope: owner, .. }
            | Self::ProcessingTimeBox { scope: owner, .. } => owner,
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
            Self::SelectedKey {
                scope: owner,
                ty,
                value,
            } => Cbor::Array(vec![
                owner_path_tree(owner),
                Cbor::Uint(1),
                Cbor::Array(vec![canonical::type_tree(ty), canonical::value(value)]),
            ]),
            Self::ProcessingTimeBox {
                scope: owner,
                start,
            } => Cbor::Array(vec![
                owner_path_tree(owner),
                Cbor::Uint(2),
                canonical::timestamp(*start),
            ]),
        }
    }
}

/// Why a logical instance address cannot be sealed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdentityError {
    /// A selected value itself is malformed.
    InvalidValue(ValueError),
    /// The declared key type is outside the closed instance-key domain.
    InvalidKeyType(DataType),
    /// The selected value does not inhabit its resolved key type.
    KeyTypeMismatch(String),
    /// A frame is not owned by the target owner or one of its ancestors.
    ForeignFrame {
        /// The target owner.
        owner: node::OwnerPath,
        /// The frame owner.
        frame: node::OwnerPath,
    },
    /// Frames are not ordered outermost-to-innermost, or two frame kinds of
    /// one owner are reversed.
    NonCanonicalFrames,
}

impl fmt::Display for IdentityError {
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

impl core::error::Error for IdentityError {}

/// Canonical logical address of one owner instance.
///
/// ```text
/// app / sessions                       static scope path (the Node that declared `instance = { … }`)
///   [sessions: key = "alice"]          segment: the selected key
///   [sessions: box = 2026-09-24T10:00] segment: the processing-time box, tumbling lifetime only
///   [dialogue: key = 42]               segment: a nested keyed Node
/// ```
///
/// The canonical CBOR encoding specified by `identity.md` is opaque to routing
/// consumers. Equality and hashing use that encoding, independently of Rust
/// enum order or host word size.
#[derive(Clone)]
pub struct InstanceAddress {
    scope: Arc<node::OwnerPath>,
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
            .field("scope", &self.scope)
            .field("bytes", &self.bytes)
            .finish()
    }
}

impl InstanceAddress {
    fn seal(owner: &node::OwnerPath, frames: &[Segment]) -> Self {
        let tree = Cbor::Array(vec![
            owner_path_tree(owner),
            Cbor::Array(frames.iter().map(Segment::tree).collect()),
        ]);
        Self {
            scope: Arc::new(owner.clone()),
            bytes: Arc::from(encode(&tree)),
        }
    }

    /// Seals an owner path and its dynamic frames.
    ///
    /// # Errors
    ///
    /// Refuses a malformed key, a key outside the closed address domain, or
    /// frames which are not an ancestor-ordered canonical chain.
    pub fn new(owner: &node::OwnerPath, frames: &[Segment]) -> Result<Self, IdentityError> {
        for frame in frames {
            if !owner.segments().starts_with(frame.owner().segments()) {
                return Err(IdentityError::ForeignFrame {
                    owner: owner.clone(),
                    frame: frame.owner().clone(),
                });
            }
            if let Segment::SelectedKey { ty, value, .. } = frame {
                if !ty.is_instance_key() {
                    return Err(IdentityError::InvalidKeyType(ty.clone()));
                }
                value
                    .check_consistency()
                    .map_err(IdentityError::InvalidValue)?;
                if let Err(violation) = ty.check_value_type("instance.key", value) {
                    return Err(IdentityError::KeyTypeMismatch(violation.to_string()));
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
                return Err(IdentityError::NonCanonicalFrames);
            }
        }
        Ok(Self::seal(owner, frames))
    }

    /// The persistent singleton address of one owner.
    #[must_use]
    pub fn singleton(owner: &node::OwnerPath) -> Self {
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

    /// Reads the canonical address form and revalidates its typed key frames.
    ///
    /// # Errors
    /// Malformed, non-canonical or structurally invalid address bytes.
    pub fn decode(bytes: &[u8]) -> Result<Self, crate::canonical_cbor::DecodeError> {
        use crate::canonical_cbor::DecodeError;
        let malformed = || DecodeError::Malformed("instance address");
        let (tree, used) = crate::canonical_cbor::decode(bytes)?;
        if used != bytes.len() {
            return Err(malformed());
        }
        let Cbor::Array(parts) = &tree else {
            return Err(malformed());
        };
        let [owner, Cbor::Array(frames)] = parts.as_slice() else {
            return Err(malformed());
        };
        let owner_from = |value: &Cbor| -> Result<node::OwnerPath, DecodeError> {
            let Cbor::Array(segments) = value else {
                return Err(malformed());
            };
            let names = segments
                .iter()
                .map(|segment| match segment {
                    Cbor::Text(name) => Ok(name.clone()),
                    _ => Err(malformed()),
                })
                .collect::<Result<alloc::vec::Vec<_>, _>>()?;
            node::OwnerPath::new(names).map_err(|_| malformed())
        };
        let owner = owner_from(owner)?;
        let mut decoded = alloc::vec::Vec::new();
        for frame in frames {
            let Cbor::Array(parts) = frame else {
                return Err(malformed());
            };
            decoded.push(match parts.as_slice() {
                [frame_owner, Cbor::Uint(1), Cbor::Array(key)] => {
                    let [ty, value] = key.as_slice() else {
                        return Err(malformed());
                    };
                    Segment::SelectedKey {
                        scope: owner_from(frame_owner)?,
                        ty: canonical::type_from(ty).map_err(|_| malformed())?,
                        value: canonical::decode_value(value)?,
                    }
                }
                [frame_owner, Cbor::Uint(2), instant] => Segment::ProcessingTimeBox {
                    scope: owner_from(frame_owner)?,
                    start: canonical::decode_timestamp(instant)?,
                },
                _ => return Err(malformed()),
            });
        }
        let address = Self::new(&owner, &decoded).map_err(|_| malformed())?;
        if address.as_bytes() != bytes {
            return Err(DecodeError::NonCanonical("instance address"));
        }
        Ok(address)
    }

    /// Static owner sealed into these bytes. Kept crate-private because
    /// routing treats the address as opaque; the journal uses it only to
    /// enforce the structural parent field of lifecycle records.
    pub(crate) fn owner(&self) -> &node::OwnerPath {
        self.scope.as_ref()
    }

    /// Structural address containment only. A lifecycle fact must separately
    /// prove the parent generation; matching keys cannot establish it.
    pub(crate) fn contains_address(&self, descendant: &Self) -> bool {
        if !descendant
            .scope
            .segments()
            .starts_with(self.scope.segments())
        {
            return false;
        }
        let Ok((Cbor::Array(ancestor), _)) = crate::canonical_cbor::decode(&self.bytes) else {
            return false;
        };
        let Ok((Cbor::Array(descendant), _)) = crate::canonical_cbor::decode(&descendant.bytes)
        else {
            return false;
        };
        let (Some(Cbor::Array(own_frames)), Some(Cbor::Array(frames))) =
            (ancestor.get(1), descendant.get(1))
        else {
            return false;
        };
        own_frames.iter().eq(frames.iter().take_while(|frame| {
            matches!(frame, Cbor::Array(parts) if matches!(parts.first(),
                Some(Cbor::Array(owner)) if owner.len() <= self.scope.segments().len()))
        }))
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

fn owner_path_tree(owner: &node::OwnerPath) -> Cbor {
    Cbor::Array(owner.segments().iter().cloned().map(Cbor::Text).collect())
}

/// Complete reference to one live generation of a logical address.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Generation {
    address: InstanceAddress,
    generation: u64,
}

impl Generation {
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

/// Validated instance policy of one Node. A missing key selects the singleton
/// address within its parent instance. The plan records the resolved lifetime
/// explicitly, including the persistent default when no lifetime was written.
#[derive(Debug, Clone, PartialEq)]
pub struct InstanceSpec {
    /// Expression and inferred type of the selected key, when keyed.
    pub key: Option<expr::Tree>,
    /// When the selected instance closes automatically.
    pub lifetime: Lifetime,
    /// Finite drain and settlement budgets. Absence is valid only when this
    /// owner's closure can never hold an external invocation open.
    pub closing: Option<Closing>,
}

/// The one bounded Closing protocol. Actions are fixed by the runtime:
/// drain admitted work, cancel calls, settle observed exits, then abandon
/// every still-unknown outcome. Only the two explicit budgets are policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Closing {
    /// Nanoseconds available to drain already-admitted work.
    pub drain_for: i64,
    /// Nanoseconds available for cancelled calls to publish an observed exit.
    pub settle_for: i64,
}

/// Closed transport vocabulary for lifetime policies. The scheduler admits
/// each profile only when its corresponding runtime gate has landed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lifetime {
    /// No automatic expiry.
    Persistent,
    /// Expire after this many nanoseconds without activity.
    Idle(i64),
    /// Expire this many nanoseconds after `l:start`.
    Fixed(i64),
    /// Admit one root delivery, then close after its work quiesces.
    SinglePass,
    /// Epoch-aligned processing-time boxes of this many nanoseconds.
    Tumbling(i64),
}

/// Runtime state for one selected instance generation. Its instance-scoped
/// values and resolved settings are immutable; activity, residual deliveries
/// and lifecycle phase change as the Execution processes committed records.
#[derive(Debug, Clone)]
pub(crate) struct LiveInstance {
    pub(crate) owner: node::NodeIndex,
    pub(crate) parent: Option<Generation>,
    pub(crate) segments: Vec<Segment>,
    pub(crate) values: BTreeMap<String, crate::value::Value>,
    pub(crate) settings: BTreeMap<String, crate::value::Value>,
    pub(crate) created_by: Envelope,
    pub(crate) last_activity: Envelope,
    /// Deliveries discarded only after the drain budget expired. Their cursor
    /// positions are sealed by this instance's eventual `l:stop`.
    pub(crate) residual: Vec<envelope::Delivery>,
    pub(crate) phase: Phase,
}

/// Admission and closure are mutually exclusive states. The first close
/// request owns the causal envelope; later expiries are stale observations.
#[derive(Debug, Clone)]
pub(crate) enum Phase {
    Open,
    Closing {
        cause: Envelope,
        scope: Generation,
        stage: ClosingStage,
    },
    Stopping {
        cause: Envelope,
        scope: Generation,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ClosingStage {
    Draining,
    Settling,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::datatype::{DataType, DateUnit, EnumType, StructType, TemporalType, TypeField};

    #[test]
    fn retained_addresses_round_trip_and_refuse_forged_typed_keys() {
        let owner = node::OwnerPath::new(["worker"]).unwrap();
        let address = InstanceAddress::new(
            &owner,
            &[
                Segment::SelectedKey {
                    scope: owner.clone(),
                    ty: DataType::Text,
                    value: Value::Text("tenant".into()),
                },
                Segment::ProcessingTimeBox {
                    scope: owner.clone(),
                    start: crate::time::Timestamp::MIN,
                },
            ],
        )
        .unwrap();
        assert_eq!(
            InstanceAddress::decode(address.as_bytes()).unwrap(),
            address
        );
        let mut trailing = address.as_bytes().to_vec();
        trailing.push(0);
        assert!(InstanceAddress::decode(&trailing).is_err());
        let (mut tree, _) = crate::canonical_cbor::decode(address.as_bytes()).unwrap();
        let Cbor::Array(parts) = &mut tree else {
            panic!("address")
        };
        let Cbor::Array(frames) = &mut parts[1] else {
            panic!("frames")
        };
        let Cbor::Array(frame) = &mut frames[0] else {
            panic!("key frame")
        };
        let Cbor::Array(key) = &mut frame[2] else {
            panic!("typed key")
        };
        key[1] = canonical::value(&Value::Int(7));
        assert!(InstanceAddress::decode(&crate::canonical_cbor::encode(&tree)).is_err());
    }

    #[test]
    fn address_containment_preserves_every_ancestor_key_and_time_box() {
        let parent = node::OwnerPath::new(["window"]).unwrap();
        let child = node::OwnerPath::new(["window", "worker"]).unwrap();
        let frames = |key: &str, at| {
            vec![
                Segment::SelectedKey {
                    scope: parent.clone(),
                    ty: DataType::Text,
                    value: Value::Text(key.into()),
                },
                Segment::ProcessingTimeBox {
                    scope: parent.clone(),
                    start: crate::time::Timestamp::from_nanoseconds(at),
                },
            ]
        };
        let address = InstanceAddress::new(&parent, &frames("alice", 10)).unwrap();
        for (key, at, matches) in [
            ("alice", 10, true),
            ("bob", 10, false),
            ("alice", 11, false),
        ] {
            let mut frames = frames(key, at);
            frames.push(Segment::SelectedKey {
                scope: child.clone(),
                ty: DataType::INT,
                value: Value::Int(7),
            });
            let descendant = InstanceAddress::new(&child, &frames).unwrap();
            assert_eq!(address.contains_address(&descendant), matches);
            assert!(!descendant.contains_address(&address));
        }
        assert!(!address.contains_address(&InstanceAddress::singleton(&child)));
        assert!(
            !InstanceAddress::singleton(&parent)
                .contains_address(&InstanceAddress::new(&child, &frames("alice", 10)).unwrap())
        );
        assert!(!address.contains_address(&InstanceAddress::singleton(
            &node::OwnerPath::new(["other"]).unwrap()
        )));
    }

    #[test]
    fn instance_addresses_seal_type_value_and_owner_order() {
        let root = node::OwnerPath::root();
        let child = node::OwnerPath::new(["session"]).expect("valid owner");
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
            Segment::SelectedKey {
                scope: root,
                ty: DataType::Text,
                value: Value::Text(String::from("eu")),
            },
            Segment::SelectedKey {
                scope: child.clone(),
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
        assert_eq!(address.virtual_shard(NonZeroU32::new(257).unwrap()), 169);
        assert_eq!(Generation::new(address, 7).generation(), 7);
    }

    #[test]
    fn one_selected_text_key_has_architecture_neutral_address_bytes() {
        let owner = node::OwnerPath::new(["node"]).expect("valid owner");
        let address = InstanceAddress::new(
            &owner,
            &[Segment::SelectedKey {
                scope: owner.clone(),
                ty: DataType::Text,
                value: Value::Text(String::from("alice")),
            }],
        )
        .expect("the text key is canonical");

        assert_eq!(
            address.as_bytes(),
            &[
                0x82, 0x81, 0x64, b'n', b'o', b'd', b'e', 0x81, 0x83, 0x81, 0x64, b'n', b'o', b'd',
                b'e', 0x01, 0x82, 0x07, 0x82, 0x06, 0x65, b'a', b'l', b'i', b'c', b'e',
            ]
        );
    }

    #[test]
    fn owner_segments_are_structural_address_identity() {
        let owner = node::OwnerPath::new(["outer", "inner"]).expect("valid nested owner");
        let address = InstanceAddress::singleton(&owner);

        assert_eq!(
            address.as_bytes(),
            &[
                0x82, 0x82, 0x65, b'o', b'u', b't', b'e', b'r', 0x65, b'i', b'n', b'n', b'e', b'r',
                0x80,
            ]
        );
    }

    #[test]
    fn composite_key_bytes_cover_binary_temporal_enum_and_nested_struct_leaves() {
        let owner = node::OwnerPath::new(["keyed"]).expect("valid owner");
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
                &[Segment::SelectedKey {
                    scope: owner.clone(),
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
        let owner = node::OwnerPath::new(["session"]).expect("valid owner");
        let reversed = vec![
            Segment::ProcessingTimeBox {
                scope: owner.clone(),
                start: crate::time::Timestamp::UNIX_EPOCH,
            },
            Segment::SelectedKey {
                scope: owner.clone(),
                ty: DataType::Text,
                value: Value::Text(String::from("alice")),
            },
        ];
        assert_eq!(
            InstanceAddress::new(&owner, &reversed),
            Err(IdentityError::NonCanonicalFrames)
        );
        let wrong = vec![Segment::SelectedKey {
            scope: owner.clone(),
            ty: DataType::INT,
            value: Value::Text(String::from("alice")),
        }];
        assert!(matches!(
            InstanceAddress::new(&owner, &wrong),
            Err(IdentityError::KeyTypeMismatch(_))
        ));
    }
}
