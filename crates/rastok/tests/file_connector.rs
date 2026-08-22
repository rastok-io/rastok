//! Real filesystem evidence for the one-file outbound Driver.
//!
//! Filesystem effects belong at this boundary, not in the deterministic core.
#![allow(clippy::disallowed_types, clippy::disallowed_methods)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use rastok::boundary::{
    Dispatcher, Draft, InboundEnvelopeProvenance, InboundSubmission, OpenGate, SubmissionSequence,
};
use rastok::compiler::compile;
use rastok::connectors::{DriverRegistry, FileFactory, RuntimeBindings};
use rastok::contract::ContractCatalog;
use rastok::envelope::{Data, Namespace, Value};
use rastok::journal::{Journal, MemJournal, Record};
use rastok::node;
use rastok::plan::GraphVersion;
use rastok::runtime::vm::Vm;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
struct TestRoot(PathBuf);

impl TestRoot {
    fn new(label: &str) -> Self {
        let nonce = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "rastok-file-{label}-{}-{nonce}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("the test root is writable");
        Self(root)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn source(mode: &str, path: &str, max_read: Option<&str>) -> String {
    let input = if mode == "replace" {
        "bytes"
    } else {
        "variant"
    };
    let max_read = max_read
        .map(|limit| format!("\n    max_read = \"{limit}\""))
        .unwrap_or_default();
    format!(
        r#"
graph file_probe : App {{
  input in: {input}

  connector file : File {{
    mode = {mode}
    path = "{path}"{max_read}
  }}

  . -> file
}}
"#
    )
}

fn submission(data: Data) -> InboundSubmission {
    InboundSubmission {
        provenance: InboundEnvelopeProvenance::single(
            node::Path::new(["test"]).expect("a valid test provenance"),
        ),
        port: node::Port::new("in").expect("the conventional port"),
        draft: Draft {
            data,
            ext: Namespace::new(),
        },
        sequence: SubmissionSequence::Next,
    }
}

fn compile_file(source: &str) -> rastok::plan::ValidatedExecutionPlan {
    compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("the File probe compiles")
    .plan
}

fn registry(root: &Path) -> DriverRegistry {
    DriverRegistry::default()
        .try_with(Box::new(
            FileFactory::try_new(root).expect("the built-in File id is canonical"),
        ))
        .expect("File is registered once")
}

fn drive(vm: &mut Vm<'_>, journal: &mut MemJournal, bindings: &mut RuntimeBindings<'_>) {
    for _ in 0..2_000 {
        vm.drain_with_bindings(journal, bindings)
            .expect("ready File work commits");
        vm.pump(journal, bindings).expect("the File boundary pumps");
        if vm.is_stable() {
            return;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    panic!("the File invocation did not become stable");
}

fn root_exit_cases(vm: &Vm<'_>) -> Vec<String> {
    vm.observations()
        .iter()
        .filter_map(|observed| {
            if observed.node.is_some() || observed.port.name() != "exit" {
                return None;
            }
            let Data::Materialized(Value::TaggedUnion(tagged)) = observed.envelope.data() else {
                return None;
            };
            Some(tagged.case.clone())
        })
        .collect()
}

fn root_bytes(vm: &Vm<'_>) -> Vec<Vec<u8>> {
    vm.observations()
        .iter()
        .filter_map(|observed| {
            if observed.node.is_some() || observed.port.name() != "out" {
                return None;
            }
            match observed.envelope.data() {
                Data::Encoded { bytes, .. } | Data::Materialized(Value::Bytes(bytes)) => {
                    Some(bytes.clone())
                }
                _ => None,
            }
        })
        .collect()
}

fn run_observed(root: &Path, source: &str, data: Data) -> (MemJournal, Vec<String>, Vec<Vec<u8>>) {
    let plan = compile_file(source);
    let registry = registry(root);
    let mut bindings = RuntimeBindings::try_new(&plan, &registry).expect("File binds");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&plan, GraphVersion(1));
    vm.inject(&mut journal, submission(data))
        .expect("the input is admitted");
    drive(&mut vm, &mut journal, &mut bindings);
    let exits = root_exit_cases(&vm);
    let bytes = root_bytes(&vm);
    (journal, exits, bytes)
}

#[test]
fn read_publishes_one_complete_bounded_value_then_completed() {
    let root = TestRoot::new("read");
    std::fs::write(root.path().join("artifact.bin"), [0, 1, 2, 255]).expect("the fixture writes");

    let (_, exits, outputs) = run_observed(
        root.path(),
        &source("read", "artifact.bin", Some("4B")),
        Data::materialized(Value::Null),
    );

    assert_eq!(outputs, [vec![0, 1, 2, 255]]);
    assert_eq!(exits, ["Completed"]);
}

#[test]
fn an_invocation_path_selects_the_real_file_before_open() {
    let root = TestRoot::new("dynamic-path");
    std::fs::create_dir(root.path().join("objects")).expect("the fixture directory exists");
    std::fs::write(root.path().join("objects/report.bin"), b"selected")
        .expect("the fixture writes");
    let source = r#"
graph dynamic_file : App {
  input in: text

  connector file : File {
    mode = read
    path = data as text
  }

  . -> file
}
"#;

    let (_, exits, outputs) = run_observed(
        root.path(),
        source,
        Data::materialized(Value::Text(String::from("objects/report.bin"))),
    );

    assert_eq!(outputs, [b"selected".to_vec()]);
    assert_eq!(exits, ["Completed"]);
}

#[test]
fn read_refusals_never_publish_partial_bytes() {
    let root = TestRoot::new("read-refusals");
    std::fs::write(root.path().join("large.bin"), b"too large").expect("the fixture writes");
    std::fs::create_dir(root.path().join("directory")).expect("the fixture directory exists");

    for (path, expected) in [
        ("missing.bin", "NotFound"),
        ("directory", "NotAFile"),
        ("large.bin", "ReadLimitExceeded"),
    ] {
        let (_, exits, outputs) = run_observed(
            root.path(),
            &source("read", path, Some("4B")),
            Data::materialized(Value::Null),
        );
        assert!(outputs.is_empty(), "{path} must not leak partial bytes");
        assert_eq!(exits, [expected]);
    }
}

#[test]
#[cfg(unix)]
fn replace_atomically_swaps_the_file_and_emits_no_data_output() {
    let root = TestRoot::new("replace");
    let target = root.path().join("artifact.bin");
    std::fs::write(&target, b"before").expect("the fixture writes");

    let (_, exits, outputs) = run_observed(
        root.path(),
        &source("replace", "artifact.bin", None),
        Data::materialized(Value::Bytes(b"after".to_vec())),
    );

    assert_eq!(std::fs::read(target).expect("the result reads"), b"after");
    assert!(outputs.is_empty(), "replace success has no `out`");
    assert_eq!(exits, ["Completed"]);
    assert!(
        std::fs::read_dir(root.path())
            .expect("the root lists")
            .all(|entry| !entry
                .expect("a directory entry")
                .file_name()
                .to_string_lossy()
                .starts_with(".rastok-")),
        "the same-directory staging file is reclaimed"
    );
}

#[test]
fn replace_may_create_the_final_file_without_creating_its_parents() {
    let root = TestRoot::new("replace-create");
    let (_, exits, outputs) = run_observed(
        root.path(),
        &source("replace", "created.bin", None),
        Data::materialized(Value::Bytes(b"created".to_vec())),
    );

    assert_eq!(
        std::fs::read(root.path().join("created.bin")).expect("the created file reads"),
        b"created"
    );
    assert!(outputs.is_empty());
    assert_eq!(exits, ["Completed"]);
}

#[test]
fn an_invalid_logical_path_refuses_before_an_outbound_open() {
    let root = TestRoot::new("invalid");
    let (journal, exits, outputs) = run_observed(
        root.path(),
        &source("read", "../outside", None),
        Data::materialized(Value::Null),
    );

    assert_eq!(exits, ["InvalidPath"]);
    assert!(outputs.is_empty());
    assert!(
        journal
            .records()
            .iter()
            .all(|record| !matches!(record, Record::Outbound(_))),
        "pure preparation refusal opens no external obligation"
    );
}

#[test]
fn filesystem_observation_begins_only_after_the_outbound_open() {
    let root = TestRoot::new("journal-before-file");
    let target = root.path().join("late.bin");
    let plan = compile_file(&source("read", "late.bin", None));
    let registry = registry(root.path());
    let mut bindings = RuntimeBindings::try_new(&plan, &registry).expect("File binds");
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&plan, GraphVersion(1));
    vm.inject(&mut journal, submission(Data::materialized(Value::Null)))
        .expect("the trigger is admitted");

    vm.drain_with_bindings(&mut journal, &mut bindings)
        .expect("request preparation and OUTBOUND open succeed");
    assert_eq!(journal.open_calls().len(), 1);
    assert!(
        !target.exists(),
        "preparation did not create or inspect a result"
    );
    std::fs::write(&target, b"visible-after-open").expect("the external file appears");

    drive(&mut vm, &mut journal, &mut bindings);
    assert_eq!(root_bytes(&vm), [b"visible-after-open".to_vec()]);
    assert_eq!(root_exit_cases(&vm), ["Completed"]);
}

#[test]
fn replay_of_an_open_replace_never_reexecutes_the_file_effect() {
    let root = TestRoot::new("replay");
    let target = root.path().join("artifact.bin");
    std::fs::write(&target, b"before-open").expect("the fixture writes");
    let plan = compile_file(&source("replace", "artifact.bin", None));
    let registry = registry(root.path());
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&plan, GraphVersion(1));
    vm.inject(
        &mut journal,
        submission(Data::materialized(Value::Bytes(b"sealed".to_vec()))),
    )
    .expect("the input is admitted");
    {
        let mut bindings = RuntimeBindings::try_new(&plan, &registry).expect("File binds");
        vm.drain_with_bindings(&mut journal, &mut bindings)
            .expect("the call opens without starting the Driver");
    }
    assert_eq!(journal.open_calls().len(), 1);
    std::fs::write(&target, b"changed-after-open").expect("the external change writes");

    let mut resumed = Dispatcher::resume(&journal);
    let mut bindings = RuntimeBindings::try_new(&plan, &registry).expect("File rebinds");
    for _ in 0..3 {
        let batch = resumed
            .pump(&mut journal, &mut bindings, &OpenGate)
            .expect("resume is a pure journal fold");
        assert!(batch.events.is_empty());
        std::thread::sleep(Duration::from_millis(5));
    }

    assert_eq!(
        std::fs::read(target).expect("the external version remains"),
        b"changed-after-open"
    );
    assert_eq!(
        journal.open_calls().len(),
        1,
        "recovery triage owns re-call"
    );
}
