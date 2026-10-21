//! Tagged frontend declarations and their compiler readers.
//!
//! Frontend framing belongs to the source language, independently of Connector Drivers.

use crate::connector::contract::{ContractCatalog, ContractValidationError, expressible_name};

/// How a tagged frontend owns the bytes after `IDENT!`.
///
/// This is compiler-layer framing policy, not host grammar.  The host chooses
/// the frontend production from the adjacent bang alone; only then does the
/// registry select one of these readers.  Adding a real guest compiler may add
/// another registered framing implementation without teaching the parser a
/// language name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrontendFraming {
    /// One self-delimited JSON value, read by the canonical house reader.
    JsonValue,
    /// A braced block where every byte except balanced braces is verbatim.
    VerbatimBraced,
    /// A braced guest region whose registered contract treats quoted strings
    /// as opaque while balancing braces.
    QuotedBraced,
}

/// The implementation attached to one tagged frontend contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrontendReader {
    /// Canonical JSON value reader; result type is `variant`.
    Json,
    /// Verbatim UTF-8 text reader; result type is `text`.
    Text,
    /// RFC 4648 decoder over a verbatim block; result type is `bytes`.
    Base64,
    /// The name and framing are known, but no compiler/reader ships in this
    /// build.  The checker emits the named deferral after parsing the region.
    Deferred {
        /// The framing contract this registered frontend owns.
        framing: FrontendFraming,
        /// Stable construct name used by the ordinary deferred diagnostic.
        construct: &'static str,
    },
}

impl FrontendReader {
    /// The framing implementation selected after the host has recognized
    /// `IDENT!`.
    #[must_use]
    pub const fn framing(self) -> FrontendFraming {
        match self {
            Self::Json => FrontendFraming::JsonValue,
            Self::Text | Self::Base64 => FrontendFraming::VerbatimBraced,
            Self::Deferred { framing, .. } => framing,
        }
    }

    /// The logical result type when this build can execute the frontend.
    #[must_use]
    pub const fn result_type(self) -> Option<crate::datatype::DataType> {
        match self {
            Self::Json => Some(crate::datatype::DataType::Variant),
            Self::Text => Some(crate::datatype::DataType::Text),
            Self::Base64 => Some(crate::datatype::DataType::BYTES),
            Self::Deferred { .. } => None,
        }
    }
}

/// One tagged frontend known to a compiler build.
///
/// The name is deliberately ordinary: it is never a parser keyword.  The
/// contract is consulted only after the adjacent `!` has selected the grammar
/// production.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrontendContract {
    /// Source name before `!` (`json`, `jq`, …).
    pub name: &'static str,
    /// Reader/compiler and, through it, the framing and output type rules.
    pub reader: FrontendReader,
}

impl ContractCatalog {
    /// Extends the registry with a tagged frontend contract.  A repeated name
    /// intentionally replaces the previous contract, matching connector
    /// extension and making application registries composable.
    ///
    /// # Errors
    ///
    /// Returns [`ContractValidationError::InexpressibleName`] when the frontend name cannot
    /// be written as the `IDENT` before `!`.
    pub fn try_with_frontend(
        mut self,
        frontend: FrontendContract,
    ) -> Result<Self, ContractValidationError> {
        if !expressible_name(frontend.name) {
            return Err(ContractValidationError::InexpressibleName(frontend.name));
        }
        self.frontends
            .retain(|existing| existing.name != frontend.name);
        self.frontends.push(frontend);
        Ok(self)
    }

    /// The tagged frontend contract for a source name, if registered.
    #[must_use]
    pub fn frontend(&self, name: &str) -> Option<&FrontendContract> {
        self.frontends.iter().find(|frontend| frontend.name == name)
    }
}
