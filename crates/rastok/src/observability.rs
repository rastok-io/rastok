//! Hosted, disposable projections of retained runtime observations.
//!
//! The journal owns accounting truth. This module owns only bounded export
//! queues: producers use `try_send`, a slow or failed consumer increments a
//! counter, and neither condition can delay or fail execution.

use crate::data_policy::{RedactedValue, requires_redaction};
use crate::envelope::Data;
use crate::journal::Journal;
use crate::observation::{ExchangeCorrelation, RecordObservation, RecordObservationKind};
use crate::schema_annotation::AnnotationTable;
use serde_json::{Map, Value as JsonValue};
use std::fmt::Write as _;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError, TrySendError};

/// A bounded log queue cannot be constructed with zero storage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JsonLogQueueError;

impl std::fmt::Display for JsonLogQueueError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a structured-log queue needs a positive capacity")
    }
}

impl std::error::Error for JsonLogQueueError {}

/// Result of one never-block projection attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectionOutcome {
    /// The exporter queue accepted the complete JSON line.
    Queued,
    /// The queue was full, disconnected, or could not serialize the event.
    Dropped,
}

/// Counts produced and dropped observations in one projection pass.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProjectionBatch {
    /// Lines accepted by the bounded queue.
    pub queued: u64,
    /// Lines dropped without affecting execution or retained observations.
    pub dropped: u64,
}

/// Cloneable, never-block producer for structured JSON logs.
#[derive(Debug, Clone)]
pub struct JsonLogSink {
    sender: SyncSender<String>,
    dropped: Arc<AtomicU64>,
}

impl JsonLogSink {
    /// Projects one retained observation without payload content.
    pub fn try_observation(&self, observation: &RecordObservation) -> ProjectionOutcome {
        self.try_value(observation_json(observation, None))
    }

    /// Projects one observation with an explicitly requested payload view.
    ///
    /// Classification is applied before JSON construction. Encoded classified
    /// data becomes one marker because decoding it merely for presentation
    /// would create another semantic authority.
    pub fn try_observation_with_payload(
        &self,
        observation: &RecordObservation,
        data: &Data,
        annotations: &AnnotationTable,
    ) -> ProjectionOutcome {
        let payload = render_payload(data, annotations);
        self.try_value(observation_json(observation, Some(payload)))
    }

    /// Total projections lost to saturation, disconnection or formatting.
    #[must_use]
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    fn try_value(&self, value: JsonValue) -> ProjectionOutcome {
        let line = match serde_json::to_string(&value) {
            Ok(line) => line,
            Err(_) => return self.record_drop(),
        };
        match self.sender.try_send(line) {
            Ok(()) => ProjectionOutcome::Queued,
            Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => self.record_drop(),
        }
    }

    fn record_drop(&self) -> ProjectionOutcome {
        let _ = self
            .dropped
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                Some(current.saturating_add(1))
            });
        ProjectionOutcome::Dropped
    }
}

/// Consumer half intended for a hosted exporter thread.
///
/// Blocking is allowed only on this side of the queue. Producers never call
/// this API and therefore never wait for log I/O.
#[derive(Debug)]
pub struct JsonLogReceiver {
    receiver: Receiver<String>,
    dropped: Arc<AtomicU64>,
}

impl JsonLogReceiver {
    /// Waits on the exporter side for one line; `None` means all senders left.
    #[must_use]
    pub fn recv(&self) -> Option<String> {
        self.receiver.recv().ok()
    }

    /// Drains every line currently available without waiting.
    #[must_use]
    pub fn try_drain(&self) -> Vec<String> {
        let mut lines = Vec::new();
        loop {
            match self.receiver.try_recv() {
                Ok(line) => lines.push(line),
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => return lines,
            }
        }
    }

    /// Total producer-side drops observed by this queue.
    #[must_use]
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

/// Creates one bounded never-block JSON projection lane.
///
/// # Errors
///
/// A zero capacity is rejected instead of becoming a rendezvous channel,
/// whose behavior would depend on exporter scheduling.
pub fn json_log_channel(
    capacity: usize,
) -> Result<(JsonLogSink, JsonLogReceiver), JsonLogQueueError> {
    if capacity == 0 {
        return Err(JsonLogQueueError);
    }
    let (sender, receiver) = std::sync::mpsc::sync_channel(capacity);
    let dropped = Arc::new(AtomicU64::new(0));
    Ok((
        JsonLogSink {
            sender,
            dropped: Arc::clone(&dropped),
        },
        JsonLogReceiver { receiver, dropped },
    ))
}

/// Incremental projector over one retained observation stream.
#[derive(Debug)]
pub struct JournalLogProjector {
    sink: JsonLogSink,
    next: usize,
}

impl JournalLogProjector {
    /// Starts at the beginning of the supplied journal stream.
    #[must_use]
    pub const fn new(sink: JsonLogSink) -> Self {
        Self { sink, next: 0 }
    }

    /// Reconstructs and projects only the journal suffix not seen previously.
    ///
    /// The projector is bound to one append-only journal run. Keeping the
    /// offset here avoids rescanning and reformatting its whole causal history
    /// on every hosted polling round. Its cursor advances even on a drop: exporter
    /// retry is not allowed to become an unbounded second backlog.
    pub fn project_journal<J: Journal>(&mut self, journal: &J) -> ProjectionBatch {
        let observations = journal.retained_observations_from(self.next);
        let mut batch = ProjectionBatch::default();
        for observation in &observations {
            match self.sink.try_observation(observation) {
                ProjectionOutcome::Queued => batch.queued = batch.queued.saturating_add(1),
                ProjectionOutcome::Dropped => batch.dropped = batch.dropped.saturating_add(1),
            }
        }
        self.next = self.next.saturating_add(observations.len());
        batch
    }

    /// Total drops across every pass through this projector's queue.
    #[must_use]
    pub fn dropped(&self) -> u64 {
        self.sink.dropped()
    }
}

fn event_name(kind: RecordObservationKind) -> &'static str {
    match kind {
        RecordObservationKind::IngressCommitted => "rastok.ingress.committed",
        RecordObservationKind::TransactionCommitted => "rastok.transaction.committed",
        RecordObservationKind::CallOpened => "rastok.call.opened",
        RecordObservationKind::CallClosed => "rastok.call.closed",
        RecordObservationKind::CallAbandoned => "rastok.call.abandoned",
        RecordObservationKind::SessionOpening => "rastok.session.opening",
        RecordObservationKind::SessionOpened => "rastok.session.opened",
        RecordObservationKind::SessionWriteOpened => "rastok.session.write.opened",
        RecordObservationKind::SessionWriteSettled => "rastok.session.write.settled",
        RecordObservationKind::SessionWriteAbandoned => "rastok.session.write.abandoned",
    }
}

pub(crate) fn observation_json(
    observation: &RecordObservation,
    payload: Option<String>,
) -> JsonValue {
    let mut fields = Map::new();
    fields.insert(
        String::from("event"),
        JsonValue::String(String::from(event_name(observation.kind()))),
    );
    let correlation = observation.correlation();
    if let Some(origin) = correlation.origin_id() {
        fields.insert(
            String::from("origin_id"),
            JsonValue::String(origin.to_string()),
        );
    }
    if let Some(owner) = correlation.owner() {
        fields.insert(String::from("owner"), JsonValue::String(owner.to_string()));
    }
    if let Some(instance) = correlation.instance() {
        fields.insert(
            String::from("instance_address"),
            JsonValue::String(hex(instance.address().as_bytes())),
        );
        fields.insert(
            String::from("instance_generation"),
            JsonValue::from(instance.generation()),
        );
    }
    if let Some(connector) = correlation.connector() {
        fields.insert(
            String::from("connector"),
            JsonValue::String(connector.to_string()),
        );
    }
    match correlation.exchange() {
        Some(ExchangeCorrelation::OutboundCall(reference)) => {
            fields.insert(
                String::from("call_id"),
                JsonValue::String(reference.to_string()),
            );
            fields.insert(
                String::from("call_direction"),
                JsonValue::String(String::from("outbound")),
            );
        }
        Some(ExchangeCorrelation::InboundCall(reference)) => {
            fields.insert(
                String::from("call_id"),
                JsonValue::String(reference.to_string()),
            );
            fields.insert(
                String::from("call_direction"),
                JsonValue::String(String::from("inbound")),
            );
        }
        Some(ExchangeCorrelation::Session { session, write }) => {
            fields.insert(
                String::from("session_id"),
                JsonValue::String(session.to_string()),
            );
            if let Some(write) = write {
                fields.insert(
                    String::from("session_write_id"),
                    JsonValue::String(write.to_string()),
                );
            }
        }
        None => {}
    }
    if let Some(payload) = payload {
        fields.insert(String::from("payload"), JsonValue::String(payload));
    }
    JsonValue::Object(fields)
}

fn render_payload(data: &Data, annotations: &AnnotationTable) -> String {
    if annotations.is_empty() {
        return format!("{data:?}");
    }
    if requires_redaction(annotations) {
        return match data {
            Data::Materialized(value) => format!("{:?}", RedactedValue::new(value, annotations)),
            Data::Encoded { .. } => String::from("[REDACTED]"),
        };
    }
    format!("{data:?}")
}

fn hex(bytes: &[u8]) -> String {
    let mut rendered = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        let _ = write!(rendered, "{byte:02x}");
    }
    rendered
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connector::exchange::EnvelopeDraft;
    use crate::connector::exchange::IngressProposal;
    use crate::connector::exchange::IngressProvenance;
    use crate::connector::exchange::IngressSequence;
    use crate::envelope::{Data, Namespace, Value};
    use crate::journal::MemJournal;
    use crate::node;
    use crate::plan::GraphVersion;
    use crate::schema_annotation::{
        AnnotationSet, DataAnnotation, SchemaPath, SchemaPathSegment, SensitivityLevel,
    };

    fn submission(value: i64) -> IngressProposal {
        IngressProposal {
            provenance: IngressProvenance::single(node::Path::new(["cli"]).expect("valid path")),
            draft: EnvelopeDraft {
                port: node::Port::new("in").expect("valid port"),
                data: Data::materialized(Value::Int(value)),
                ext: Namespace::new(),
            },
            sequence: IngressSequence::Next,
            session: None,
        }
    }

    #[test]
    fn exchange_logs_use_id_keys_without_reference_aliases() {
        let catalogue =
            crate::connector::contract::ContractCatalog::try_m0().expect("valid contracts");
        let mut seen = [false; 4];
        for source in [
            include_str!("../../../examples/echo.rkg"),
            include_str!("../../../examples/agent-gateway.rkg"),
            include_str!("../tests/fixtures/lab-http-source.rkg"),
        ] {
            let compilation = crate::compiler::compile(&[source], &catalogue).expect("valid graph");
            for run in crate::lab::run(&compilation.plan, GraphVersion(1), &compilation.labs[0]) {
                assert!(run.report.passed(), "{:?}", run.report);
                let journal = MemJournal::from_records(GraphVersion(1), None, &run.records)
                    .expect("retained records reconstruct");
                let observations = journal.retained_observations();
                let (sink, receiver) =
                    json_log_channel(observations.len()).expect("positive capacity");
                for observation in &observations {
                    assert_eq!(sink.try_observation(observation), ProjectionOutcome::Queued);
                }
                for (observation, line) in observations.iter().zip(receiver.try_drain()) {
                    let json: JsonValue = serde_json::from_str(&line).expect("valid JSON log");
                    assert!(
                        json.as_object()
                            .expect("log object")
                            .keys()
                            .all(|key| !key.ends_with("_ref"))
                    );
                    match observation.correlation().exchange() {
                        Some(ExchangeCorrelation::OutboundCall(id)) => {
                            assert_eq!(json["call_id"], id.to_string());
                            assert_eq!(json["call_direction"], "outbound");
                            seen[0] = true;
                        }
                        Some(ExchangeCorrelation::InboundCall(id)) => {
                            assert_eq!(json["call_id"], id.to_string());
                            assert_eq!(json["call_direction"], "inbound");
                            seen[1] = true;
                        }
                        Some(ExchangeCorrelation::Session { session, write }) => {
                            assert_eq!(json["session_id"], session.to_string());
                            seen[2] = true;
                            if let Some(id) = write {
                                assert_eq!(json["session_write_id"], id.to_string());
                                seen[3] = true;
                            } else {
                                assert!(json.get("session_write_id").is_none());
                            }
                        }
                        None => {}
                    }
                }
            }
        }
        assert_eq!(
            seen, [true; 4],
            "cover Calls in both directions, Sessions and writes"
        );
    }

    #[test]
    fn saturation_and_exporter_loss_never_change_retained_observations() {
        let mut journal = MemJournal::new(GraphVersion(1));
        journal.append_ingress(submission(1)).expect("first root");
        journal.append_ingress(submission(2)).expect("second root");
        let retained = journal.retained_observations();
        let records = journal.records().to_vec();

        let (sink, receiver) = json_log_channel(1).expect("positive capacity");
        let mut projector = JournalLogProjector::new(sink);
        assert_eq!(
            projector.project_journal(&journal),
            ProjectionBatch {
                queued: 1,
                dropped: 1,
            }
        );
        assert_eq!(receiver.try_drain().len(), 1);
        assert_eq!(receiver.dropped(), 1);
        assert_eq!(journal.records(), records);
        assert_eq!(journal.retained_observations(), retained);

        drop(receiver);
        journal.append_ingress(submission(3)).expect("third root");
        assert_eq!(
            projector.project_journal(&journal),
            ProjectionBatch {
                queued: 0,
                dropped: 1,
            }
        );
        assert_eq!(projector.dropped(), 2);
        assert_eq!(journal.records().len(), 3);
        assert_eq!(journal.retained_observations().len(), 3);
    }

    #[test]
    fn payloads_are_omitted_by_default_and_redacted_before_json_formatting() {
        let mut journal = MemJournal::new(GraphVersion(1));
        journal.append_ingress(submission(1)).expect("root commits");
        let observations = journal.retained_observations();
        let observation = &observations[0];
        let annotations = AnnotationTable::new(vec![(
            SchemaPath::root().child(SchemaPathSegment::Field(String::from("prompt"))),
            AnnotationSet::new(vec![DataAnnotation::Sensitive(
                SensitivityLevel::Restricted,
            )])
            .expect("one annotation"),
        )])
        .expect("one path");
        let payload = Data::materialized(Value::Struct(vec![
            (
                String::from("prompt"),
                Value::Text(String::from("never print this")),
            ),
            (String::from("tokens"), Value::Int(7)),
        ]));
        let (sink, receiver) = json_log_channel(2).expect("positive capacity");

        assert_eq!(sink.try_observation(observation), ProjectionOutcome::Queued);
        assert_eq!(
            sink.try_observation_with_payload(observation, &payload, &annotations),
            ProjectionOutcome::Queued
        );
        let lines = receiver.try_drain();
        assert_eq!(lines.len(), 2);
        let without_payload: JsonValue =
            serde_json::from_str(&lines[0]).expect("valid structured log");
        assert!(without_payload.get("payload").is_none());
        assert_eq!(
            without_payload.get("event").and_then(JsonValue::as_str),
            Some("rastok.ingress.committed")
        );
        assert!(without_payload.get("origin_id").is_some());

        let with_payload: JsonValue =
            serde_json::from_str(&lines[1]).expect("valid structured log");
        let rendered = with_payload
            .get("payload")
            .and_then(JsonValue::as_str)
            .expect("explicit payload projection");
        assert!(rendered.contains("[REDACTED]"));
        assert!(rendered.contains("tokens"));
        assert!(!rendered.contains("never print this"));
    }
}
