#![allow(clippy::disallowed_methods, clippy::disallowed_types)]

use super::{Error, Recording};
use crate::ui::{
    self, ActivityView, ExecutionState, FaultView, SnapshotInput, UiCommand, UiServer, UiSnapshot,
};

const MAX_UI_RECORDS: usize = 1000;
const MAX_UI_HISTORY_BYTES: usize = 64 * 1024 * 1024;

fn snapshots(recording: &Recording) -> Result<Vec<UiSnapshot>, Error> {
    if recording.records().len() > MAX_UI_RECORDS {
        return Err(Error::Unsupported(
            "the recording UI retains at most 1000 records; CLI replay remains available",
        ));
    }
    let incomplete = (!recording.finished()).then(|| FaultView {
        message: String::from(
            "The recording has no completion footer. Only its retained prefix is available.",
        ),
        help: String::from("Inspection cannot resume effects or claim that this run completed."),
        commit: String::from("all displayed records are retained facts"),
        retryable: false,
    });
    let mut views = Vec::new();
    let mut bytes = 0_usize;
    for through in 0..=recording.records().len() {
        let (journal, vm) = recording.inspect(through)?;
        let activity = through.checked_sub(1).and_then(|before| {
            ActivityView::journal_change(&journal, before, Some(through as u64))
        });
        let snapshot = ui::snapshot(SnapshotInput {
            revision: through as u64,
            mode: "replay",
            state: if recording.finished() {
                ExecutionState::Completed
            } else {
                ExecutionState::Faulted
            },
            plan: recording.plan(),
            definitions: recording.definitions(),
            execution: &vm,
            journal: &journal,
            fault: incomplete.as_ref(),
            activity: activity.as_ref(),
            scenario: None,
            session: 0,
            boundary: through as u64,
        })
        .map_err(|error| Error::Ui(error.to_string()))?;
        bytes = bytes.saturating_add(snapshot.encoded_len());
        if bytes > MAX_UI_HISTORY_BYTES {
            return Err(Error::Unsupported(
                "recording UI history exceeds 64 MiB; CLI replay remains available",
            ));
        }
        views.push(snapshot);
    }
    Ok(views)
}

/// Opens the existing debugger over retained, immutable prefixes. The only
/// server command accepted is Close; temporal navigation is presentation only.
///
/// # Errors
/// Invalid prefix, bounded view size, projection or loopback-server failure.
pub fn serve_ui(recording: &Recording, port: Option<u16>) -> Result<(), Error> {
    let mut views = snapshots(recording)?.into_iter();
    let initial = views
        .next()
        .ok_or(Error::Invalid("missing initial replay view"))?;
    let mut server =
        UiServer::start(port, initial).map_err(|error| Error::Ui(error.to_string()))?;
    for view in views {
        server.publish(view);
    }
    server.flush();
    println!("ui: {}", server.url());
    loop {
        if matches!(server.try_command(), Some(UiCommand::Close)) {
            return Ok(());
        }
        server.flush();
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn recorded_snapshots_offer_no_execution_or_scenario_command() {
        let compilation = crate::compiler::compile(
            &[include_str!("../../../../examples/echo.rkg")],
            &crate::contract::ContractCatalog::try_m0().unwrap(),
        )
        .unwrap();
        let run = crate::lab::run(
            &compilation.plan,
            crate::plan::GraphVersion(1),
            &compilation.labs[0],
        )
        .remove(0);
        let journal = crate::journal::MemJournal::from_records(
            crate::plan::GraphVersion(1),
            None,
            &run.records,
        )
        .unwrap();
        let recording = Recording::capture(&compilation, &journal, 10_000_000_000).unwrap();
        let views = snapshots(&recording).unwrap();
        assert_eq!(views.len(), run.records.len() + 1);
        // The same closed command set is checked by the generic snapshot tests.
        assert_eq!(views.first().unwrap().revision(), 0);
        assert_eq!(views.last().unwrap().revision(), run.records.len() as u64);
    }
}
