//! Connector Contracts, portable exchange interfaces and hosted Driver implementations.
//!
//! Core types are exposed directly, without a second representation. Only `drivers`
//! observes the external world; Contracts remain available without a physical Driver.

pub use rastok_core::connector::{
    CallCommandError, CallSemantics, ConnectorDirection, ContractId, ContractIdError, ExchangeKind,
    FrameProduction, MAX_BYTE_SIZE, RetryCaseError, TerminalVisibility, byte_size, driver,
    exchange, outbound_call_semantics,
};

pub mod contract;
pub mod drivers;
pub mod stdlib;
