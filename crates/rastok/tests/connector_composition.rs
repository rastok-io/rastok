//! Static build composition and production/mock parity for M1.5 C5.

#![allow(clippy::disallowed_types, clippy::disallowed_methods)]
#![allow(clippy::expect_used, clippy::panic)]

use rastok::compiler::compile;
use rastok::connectors::{DriverRegistry, FileFactory, ProcessFactory, RuntimeBindings};
use rastok::contract::ContractCatalog;
use rastok::journal::{OutboundRecord, OutboundStatus, Record};
use rastok::plan::GraphVersion;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

const PROCESS_ONLY: &str = r#"
graph process_only : App {
  input in: text

  connector shell : Process {
    mode = job
    command = "cat"
    codec = lines
    effect = idempotent
    behavior = deterministic
  }

  in -> shell
  shell.out -> nothing
  shell.err -> nothing
  shell.exit -> nothing
}
"#;

const FILE_ONLY: &str = r#"
graph file_only : App {
  input in: variant

  connector file : File {
    mode = read
    path = "artifact.bin"
  }

  in -> file
  file.out -> nothing
  file.exit -> nothing
}
"#;

const MIXED_GRAPH: &str = r#"
graph mixed : App {
  input in: struct { text: text }
  output process_out: text
  output file_out: bytes

  connector shell : Process {
    mode = job
    command = "cat"
    codec = lines
    effect = idempotent
    behavior = deterministic
  }

  connector file : File {
    mode = read
    path = "artifact.bin"
  }

  operator process_request {
    input in: struct { text: text }
    on in { out = data.text }
  }

  in -> process_request -> shell
  in -> file
  shell.out -> process_out
  file.out -> file_out
  shell.err -> nothing
  shell.exit -> nothing
  file.exit -> nothing
}
"#;

const MIXED_LAB: &str = r#"

lab mixed {
  simulation scripted on app {
    mock shell {
      on in {
        out = "process-mock"
        exit = Exited { exit_code = 0, signal = null }
      }
    }

    mock file {
      on in {
        out = base64! { ZmlsZS1tb2Nr }
        exit = Completed { }
      }
    }

    scenario both "one world supplies two connector families" {
      app.in = { text = "request" }
      expect "Process replied" app.process_out count == 1 where data == "process-mock"
      expect "File replied" app.file_out count == 1
    }
  }
}
"#;

fn catalogue() -> ContractCatalog {
    ContractCatalog::try_m0().expect("the standard catalogue holds")
}

fn compile_one(source: &str) -> rastok::compiler::Compilation {
    compile(&[source], &catalogue()).expect("the composition probe compiles")
}

fn process_registry() -> DriverRegistry {
    DriverRegistry::default()
        .try_with(Box::new(
            ProcessFactory::try_new().expect("the Process id is canonical"),
        ))
        .expect("Process is registered once")
}

fn file_registry() -> DriverRegistry {
    DriverRegistry::default()
        .try_with(Box::new(
            FileFactory::try_new(Path::new("unused-physical-root"))
                .expect("the File id is canonical"),
        ))
        .expect("File is registered once")
}

fn combined_registry() -> DriverRegistry {
    process_registry()
        .try_with(Box::new(
            FileFactory::try_new(Path::new("unused-physical-root"))
                .expect("the File id is canonical"),
        ))
        .expect("File is registered once beside Process")
}

#[test]
fn static_builds_may_select_process_file_or_both() {
    let process = compile_one(PROCESS_ONLY);
    let file = compile_one(FILE_ONLY);
    let mixed = compile_one(MIXED_GRAPH);

    RuntimeBindings::try_new(&process.plan, &process_registry())
        .expect("a Process-only build binds its plan");
    RuntimeBindings::try_new(&file.plan, &file_registry())
        .expect("a File-only build binds its plan");
    RuntimeBindings::try_new(&mixed.plan, &combined_registry())
        .expect("a combined static build binds both families");

    assert!(
        RuntimeBindings::try_new(&mixed.plan, &process_registry()).is_err(),
        "a Process-only build never claims File support"
    );
    assert!(
        RuntimeBindings::try_new(&mixed.plan, &file_registry()).is_err(),
        "a File-only build never claims Process support"
    );
}

#[test]
fn one_lab_world_uses_both_families_without_changing_the_plan() {
    let real = compile_one(MIXED_GRAPH);
    let lab_source = format!("{MIXED_GRAPH}{MIXED_LAB}");
    let scripted = compile_one(&lab_source);

    assert_eq!(
        scripted.hash, real.hash,
        "Lab text is not semantic identity"
    );
    assert_eq!(
        scripted.plan, real.plan,
        "real and scripted worlds execute the same categorical plan"
    );
    RuntimeBindings::try_new(&real.plan, &combined_registry())
        .expect("the exact Lab plan also binds both real Drivers");

    let runs = rastok::lab::run(&scripted.plan, GraphVersion(1), &scripted.labs[0]);
    assert_eq!(runs.len(), 1);
    assert!(
        runs[0].report.passed(),
        "the mixed scenario failed: {:#?}",
        runs[0].report.failures
    );

    let mut states: BTreeMap<String, (bool, bool)> = BTreeMap::new();
    for record in &runs[0].records {
        let Record::Outbound(OutboundRecord::Call(outbound)) = record else {
            continue;
        };
        let state = states
            .entry(outbound.intent.connector_path.to_string())
            .or_default();
        match outbound.status {
            OutboundStatus::Open => state.0 = true,
            OutboundStatus::Closed(_) => state.1 = true,
            OutboundStatus::Abandoned => {}
        }
    }
    assert_eq!(
        states,
        BTreeMap::from([
            (String::from("file"), (true, true)),
            (String::from("shell"), (true, true)),
        ]),
        "both mock effects are opened and closed by the ordinary journal boundary"
    );
}

#[test]
fn a_file_mock_cannot_bypass_its_typed_output_contract() {
    let malformed = format!(
        "{MIXED_GRAPH}{}",
        MIXED_LAB.replace("out = base64! { ZmlsZS1tb2Nr }", "out = \"not bytes\"")
    );
    let bag = compile(&[malformed.as_str()], &catalogue())
        .expect_err("a text reply cannot inhabit File.out");
    let codes: BTreeSet<_> = bag
        .diagnostics()
        .iter()
        .map(|diagnostic| diagnostic.code().as_str())
        .collect();
    assert!(
        codes.contains("E-PORT-TYPE"),
        "the pin refusal is typed: {codes:?}"
    );
}

#[test]
fn a_reduced_image_refuses_an_omitted_network_family_before_vm_creation() {
    let source = r#"
graph websocket_only : App {
  input in: text
  connector socket : WebSocketClient {
    url = "ws://example.test/socket"
    codec = text
    handshake_timeout = duration("5s")
    effect = idempotent ; behavior = deterministic
    versions = ["http/1.1"]
  }
  in -> socket
  socket.out -> nothing
  socket.exit -> nothing
}
"#;
    let compilation = compile_one(source);
    let Err(error) = RuntimeBindings::try_new(&compilation.plan, &process_registry()) else {
        panic!("a Process-only image cannot create bindings for WebSocketClient")
    };
    assert!(
        error.to_string().contains("rastok/websocket-client@2"),
        "the omitted semantic capability is named before any VM exists: {error}"
    );
}
