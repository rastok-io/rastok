// The binary wires the OS to the machine: filesystem and process
// are its business — the frontier lints stop at this file.
#![allow(clippy::disallowed_types, clippy::disallowed_methods)]
//! The `rastok` binary: `check`, `run`, `simulate`, and project OpenAPI
//! resolution.
//!
//! One deployed unit in, evidence out. `check` compiles and prints
//! the semantic hash; `run` executes the App against the real
//! Drivers, stdin lines in, boundary envelopes out; `simulate` runs
//! the unit's lab and reports every scenario. Diagnostics render in
//! the rustc shape through [`rastok::render`]. `--resolve-open-api` owns one
//! deterministic generated source without rewriting authored project files.

use rastok::boundary::{EnvelopeDraft, IngressProposal, IngressProvenance, IngressSequence};
use rastok::compiler::Compilation;
use rastok::contract::ContractCatalog;
use rastok::envelope::{Data, Namespace, Value};
use rastok::journal::MemJournal;
use rastok::plan::GraphVersion;
use rastok::render::{Sources, render_all};
use rastok::runtime::vm::Vm;
use rastok::{connectors, lab, node};
use std::io::{BufRead, Write as _};
use std::process::ExitCode;

const USAGE: &str = "usage: rastok new <name>\n       rastok check <file.rkg>...\n       rastok run <file.rkg>... [--file-root <dir>] [--deployment <profile.json>] [--json-logs] [--record <file.rkrec>]\n       rastok replay <file.rkrec> [--ui [--ui-port <port>]]\n       rastok simulate <file.rkg>... [--deployment <profile.json>] [--golden <dir>] [--bless] [--ui [--ui-port <port>]]\n       rastok --resolve-open-api <project-dir> [--check]";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct UiOptions {
    enabled: bool,
    port: Option<u16>,
}

impl UiOptions {
    fn enable(&mut self) -> Result<(), String> {
        if self.enabled {
            return Err(String::from("--ui may be supplied only once"));
        }
        self.enabled = true;
        Ok(())
    }

    fn set_port(&mut self, spelling: Option<&str>) -> Result<(), String> {
        if self.port.is_some() {
            return Err(String::from("--ui-port may be supplied only once"));
        }
        let port = spelling
            .and_then(|spelling| spelling.parse::<u16>().ok())
            .filter(|port| *port != 0)
            .ok_or_else(|| String::from("--ui-port requires one non-zero TCP port"))?;
        self.port = Some(port);
        Ok(())
    }

    fn validate(self) -> Result<Self, String> {
        if self.port.is_some() && !self.enabled {
            return Err(String::from("--ui-port requires --ui"));
        }
        #[cfg(not(feature = "hosted-ui"))]
        if self.enabled {
            return Err(String::from(
                "--ui requires a build with the `hosted-ui` feature",
            ));
        }
        Ok(self)
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (command, paths) = match args.split_first() {
        Some((command, paths)) if !paths.is_empty() => (command.as_str(), paths),
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    match command {
        "new" => new_graph(paths),
        "replay" => replay(paths),
        "--resolve-open-api" => resolve_open_api(paths),
        "check" => {
            if paths
                .iter()
                .any(|argument| matches!(argument.as_str(), "--ui" | "--ui-port"))
            {
                eprintln!("error: --ui is available only for `simulate` and recorded `replay`");
                return ExitCode::from(2);
            }
            check(paths)
        }
        "run" => {
            let mut files = Vec::new();
            let mut file_root: Option<std::path::PathBuf> = None;
            let mut deployment: Option<std::path::PathBuf> = None;
            let mut json_logs = false;
            let mut record = None;
            let mut rest = paths.iter();
            while let Some(argument) = rest.next() {
                match argument.as_str() {
                    "--record" => match rest.next() {
                        Some(path) if record.is_none() && !path.starts_with('-') => {
                            record = Some(std::path::PathBuf::from(path))
                        }
                        _ => {
                            eprintln!("error: --record requires one unique output path");
                            return ExitCode::from(2);
                        }
                    },
                    "--file-root" => match rest.next() {
                        Some(dir) => file_root = Some(std::path::PathBuf::from(dir)),
                        None => {
                            eprintln!("{USAGE}");
                            return ExitCode::from(2);
                        }
                    },
                    "--deployment" => match rest.next() {
                        Some(path) if deployment.is_none() => {
                            deployment = Some(std::path::PathBuf::from(path));
                        }
                        _ => {
                            eprintln!("error: --deployment requires one unique profile path");
                            return ExitCode::from(2);
                        }
                    },
                    "--json-logs" if !json_logs => json_logs = true,
                    "--json-logs" => {
                        eprintln!("error: --json-logs may be supplied only once");
                        return ExitCode::from(2);
                    }
                    "--ui" | "--ui-port" => {
                        eprintln!(
                            "error: --ui is available only for `simulate` and recorded `replay`; real-I/O experiments are not yet available in the visual debugger"
                        );
                        return ExitCode::from(2);
                    }
                    _ => files.push(argument.clone()),
                }
            }
            if files.is_empty() {
                eprintln!("{USAGE}");
                return ExitCode::from(2);
            }
            let boot = match deployment.as_deref().map(rastok::deployment_config::load) {
                Some(Ok(boot)) => Some(boot),
                Some(Err(error)) => {
                    eprintln!("error: {error}");
                    return ExitCode::FAILURE;
                }
                None => None,
            };
            run(
                &files,
                file_root.as_deref(),
                boot.as_ref(),
                json_logs,
                record.as_deref(),
            )
        }
        "simulate" => {
            let mut files = Vec::new();
            let mut golden_dir: Option<std::path::PathBuf> = None;
            let mut bless = false;
            let mut deployment: Option<std::path::PathBuf> = None;
            let mut ui = UiOptions::default();
            let mut rest = paths.iter();
            while let Some(argument) = rest.next() {
                match argument.as_str() {
                    "--golden" => match rest.next() {
                        Some(dir) => golden_dir = Some(std::path::PathBuf::from(dir)),
                        None => {
                            eprintln!("{USAGE}");
                            return ExitCode::from(2);
                        }
                    },
                    "--bless" => bless = true,
                    "--deployment" => match rest.next() {
                        Some(path) if deployment.is_none() => {
                            deployment = Some(std::path::PathBuf::from(path));
                        }
                        _ => {
                            eprintln!("error: --deployment requires one unique profile path");
                            return ExitCode::from(2);
                        }
                    },
                    "--ui" => {
                        if let Err(error) = ui.enable() {
                            eprintln!("error: {error}");
                            return ExitCode::from(2);
                        }
                    }
                    "--ui-port" => {
                        if let Err(error) = ui.set_port(rest.next().map(String::as_str)) {
                            eprintln!("error: {error}");
                            return ExitCode::from(2);
                        }
                    }
                    _ => files.push(argument.clone()),
                }
            }
            if bless && golden_dir.is_none() {
                eprintln!("error: --bless requires --golden <dir>");
                return ExitCode::from(2);
            }
            let boot = match deployment.as_deref().map(rastok::deployment_config::load) {
                Some(Ok(boot)) => Some(boot),
                Some(Err(error)) => {
                    eprintln!("error: {error}");
                    return ExitCode::FAILURE;
                }
                None => None,
            };
            let ui = match ui.validate() {
                Ok(ui) => ui,
                Err(error) => {
                    eprintln!("error: {error}");
                    return ExitCode::from(2);
                }
            };
            simulate(&files, golden_dir.as_deref(), bless, boot.as_ref(), ui)
        }
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

fn new_graph(arguments: &[String]) -> ExitCode {
    let [name] = arguments else {
        eprintln!("error: new requires one name, without a path or extension");
        return ExitCode::from(2);
    };
    if !name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
        || !name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-'))
    {
        eprintln!(
            "error: new requires a name starting with a letter or underscore, followed by letters, digits, underscores or hyphens; omit .rkg"
        );
        return ExitCode::from(2);
    }
    let source = include_str!("starter.rkg").replace("starter", &name.replace('-', "_"));
    let path = format!("{name}.rkg");
    let result = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .and_then(|mut file| file.write_all(source.as_bytes()));
    match result {
        Ok(()) => {
            println!(
                "Created {path}\nNext: rastok simulate {path}\nDebug: rastok simulate {path} --ui"
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!(
                "error: cannot create {path:?} (existing files are never overwritten): {error}"
            );
            ExitCode::FAILURE
        }
    }
}

fn resolve_open_api(arguments: &[String]) -> ExitCode {
    let mut root: Option<&std::path::Path> = None;
    let mut mode = rastok::openapi::ResolveMode::Write;
    for argument in arguments {
        if argument == "--check" {
            mode = rastok::openapi::ResolveMode::Check;
        } else if root.replace(std::path::Path::new(argument)).is_some() {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    }
    let Some(root) = root else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    match rastok::openapi::resolve_project(root, mode) {
        Ok(report) => {
            let action = match (mode, report.changed) {
                (rastok::openapi::ResolveMode::Write, true) => "updated",
                (rastok::openapi::ResolveMode::Write, false) => "unchanged",
                (rastok::openapi::ResolveMode::Check, _) => "current",
            };
            println!(
                "ok: {} is {action} ({} endpoint(s), {} OpenAPI document(s), {} type(s))",
                report.output.display(),
                report.endpoints,
                report.specifications,
                report.types
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Reads and compiles the unit; on refusal, renders the bag to
/// stderr and reports failure.
fn load(paths: &[String]) -> Result<(Compilation, Sources), ExitCode> {
    let mut sources = Sources::new();
    let mut texts = Vec::new();
    for path in paths {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                sources.add(path.clone(), text.clone());
                texts.push(text);
            }
            Err(error) => {
                eprintln!("error: cannot read `{path}`: {error}");
                return Err(ExitCode::FAILURE);
            }
        }
    }
    let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
    let registry = match ContractCatalog::try_m0() {
        Ok(registry) => registry,
        Err(defect) => {
            eprintln!("error: the standard catalogue is broken: {defect}");
            std::process::exit(2);
        }
    };
    // The App root for `asset(…)` resolution (D-2026-08-10-5): the
    // directory of the file DECLARING the `: App` graph, found by
    // the PARSER's reading — never a line scan, so a multi-line
    // annotation or a raw block cannot fool it. Fallback: the
    // first file's directory.
    let app_index = rastok::compiler::app_source_index(&refs, &registry).unwrap_or(0);
    let asset_root = paths
        .get(app_index)
        .and_then(|path| std::path::Path::new(path).parent())
        .map(std::path::Path::to_path_buf);
    match rastok::compiler::compile_with_assets(&refs, &registry, asset_root.as_deref()) {
        Ok(compilation) => {
            let warnings = render_all(&compilation.warnings, &sources);
            if !warnings.is_empty() {
                eprint!("{warnings}");
            }
            Ok((compilation, sources))
        }
        Err(bag) => {
            eprint!("{}", render_all(bag.diagnostics(), &sources));
            Err(ExitCode::FAILURE)
        }
    }
}

fn check(paths: &[String]) -> ExitCode {
    let Ok((compilation, _)) = load(paths) else {
        return ExitCode::FAILURE;
    };
    println!(
        "ok: {} node(s), semantic hash {}",
        compilation.plan.nodes.len(),
        compilation.hash
    );
    ExitCode::SUCCESS
}

/// Hosted mapping from one wall-clock observation plus monotonic elapsed time
/// to the VM's explicit logical timeline. Nothing below this boundary reads
/// the OS clock, and wall-clock adjustments cannot move a running VM back.
struct HostedTimeSource {
    origin: i64,
    started: std::time::Instant,
}

impl HostedTimeSource {
    fn start() -> Result<Self, String> {
        let since_epoch = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| String::from("the hosted wall clock is before the Unix epoch"))?;
        let origin = i64::try_from(since_epoch.as_nanos()).map_err(|_| {
            String::from("the hosted wall clock does not fit Rastok's timestamp range")
        })?;
        Ok(Self {
            origin,
            started: std::time::Instant::now(),
        })
    }

    fn logical_now(&self) -> i64 {
        let elapsed = i64::try_from(self.started.elapsed().as_nanos()).unwrap_or(i64::MAX);
        self.origin.saturating_add(elapsed)
    }
}

/// Runs the App against the real Drivers: each stdin line is one
/// text submission; public boundary envelopes print to stdout.
fn run(
    paths: &[String],
    file_root: Option<&std::path::Path>,
    boot: Option<&rastok::deployment::BootBindings>,
    json_logs: bool,
    record: Option<&std::path::Path>,
) -> ExitCode {
    let Ok((compilation, _)) = load(paths) else {
        return ExitCode::FAILURE;
    };
    let mut recorder = match record
        .map(|path| rastok::recording::Recorder::create(path, &compilation, GraphVersion(1), boot))
        .transpose()
    {
        Ok(recorder) => recorder,
        Err(error) => {
            eprintln!("error: {error}");
            return ExitCode::FAILURE;
        }
    };
    let registry = match connectors::hosted_registry(file_root) {
        Ok(registry) => registry,
        Err(error) => {
            eprintln!("error: {error}");
            return ExitCode::FAILURE;
        }
    };
    let bindings = match boot {
        Some(boot) => {
            connectors::RuntimeBindings::try_new_with_boot(&compilation.plan, &registry, boot)
        }
        None => connectors::RuntimeBindings::try_new(&compilation.plan, &registry),
    };
    let mut world = match bindings {
        Ok(bindings) => bindings,
        Err(error) => {
            eprintln!("error: {error}");
            return ExitCode::FAILURE;
        }
    };
    // Binding is complete before either mutable execution authority exists.
    let version = GraphVersion(1);
    let mut journal = boot.map_or_else(
        || MemJournal::new(version),
        |boot| MemJournal::new_with_deployment(version, boot.attestation().clone()),
    );
    let mut log_projector = if json_logs {
        match rastok::observability::json_log_channel(256) {
            Ok((sink, receiver)) => {
                let exporter = std::thread::Builder::new()
                    .name(String::from("rastok-json-logs"))
                    .spawn(move || {
                        let stderr = std::io::stderr();
                        let mut stderr = stderr.lock();
                        while let Some(line) = receiver.recv() {
                            if writeln!(stderr, "{line}").is_err() {
                                break;
                            }
                        }
                    });
                if let Err(error) = exporter {
                    eprintln!(
                        "warning: structured-log exporter unavailable; observations will be dropped: {error}"
                    );
                }
                Some(rastok::observability::JournalLogProjector::new(sink))
            }
            Err(error) => {
                eprintln!("warning: structured-log exporter unavailable: {error}");
                None
            }
        }
    } else {
        None
    };
    let time = match HostedTimeSource::start() {
        Ok(time) => time,
        Err(error) => {
            eprintln!("error: {error}");
            return ExitCode::FAILURE;
        }
    };
    let mut vm = Vm::boot_program(
        &compilation.plan,
        compilation.hash,
        version,
        time.logical_now(),
        boot,
    );
    if let Err(error) = recording_checkpoint(&mut recorder, &journal, vm.now()) {
        eprintln!("error: {error}");
        return ExitCode::FAILURE;
    }
    let inbound_path = match node::Path::new(["cli"]) {
        Ok(path) => path,
        Err(_) => return ExitCode::FAILURE,
    };
    let polls_inbound = world.has_runnable_inbound();
    let (lines, input) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        for line in stdin.lock().lines() {
            if lines.send(Some(line)).is_err() {
                return;
            }
        }
        let _ = lines.send(None);
    });
    let mut printed_outputs = 0;
    let mut stdin_open = true;
    loop {
        let mut progressed = false;
        let target = time.logical_now();
        if target > vm.now()
            && let Err(error) = vm.advance(&mut journal, &mut world, target)
        {
            eprintln!("error: {error}");
            return ExitCode::FAILURE;
        }
        while let Ok(message) = input.try_recv() {
            let Some(line) = message else {
                stdin_open = false;
                break;
            };
            let line = match line {
                Ok(line) => line,
                Err(error) => {
                    eprintln!("error: stdin: {error}");
                    return ExitCode::FAILURE;
                }
            };
            let submission = cli_submission(&inbound_path, line);
            if let Err(error) = vm.inject(&mut journal, submission) {
                eprintln!("error: the submission was refused: {error}");
                return ExitCode::FAILURE;
            }
            progressed = true;
        }

        match drive_boundary(&mut vm, &mut journal, &mut world) {
            Ok(boundary_progress) => progressed |= boundary_progress,
            Err(error) => {
                if let Err(record_error) = recording_checkpoint(&mut recorder, &journal, vm.now()) {
                    eprintln!("error: {record_error}");
                }
                eprintln!("error: {error}");
                return ExitCode::FAILURE;
            }
        }
        if let Err(error) = recording_checkpoint(&mut recorder, &journal, vm.now()) {
            eprintln!("error: {error}");
            return ExitCode::FAILURE;
        }
        if let Some(projector) = &mut log_projector {
            let _ = projector.project_journal(&journal);
        }
        print_new_outputs(&vm, &mut printed_outputs);

        if !stdin_open && !polls_inbound && !vm.has_open_obligations() {
            break;
        }
        if !progressed {
            // The next polling round translates monotonic elapsed time at the one
            // hosted boundary. Deadlines remain deterministic logical dates and
            // Drivers never manufacture timeout outcomes themselves.
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
    if let Some(recorder) = recorder {
        match recorder.finish(&compilation, &journal, vm.now()) {
            Ok(()) => eprintln!(
                "recorded: {} causal records · seal {}",
                journal.records().len(),
                rastok::golden::project(journal.records()).seal_hex()
            ),
            Err(error) => {
                eprintln!("error: {error}");
                return ExitCode::FAILURE;
            }
        }
    }
    ExitCode::SUCCESS
}

fn recording_checkpoint(
    recorder: &mut Option<rastok::recording::Recorder>,
    journal: &MemJournal,
    now: i64,
) -> Result<(), rastok::recording::Error> {
    if let Some(recorder) = recorder {
        recorder.checkpoint(journal, now)?;
    }
    Ok(())
}

fn replay(arguments: &[String]) -> ExitCode {
    let mut path = None;
    let mut ui = UiOptions::default();
    let mut rest = arguments.iter();
    while let Some(argument) = rest.next() {
        let result = match argument.as_str() {
            "--ui" => ui.enable(),
            "--ui-port" => ui.set_port(rest.next().map(String::as_str)),
            value if !value.starts_with('-') && path.is_none() => {
                path = Some(std::path::PathBuf::from(value));
                Ok(())
            }
            _ => Err(String::from(
                "replay requires one recording path and only --ui / --ui-port options",
            )),
        };
        if let Err(error) = result {
            eprintln!("error: {error}");
            return ExitCode::from(2);
        }
    }
    let Some(path) = path else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    let ui = match ui.validate() {
        Ok(ui) => ui,
        Err(error) => {
            eprintln!("error: {error}");
            return ExitCode::from(2);
        }
    };
    let recording = match rastok::recording::Recording::read(&path) {
        Ok(recording) => recording,
        Err(error) => {
            eprintln!("error: {error}");
            return ExitCode::FAILURE;
        }
    };
    eprintln!(
        "replay: {} causal records · seal {} · {} · no Driver I/O",
        recording.records().len(),
        rastok::golden::project(recording.records()).seal_hex(),
        if recording.finished() {
            "completed recording"
        } else {
            "incomplete run; retained prefix only"
        }
    );
    #[cfg(feature = "hosted-ui")]
    if ui.enabled {
        return match rastok::recording::serve_ui(&recording, ui.port) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("error: {error}");
                ExitCode::FAILURE
            }
        };
    }
    #[cfg(not(feature = "hosted-ui"))]
    let _ = ui;
    match recording.inspect(recording.records().len()) {
        Ok((_, vm)) => {
            print_new_outputs(&vm, &mut 0);
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn drive_boundary<'plan>(
    vm: &mut Vm<'plan>,
    journal: &mut MemJournal,
    world: &mut connectors::RuntimeBindings<'plan>,
) -> Result<bool, String> {
    let mut progressed = false;
    for _ in 0..64 {
        match world
            .poll_inbound(vm, journal)
            .map_err(|error| error.to_string())?
        {
            Some(_) => progressed = true,
            None => break,
        }
    }
    vm.run_ready_with_bindings(journal, world)
        .map_err(|error| error.to_string())?;
    let responses = world
        .poll_inbound_call_responses(vm)
        .map_err(|error| error.to_string())?;
    let timeouts = world
        .poll_inbound_call_timeouts(vm)
        .map_err(|error| error.to_string())?;
    let inbound_events = world
        .poll_inbound_call_events(vm)
        .map_err(|error| error.to_string())?;
    let outbound = vm
        .poll_drivers(journal, world)
        .map_err(|error| error.to_string())?;
    for (_, reason) in &outbound.rejected {
        eprintln!("err: a reply was refused before closure: {reason}");
    }
    progressed |= responses.started > 0
        || timeouts.started > 0
        || inbound_events.queued > 0
        || outbound.settled > 0;
    vm.run_ready_with_bindings(journal, world)
        .map_err(|error| error.to_string())?;
    Ok(progressed)
}

#[cfg(feature = "hosted-ui")]
fn transaction_fault(message: String) -> rastok::ui::FaultView {
    rastok::ui::FaultView {
        message,
        help: String::from(
            "The faulting transaction committed nothing and still owns its work. Retry the step after correcting its external cause, or close the run.",
        ),
        commit: String::from("nothing from the faulting transaction was committed"),
        retryable: true,
    }
}

#[cfg(feature = "hosted-ui")]
fn boundary_fault(message: String) -> rastok::ui::FaultView {
    rastok::ui::FaultView {
        message,
        help: String::from(
            "Execution stopped at the physical boundary. Inspect the retained history; close and restart rather than replaying an already committed transaction.",
        ),
        commit: String::from(
            "transactions committed before this boundary failure remain committed",
        ),
        retryable: false,
    }
}

fn cli_submission(inbound_path: &node::Path, line: String) -> IngressProposal {
    IngressProposal {
        provenance: IngressProvenance::single(inbound_path.clone()),
        draft: EnvelopeDraft {
            port: node::Port::new("in").unwrap_or_else(|_| unreachable!("`in` is a valid port")),
            data: Data::materialized(Value::Text(line)),
            ext: {
                let mut ext = Namespace::new();
                ext.insert(
                    String::from("content_type"),
                    rastok::envelope::MetaValue::Scalar(rastok::envelope::Scalar::Text(
                        String::from("text/plain"),
                    )),
                );
                ext
            },
        },
        sequence: IngressSequence::Next,
        session: None,
    }
}

fn print_new_outputs(vm: &Vm<'_>, printed_outputs: &mut usize) {
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    for output in &vm.outputs()[*printed_outputs..] {
        let _ = writeln!(
            out,
            "{}: {}",
            output.port,
            render_data(
                output.envelope.data(),
                vm.boundary_annotations(&output.port),
            )
        );
    }
    *printed_outputs = vm.outputs().len();
}

/// Runs every lab of the unit; one line per scenario, non-zero exit
/// on any red.
fn simulate(
    paths: &[String],
    golden_dir: Option<&std::path::Path>,
    bless: bool,
    boot: Option<&rastok::deployment::BootBindings>,
    ui: UiOptions,
) -> ExitCode {
    let Ok((compilation, _)) = load(paths) else {
        return ExitCode::FAILURE;
    };
    if compilation.labs.is_empty() {
        eprintln!("error: this unit declares no lab");
        return ExitCode::FAILURE;
    }
    #[cfg(feature = "hosted-ui")]
    if ui.enabled {
        return simulate_with_ui(&compilation, golden_dir, bless, boot, ui);
    }
    #[cfg(not(feature = "hosted-ui"))]
    let _ = ui;
    let mut failed = 0usize;
    let mut total = 0usize;
    for lab_decl in &compilation.labs {
        let runs = lab::run_program(
            &compilation.plan,
            compilation.hash,
            GraphVersion(1),
            lab_decl,
            boot,
        );
        for run in runs {
            let report = &run.report;
            total += 1;
            let golden_failures =
                check_or_bless_golden(report, &run.records, golden_dir, bless, &compilation.plan);
            let passed = report.passed() && golden_failures.is_empty();
            let verdict = if passed { "PASS" } else { "FAIL" };
            match &report.label {
                Some(label) => {
                    println!(
                        "{verdict} {}.{} — {label}",
                        report.simulation, report.scenario
                    );
                }
                None => println!("{verdict} {}.{}", report.simulation, report.scenario),
            }
            if !passed {
                failed += 1;
                for failure in report.failures.iter().chain(&golden_failures) {
                    println!("     {failure}");
                }
            }
        }
    }
    println!("{} scenario(s), {} failed", total, failed);
    if failed == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

#[cfg(feature = "hosted-ui")]
fn simulate_with_ui(
    compilation: &Compilation,
    golden_dir: Option<&std::path::Path>,
    bless: bool,
    boot: Option<&rastok::deployment::BootBindings>,
    options: UiOptions,
) -> ExitCode {
    use rastok::ui::{ActivityView, ExecutionState, FaultView, UiCommand, UiServer};

    let Some((simulation, scenario)) = compilation
        .labs
        .iter()
        .flat_map(|lab| &lab.simulations)
        .find_map(|simulation| {
            simulation
                .scenarios
                .first()
                .map(|scenario| (simulation, scenario))
        })
    else {
        eprintln!("error: this unit declares no scenario");
        return ExitCode::FAILURE;
    };
    let mut session = match lab::Simulator::try_new_program(
        &compilation.plan,
        compilation.hash,
        GraphVersion(1),
        &simulation.name,
        &simulation.mocks,
        scenario,
        boot,
    ) {
        Ok(session) => session,
        Err(error) => {
            eprintln!("error: the Lab Driver composition refused: {error}");
            return ExitCode::FAILURE;
        }
    };
    let mut state = ExecutionState::Paused;
    let mut revision = 0_u64;
    let mut fault: Option<FaultView> = None;
    let mut activity: Option<ActivityView> = None;
    let mut boundary_ordinal = 0_u64;
    let mut session_id = 0_u64;
    let mut next_session_id = 1_u64;
    let initial = match rastok::ui::snapshot(rastok::ui::SnapshotInput {
        revision,
        mode: "simulate",
        state,
        plan: &compilation.plan,
        definitions: &compilation.source_definitions,
        vm: session.vm(),
        journal: session.journal(),
        fault: None,
        activity: None,
        scenario: Some(simulation_view(compilation, &session, &[])),
        session: session_id,
        boundary: boundary_ordinal,
    }) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            eprintln!("error: the initial UI view could not be serialized: {error}");
            return ExitCode::FAILURE;
        }
    };
    let mut server = match UiServer::start(options.port, initial) {
        Ok(server) => server,
        Err(error) => {
            eprintln!("error: {error}");
            return ExitCode::FAILURE;
        }
    };
    println!("ui: {}", server.url());

    let mut step_requested = false;
    let mut until_assertion = false;
    let mut until_action = false;
    let mut close_requested = false;
    let mut reported = false;
    let mut any_failed = false;
    let mut golden_failures = Vec::new();
    loop {
        let records_before = session.journal().records().len();
        let observations_before = session.vm().envelope_views().len();
        let failures_before = session.failures().len();
        let mut dirty = false;
        let mut activity_decided = false;
        while let Some(command) = server.try_command() {
            match command {
                UiCommand::Resume | UiCommand::RunToCompletion
                    if !matches!(state, ExecutionState::Faulted | ExecutionState::Completed) =>
                {
                    state = ExecutionState::Running;
                    until_assertion = false;
                    until_action = false;
                    step_requested = false;
                    fault = None;
                    dirty = true;
                }
                UiCommand::RunUntilAssertion
                    if !matches!(state, ExecutionState::Faulted | ExecutionState::Completed) =>
                {
                    state = ExecutionState::Running;
                    until_assertion = true;
                    until_action = false;
                    step_requested = false;
                    fault = None;
                    dirty = true;
                }
                UiCommand::Pause if state == ExecutionState::Running => {
                    state = if session.is_advancing_time()
                        || session.vm().awaiting_outbound_calls() == 0
                    {
                        ExecutionState::Paused
                    } else {
                        ExecutionState::PauseRequested
                    };
                    until_assertion = false;
                    until_action = false;
                    dirty = true;
                }
                UiCommand::Step if state == ExecutionState::Paused && !session.is_quiescent() => {
                    step_requested = true;
                    fault = None;
                    dirty = true;
                }
                UiCommand::RetryStep
                    if state == ExecutionState::Faulted
                        && fault.as_ref().is_some_and(|fault| fault.retryable) =>
                {
                    fault = None;
                    state = ExecutionState::Paused;
                    step_requested = true;
                    dirty = true;
                }
                UiCommand::Step | UiCommand::NextScenarioAction
                    if state == ExecutionState::Paused =>
                {
                    let action_records_before = session.journal().records().len();
                    match session.next_action() {
                        Ok(Some(action)) => {
                            fault = None;
                            reported = false;
                            if command == UiCommand::NextScenarioAction && !session.is_quiescent() {
                                until_action = true;
                                state = ExecutionState::Running;
                            }
                            boundary_ordinal = boundary_ordinal.saturating_add(1);
                            activity = Some(ActivityView::scenario_action(
                                scenario_action_name(action),
                                session.journal(),
                                action_records_before,
                                boundary_ordinal,
                            ));
                            activity_decided = true;
                        }
                        Ok(None) => {}
                        Err(error) => {
                            fault = Some(FaultView {
                                message: error,
                                help: String::from(
                                    "Use Next transition until the current action reaches its next effect.",
                                ),
                                commit: String::from("no scenario action was applied"),
                                retryable: false,
                            });
                        }
                    }
                    dirty = true;
                }
                UiCommand::ResetScenario => {
                    match recreate_scenario(
                        compilation,
                        session.simulation(),
                        session.scenario(),
                        boot,
                    ) {
                        Ok(recreated) => {
                            session = recreated;
                            session_id = next_session_id;
                            next_session_id = next_session_id.saturating_add(1);
                            state = ExecutionState::Paused;
                            fault = None;
                            activity = None;
                            boundary_ordinal = 0;
                            activity_decided = true;
                            reported = false;
                            any_failed = false;
                            golden_failures.clear();
                            until_action = false;
                            until_assertion = false;
                            step_requested = false;
                        }
                        Err(error) => fault = Some(boundary_fault(error)),
                    }
                    dirty = true;
                }
                UiCommand::SelectScenario {
                    simulation,
                    scenario,
                } => {
                    match recreate_scenario(compilation, &simulation, &scenario, boot) {
                        Ok(selected) => {
                            session = selected;
                            session_id = next_session_id;
                            next_session_id = next_session_id.saturating_add(1);
                            state = ExecutionState::Paused;
                            fault = None;
                            activity = None;
                            boundary_ordinal = 0;
                            activity_decided = true;
                            reported = false;
                            any_failed = false;
                            golden_failures.clear();
                            until_action = false;
                            until_assertion = false;
                            step_requested = false;
                        }
                        Err(error) => {
                            fault = Some(FaultView {
                                message: error,
                                help: String::from(
                                    "Choose one of the checked simulation/scenario pairs listed by the UI.",
                                ),
                                commit: String::from(
                                    "the current scenario world was left unchanged",
                                ),
                                retryable: false,
                            });
                        }
                    }
                    dirty = true;
                }
                UiCommand::Close => close_requested = true,
                UiCommand::Pause
                | UiCommand::Step
                | UiCommand::RetryStep
                | UiCommand::NextScenarioAction
                | UiCommand::RunUntilAssertion
                | UiCommand::Resume
                | UiCommand::RunToCompletion => {}
            }
        }
        if close_requested {
            return if state == ExecutionState::Faulted || any_failed {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            };
        }

        if step_requested && state == ExecutionState::Paused && session.is_quiescent() {
            let action_records_before = session.journal().records().len();
            match session.next_action() {
                Ok(Some(action)) => {
                    boundary_ordinal = boundary_ordinal.saturating_add(1);
                    activity = Some(ActivityView::scenario_action(
                        scenario_action_name(action),
                        session.journal(),
                        action_records_before,
                        boundary_ordinal,
                    ));
                    activity_decided = true;
                }
                Ok(None) => {}
                Err(error) => {
                    fault = Some(boundary_fault(error));
                    state = ExecutionState::Faulted;
                }
            }
            step_requested = false;
            dirty = true;
        } else if step_requested && state == ExecutionState::Paused {
            step_requested = false;
            let transaction_records_before = session.journal().records().len();
            match session.step_transaction() {
                Ok(ran) => {
                    let records_changed =
                        transaction_records_before != session.journal().records().len();
                    if ran || records_changed {
                        boundary_ordinal = boundary_ordinal.saturating_add(1);
                        activity = ActivityView::journal_change(
                            session.journal(),
                            transaction_records_before,
                            Some(boundary_ordinal),
                        )
                        .or_else(|| {
                            Some(ActivityView::external(
                                boundary_ordinal,
                                "transaction",
                                "Execution advanced without a new causal record",
                            ))
                        });
                        activity_decided = true;
                    } else if session.is_quiescent() {
                        // Discovering quiescence is not a visible boundary.
                        // Keep this one Step request until the next authored
                        // action, just as continuous execution would do.
                        step_requested = true;
                    }
                    // Step starts from an already-safe pause. An opened Call
                    // remains inspectable there and may depend on a later Lab
                    // `advance`; only an explicit Pause requested while
                    // Running waits for outstanding Calls to settle.
                    state = ExecutionState::Paused;
                }
                Err(error) => {
                    fault = Some(if error.retryable() {
                        transaction_fault(error.to_string())
                    } else {
                        boundary_fault(error.to_string())
                    });
                    state = ExecutionState::Faulted;
                }
            }
            dirty = !step_requested;
        } else if state == ExecutionState::PauseRequested {
            let poll_records_before = session.journal().records().len();
            match session.poll_world() {
                Ok(progressed) => {
                    dirty |= progressed;
                    if progressed {
                        boundary_ordinal = boundary_ordinal.saturating_add(1);
                        activity = ActivityView::journal_change(
                            session.journal(),
                            poll_records_before,
                            Some(boundary_ordinal),
                        )
                        .or_else(|| {
                            Some(ActivityView::external(
                                boundary_ordinal,
                                "connector-boundary",
                                "A Connector exchange observation became ready",
                            ))
                        });
                        activity_decided = true;
                    }
                }
                Err(error) => {
                    fault = Some(boundary_fault(error));
                    state = ExecutionState::Faulted;
                    dirty = true;
                }
            }
            if state == ExecutionState::PauseRequested
                && session.vm().awaiting_outbound_calls() == 0
            {
                state = ExecutionState::Paused;
                dirty = true;
            }
        } else if state == ExecutionState::Running && !activity_decided {
            if until_action && session.is_quiescent() {
                state = ExecutionState::Paused;
                until_action = false;
                dirty = true;
            } else if until_assertion
                && session.is_quiescent()
                && session.next_action_kind() == Some(lab::ScenarioActionKind::Expect)
            {
                state = ExecutionState::Paused;
                until_assertion = false;
                dirty = true;
            } else if session.is_quiescent() {
                let action_records_before = session.journal().records().len();
                match session.next_action() {
                    Ok(Some(action)) => {
                        boundary_ordinal = boundary_ordinal.saturating_add(1);
                        activity = Some(ActivityView::scenario_action(
                            scenario_action_name(action),
                            session.journal(),
                            action_records_before,
                            boundary_ordinal,
                        ));
                        activity_decided = true;
                        dirty = true;
                    }
                    Ok(None) => {
                        state = ExecutionState::Completed;
                        dirty = true;
                    }
                    Err(error) => {
                        fault = Some(boundary_fault(error));
                        state = ExecutionState::Faulted;
                        dirty = true;
                    }
                }
            } else {
                let transaction_records_before = session.journal().records().len();
                match session.step_transaction() {
                    Ok(ran) => {
                        let records_changed =
                            transaction_records_before != session.journal().records().len();
                        if ran || records_changed {
                            boundary_ordinal = boundary_ordinal.saturating_add(1);
                            activity = ActivityView::journal_change(
                                session.journal(),
                                transaction_records_before,
                                Some(boundary_ordinal),
                            )
                            .or_else(|| {
                                Some(ActivityView::external(
                                    boundary_ordinal,
                                    "transaction",
                                    "Execution advanced without a new causal record",
                                ))
                            });
                            activity_decided = true;
                        }
                        dirty = true;
                    }
                    Err(error) => {
                        fault = Some(if error.retryable() {
                            transaction_fault(error.to_string())
                        } else {
                            boundary_fault(error.to_string())
                        });
                        state = ExecutionState::Faulted;
                        dirty = true;
                    }
                }
            }
            if session.is_complete() {
                state = ExecutionState::Completed;
                dirty = true;
            }
        }

        if state == ExecutionState::Paused && session.is_complete() {
            state = ExecutionState::Completed;
            dirty = true;
        }

        if state == ExecutionState::Completed && !reported {
            let report = session.report();
            golden_failures = check_or_bless_golden(
                &report,
                session.journal().records(),
                golden_dir,
                bless,
                &compilation.plan,
            );
            let passed = report.passed() && golden_failures.is_empty();
            any_failed |= !passed;
            print_scenario_verdict(&report, &golden_failures);
            reported = true;
            dirty = true;
        }
        dirty |= records_before != session.journal().records().len()
            || observations_before != session.vm().envelope_views().len()
            || failures_before != session.failures().len();
        if records_before != session.journal().records().len() && !activity_decided {
            boundary_ordinal = boundary_ordinal.saturating_add(1);
            activity = ActivityView::journal_change(
                session.journal(),
                records_before,
                Some(boundary_ordinal),
            );
        }
        if dirty {
            revision = revision.saturating_add(1);
            match rastok::ui::snapshot(rastok::ui::SnapshotInput {
                revision,
                mode: "simulate",
                state,
                plan: &compilation.plan,
                definitions: &compilation.source_definitions,
                vm: session.vm(),
                journal: session.journal(),
                fault: fault.as_ref(),
                activity: activity.as_ref(),
                scenario: Some(simulation_view(compilation, &session, &golden_failures)),
                session: session_id,
                boundary: boundary_ordinal,
            }) {
                Ok(snapshot) => server.publish(snapshot),
                Err(error) => eprintln!("warning: the UI view was dropped: {error}"),
            }
        } else {
            server.flush();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
}

#[cfg(feature = "hosted-ui")]
fn recreate_scenario<'plan>(
    compilation: &'plan Compilation,
    simulation_name: &str,
    scenario_name: &str,
    boot: Option<&rastok::deployment::BootBindings>,
) -> Result<lab::Simulator<'plan, 'plan>, String> {
    let simulation = compilation
        .labs
        .iter()
        .flat_map(|lab| &lab.simulations)
        .find(|simulation| simulation.name == simulation_name)
        .ok_or_else(|| format!("unknown simulation `{simulation_name}`"))?;
    let scenario = simulation
        .scenarios
        .iter()
        .find(|scenario| scenario.name == scenario_name)
        .ok_or_else(|| {
            format!("simulation `{simulation_name}` has no scenario `{scenario_name}`")
        })?;
    lab::Simulator::try_new_program(
        &compilation.plan,
        compilation.hash,
        GraphVersion(1),
        &simulation.name,
        &simulation.mocks,
        scenario,
        boot,
    )
}

#[cfg(feature = "hosted-ui")]
fn simulation_view(
    compilation: &Compilation,
    session: &lab::Simulator<'_, '_>,
    golden_failures: &[String],
) -> serde_json::Value {
    let next_injection = match session.next_injection_draft() {
        Ok(Some(draft)) => rastok::ui::scenario_injection_json(&compilation.plan, &draft),
        Ok(None) => serde_json::Value::Null,
        Err(error) => serde_json::json!({ "error": error }),
    };
    let available = compilation
        .labs
        .iter()
        .flat_map(|lab| &lab.simulations)
        .map(|simulation| {
            serde_json::json!({
                "name": simulation.name,
                "scenarios": simulation.scenarios.iter().map(|scenario| serde_json::json!({
                    "name": scenario.name,
                    "label": scenario.label,
                })).collect::<Vec<_>>(),
            })
        })
        .collect::<Vec<_>>();
    serde_json::json!({
        "available": available,
        "current": {
            "simulation": session.simulation(),
            "scenario": session.scenario(),
            "definition": rastok::ui::scenario_definition_json(compilation, session.simulation(), session.scenario()),
            "label": session.label(),
            "next_action_index": session.next_action_index(),
            "action_count": session.action_count(),
            "next_action": session.next_action_kind().map(scenario_action_name),
            "last_action": session.last_action_kind().map(scenario_action_name),
            "quiescent": session.is_quiescent(),
            "complete": session.is_complete(),
            "next_injection": next_injection,
            "failures": session.failures(),
            "golden_failures": golden_failures,
            "expectation": rastok::ui::expectation_json(&compilation.plan, session),
        },
    })
}

#[cfg(feature = "hosted-ui")]
const fn scenario_action_name(action: lab::ScenarioActionKind) -> &'static str {
    match action {
        lab::ScenarioActionKind::Inject => "inject",
        lab::ScenarioActionKind::Advance => "advance",
        lab::ScenarioActionKind::Expect => "expect",
    }
}

fn check_or_bless_golden(
    report: &lab::ScenarioReport,
    records: &[rastok::journal::Record],
    golden_dir: Option<&std::path::Path>,
    bless: bool,
    plan: &rastok::plan::ValidatedExecutionPlan,
) -> Vec<String> {
    let Some(dir) = golden_dir else {
        return Vec::new();
    };
    let trace = rastok::golden::project(records);
    let rendered = render_trace(&trace);
    let path = dir.join(format!("{}.{}.trace", report.simulation, report.scenario));
    if bless {
        return std::fs::create_dir_all(dir)
            .and_then(|()| std::fs::write(&path, &rendered))
            .err()
            .map(|error| vec![format!("cannot bless `{}`: {error}", path.display())])
            .unwrap_or_default();
    }
    match std::fs::read_to_string(&path) {
        Err(_) => vec![format!(
            "no golden at `{}`; bless with --bless",
            path.display()
        )],
        Ok(stored) if stored != rendered => {
            let divergence = parse_stored(&stored)
                .and_then(|stored| rastok::golden::explain_diff(&trace, &stored, plan));
            vec![match divergence {
                Some(divergence) => format!("golden divergence: {divergence}"),
                None => String::from("the stored golden is unreadable by this build"),
            }]
        }
        Ok(_) => Vec::new(),
    }
}

#[cfg(feature = "hosted-ui")]
fn print_scenario_verdict(report: &lab::ScenarioReport, golden_failures: &[String]) {
    let passed = report.passed() && golden_failures.is_empty();
    let verdict = if passed { "PASS" } else { "FAIL" };
    match &report.label {
        Some(label) => println!(
            "{verdict} {}.{} — {label}",
            report.simulation, report.scenario
        ),
        None => println!("{verdict} {}.{}", report.simulation, report.scenario),
    }
    if !passed {
        for failure in report.failures.iter().chain(golden_failures) {
            println!("     {failure}");
        }
    }
}

/// One hex line per canonical record, then the seal — the stored
/// golden shape.
fn render_trace(trace: &rastok::golden::Trace) -> String {
    let mut out = String::new();
    for index in 0..trace.len() {
        if let Some(bytes) = trace.record_bytes(index) {
            for byte in bytes {
                out.push_str(&format!("{byte:02x}"));
            }
            out.push('\n');
        }
    }
    out.push_str("seal ");
    out.push_str(&trace.seal_hex());
    out.push('\n');
    out
}

/// Parses a stored golden back into a trace, if this build reads it.
fn parse_stored(rendered: &str) -> Option<rastok::golden::Trace> {
    let mut records = Vec::new();
    for line in rendered.lines() {
        if line.starts_with("seal ") || line.is_empty() {
            continue;
        }
        if !line.len().is_multiple_of(2) || !line.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let bytes = line
            .as_bytes()
            .chunks_exact(2)
            .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok())
            .collect::<Option<Vec<_>>>()?;
        records.push(bytes);
    }
    rastok::golden::Trace::from_stored(&records)
}

/// A boundary envelope for a terminal: text prints bare, raw bytes
/// print lossily, anything else prints in its debug shape.
fn render_data(
    data: &Data,
    annotations: Option<&rastok::schema_annotation::AnnotationTable>,
) -> String {
    if let Some(annotations) = annotations
        && rastok::data_policy::requires_redaction(annotations)
    {
        return match data {
            Data::Materialized(value) => format!(
                "{:?}",
                rastok::data_policy::RedactedValue::new(value, annotations)
            ),
            Data::Encoded { .. } => String::from("[REDACTED]"),
        };
    }
    match data {
        Data::Materialized(Value::Text(text)) => text.trim_end_matches('\n').to_string(),
        Data::Encoded {
            encoding: rastok::envelope::Encoding::Raw,
            bytes,
            ..
        } => String::from_utf8_lossy(bytes)
            .trim_end_matches('\n')
            .to_string(),
        other => format!("{other:?}"),
    }
}
