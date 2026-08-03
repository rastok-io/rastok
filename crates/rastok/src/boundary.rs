//! Contracts with the external world — the data vocabulary.
//!
//! Governing rule: [`delivery-semantics.md`]. This module carries the
//! value types the journal records reference: the world-facing traits
//! (`Ingress`, `Partner`, `Clock`, `Supervisor`) and the dispatcher
//! join at the boundary stage.
//!
//! [`delivery-semantics.md`]: https://github.com/rastok/rastok/blob/main/docs/spec/delivery-semantics.md

use crate::envelope;
use crate::node;
use std::fmt;

/// Opaque ingress identity: exactly `(connector_path, replica)`.
/// Opaque so the replica is never inlined anywhere else — the
/// multi-host door stays open. In single-process execution the
/// replica is 0.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct IngressAuthority {
    connector_path: node::Path,
    replica: u64,
}

impl IngressAuthority {
    /// The single-process authority for one ingress connector.
    #[must_use]
    pub fn single(connector_path: node::Path) -> Self {
        Self {
            connector_path,
            replica: 0,
        }
    }

    /// The ingress connector's root-relative path — root-recipe input.
    #[must_use]
    pub fn connector_path(&self) -> &node::Path {
        &self.connector_path
    }

    /// The logical replica id; 0 in single-process execution.
    #[must_use]
    pub fn replica(&self) -> u64 {
        self.replica
    }
}

/// Deterministic internal call identity — a derivation of the
/// canonical call fields, never a new serialized causal field: it is
/// always recomputable from the record that carries those fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CallRef(envelope::Id);

impl CallRef {
    /// Wraps a derived identity; the recipe lives with the journal's
    /// `ids` module.
    #[must_use]
    pub const fn new(id: envelope::Id) -> Self {
        Self(id)
    }
}

impl fmt::Display for CallRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Canonical journaled outbound-call intent (`identity.md` §3, keys
/// 40–43): journaled open before the effect executes.
#[derive(Debug, Clone, PartialEq)]
pub struct Intent {
    /// The derived internal call identity.
    pub call_ref: CallRef,
    /// The envelope that opened the call — the cause.
    pub emitting_id: envelope::Id,
    /// The calling connector node.
    pub connector_path: node::Path,
    /// The boundary integrity seal over the request bytes.
    pub request_digest: [u8; 32],
    /// Names the business operation; stable across retries. `None`
    /// only for `compensable` without a key.
    pub idempotency_key: Option<String>,
    /// The declared effect class of the call.
    pub effect: node::EffectClass,
}

/// Protocol-correlated partner answer. A partner answers or stays
/// silent — never a timeout: timeouts are the scheduler's.
#[derive(Debug, Clone, PartialEq)]
pub struct Response {
    /// The call this answers, correlated by the connector's own
    /// protocol.
    pub call_ref: CallRef,
    /// The answer payload.
    pub data: envelope::Data,
}

/// How a submission's sequence number is chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmissionSequence {
    /// Allocate the next number — the live mode.
    Next,
    /// Replay an explicit number — fixtures replay recorded seqs; a
    /// mismatch with the expected next number is refused.
    Explicit(u64),
}

/// A draft proposed for atomic acceptance: ingress births drafts,
/// never envelopes — a root envelope cannot pre-exist its sequence.
#[derive(Debug, Clone, PartialEq)]
pub struct InboundSubmission {
    /// Who proposes it.
    pub authority: IngressAuthority,
    /// The payload and birth metadata.
    pub draft: Draft,
    /// How the sequence is chosen.
    pub sequence: SubmissionSequence,
}

/// The payload and birth metadata of a submission; the runtime mints
/// `meta.sys` — a draft never carries one.
#[derive(Debug, Clone, PartialEq)]
pub struct Draft {
    /// The payload.
    pub data: envelope::Data,
    /// The content-type hint, when the transport provides one.
    pub mime_type: Option<String>,
    /// The writable `meta.app` namespace seeded at birth.
    pub app: envelope::Namespace,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_authority_stays_opaque_but_answers_the_recipes() {
        let authority = IngressAuthority::single(node::Path::new(["inbound"]).unwrap());
        assert_eq!(authority.connector_path().to_string(), "inbound");
        assert_eq!(authority.replica(), 0);
    }
}
