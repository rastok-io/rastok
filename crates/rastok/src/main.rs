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

use rastok::compiler::Compilation;
use rastok::connector::contract::ContractCatalog;
use rastok::connector::exchange::EnvelopeDraft;
use rastok::connector::exchange::IngressProposal;
use rastok::connector::exchange::IngressProvenance;
use rastok::connector::exchange::IngressSequence;
use rastok::envelope::{Data, Namespace, Value};
use rastok::journal::MemJournal;
use rastok::plan::GraphVersion;
use rastok::render::{Sources, render_all};
use rastok::runtime::execution::Execution;
use rastok::{connector, lab, node};
use std::io::{BufRead, Write as _};
use std::process::ExitCode;

mod connector_catalogue;

const USAGE: &str = "usage: rastok new <name>\n       rastok connectors [--json]\n       rastok check <file.rkg>...\n       rastok run <file.rkg>... [--file-root <dir>] [--deployment <profile.json>] [--json-logs] [--record <file.rkrec>]\n       rastok replay <file.rkrec> [--ui [--ui-port <port>]]\n       rastok simulate <file.rkg>... [--deployment <profile.json>] [--compare <dir> | --save <dir>] [--ui [--ui-port <port>]]\n       rastok --resolve-open-api <project-dir> [--check]\n       rastok --help | -h | help\n       rastok --version";

/// Comparing is read-only; saving explicitly creates or replaces reference traces.
enum GoldenAction {
    Compare(std::path::PathBuf),
    Save(std::path::PathBuf),
}

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

/// Parses the command line and dispatches to one sub-command. Explicit help
/// and version requests succeed on stdout. Usage errors exit with 2;
/// a refused unit or a failed run exits with 1.
fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (command, paths) = match args.split_first() {
        Some((command, [])) if matches!(command.as_str(), "--help" | "-h" | "help") => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Some((command, [])) if command == "--version" => {
            println!("rastok {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Some((command, paths)) if !paths.is_empty() || command == "connectors" => {
            (command.as_str(), paths)
        }
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    match command {
        "new" => new_graph(paths),
        "connectors" => connector_catalogue::run(paths),
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
            let mut golden = None;
            let mut deployment: Option<std::path::PathBuf> = None;
            let mut ui = UiOptions::default();
            let mut rest = paths.iter();
            while let Some(argument) = rest.next() {
                match argument.as_str() {
                    "--compare" | "--save" => {
                        if golden.is_some() {
                            eprintln!(
                                "error: choose exactly one of --compare <dir> or --save <dir>"
                            );
                            return ExitCode::from(2);
                        }
                        let Some(dir) = rest
                            .next()
                            .filter(|dir| !dir.is_empty() && !dir.starts_with('-'))
                        else {
                            eprintln!("error: {argument} requires a directory\n{USAGE}");
                            return ExitCode::from(2);
                        };
                        golden = Some(if argument == "--compare" {
                            GoldenAction::Compare(dir.into())
                        } else {
                            GoldenAction::Save(dir.into())
                        });
                    }
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
                    option if option.starts_with('-') => {
                        eprintln!("error: unknown simulate option `{option}`\n{USAGE}");
                        return ExitCode::from(2);
                    }
                    _ => files.push(argument.clone()),
                }
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
            simulate(
                &files,
                golden.as_ref(),
                boot.as_ref()
                    .map(rastok::deployment_config::DeploymentConfig::boot),
                ui,
            )
        }
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

/// Writes the starter graph and its Lab to `<name>.rkg`, never over an
/// existing file.
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

/// Creates or updates `openapi.generated.rkg` from the OpenAPI documents of
/// a project directory; `--check` only reports whether it is current.
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

/// Compiles the unit and prints its Node count and semantic hash.
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
/// to the Execution's explicit logical timeline. Nothing below this boundary reads
/// the OS clock, and wall-clock adjustments cannot move a running Execution back.
struct HostedTimeSource {
    origin: rastok::time::Timestamp,
    started: std::time::Instant,
}

impl HostedTimeSource {
    fn start() -> Result<Self, String> {
        let since_epoch = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| String::from("the hosted wall clock is before the Unix epoch"))?;
        let seconds = i64::try_from(since_epoch.as_secs()).map_err(|_| {
            String::from("the hosted wall clock does not fit Rastok's timestamp range")
        })?;
        let origin = rastok::time::Timestamp::try_new(seconds, since_epoch.subsec_nanos())
            .map_err(|_| String::from("the hosted wall clock has an invalid fractional second"))?;
        Ok(Self {
            origin,
            started: std::time::Instant::now(),
        })
    }

    fn logical_now(&self) -> rastok::time::LogicalTime {
        let elapsed = i64::try_from(self.started.elapsed().as_nanos()).unwrap_or(i64::MAX);
        rastok::time::LogicalTime::from_nanoseconds(elapsed)
    }
}

/// OS input and presentation for the shared Worker. The Worker owns the
/// processing loop; this adapter supplies the existing clock, stdin channel,
/// recording order and idle-wait policy.
struct CliSupervision<'a> {
    time: HostedTimeSource,
    input: std::sync::mpsc::Receiver<Option<std::io::Result<String>>>,
    inbound_path: node::Path,
    recorder: &'a mut Option<rastok::recording::Recorder>,
    log_projector: &'a mut Option<rastok::observability::JournalLogProjector>,
    printed_outputs: usize,
}

impl rastok::worker::Supervision for CliSupervision<'_> {
    type Error = String;

    fn logical_time(&mut self) -> rastok::time::LogicalTime {
        self.time.logical_now()
    }

    fn poll_input(&mut self) -> Result<std::task::Poll<Option<IngressProposal>>, Self::Error> {
        match self.input.try_recv() {
            Ok(Some(Ok(line))) => Ok(std::task::Poll::Ready(Some(cli_submission(
                &self.inbound_path,
                line,
            )))),
            Ok(Some(Err(error))) => Err(format!("stdin: {error}")),
            Ok(None) => Ok(std::task::Poll::Ready(None)),
            Err(_) => Ok(std::task::Poll::Pending),
        }
    }

    fn observe_poll(&mut self, report: &rastok::runtime::execution::PollReport) {
        for (_, reason) in &report.rejected {
            eprintln!("err: a reply was refused before closure: {reason}");
        }
    }

    fn checkpoint(
        &mut self,
        execution: &Execution<'_>,
        journal: &MemJournal,
    ) -> Result<(), String> {
        recording_checkpoint(self.recorder, journal, execution.logical_time())
            .map_err(|error| error.to_string())?;
        if let Some(projector) = self.log_projector {
            let _ = projector.project_journal(journal);
        }
        print_new_outputs(execution, &mut self.printed_outputs);
        Ok(())
    }

    fn wait(&mut self) {
        // The next poll supplies monotonic elapsed time explicitly. Deadlines
        // remain logical dates; Drivers do not manufacture timeout outcomes.
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

/// Runs the App against the real Drivers: each stdin line is one
/// text submission; public boundary envelopes print to stdout.
fn run(
    paths: &[String],
    file_root: Option<&std::path::Path>,
    configuration: Option<&rastok::deployment_config::DeploymentConfig>,
    json_logs: bool,
    record: Option<&std::path::Path>,
) -> ExitCode {
    let Ok((compilation, _)) = load(paths) else {
        return ExitCode::FAILURE;
    };
    let boot = configuration.map(rastok::deployment_config::DeploymentConfig::boot);
    let time = match HostedTimeSource::start() {
        Ok(time) => time,
        Err(error) => {
            eprintln!("error: {error}");
            return ExitCode::FAILURE;
        }
    };
    let mut recorder = match record
        .map(|path| {
            rastok::recording::Recorder::create(
                path,
                &compilation,
                GraphVersion(1),
                time.origin,
                boot,
            )
        })
        .transpose()
    {
        Ok(recorder) => recorder,
        Err(error) => {
            eprintln!("error: {error}");
            return ExitCode::FAILURE;
        }
    };
    let registry = match connector::drivers::hosted_registry(file_root) {
        Ok(registry) => registry,
        Err(error) => {
            eprintln!("error: {error}");
            return ExitCode::FAILURE;
        }
    };
    let drivers = match configuration {
        Some(configuration) => connector::drivers::RuntimeDrivers::prepare_with_selection(
            &compilation.plan,
            &registry,
            configuration.boot(),
            configuration.drivers(),
        ),
        None => connector::drivers::RuntimeDrivers::prepare(&compilation.plan, &registry),
    };
    let world = match drivers {
        Ok(drivers) => drivers,
        Err(error) => {
            eprintln!("error: {error}");
            return ExitCode::FAILURE;
        }
    };
    let version = GraphVersion(1);
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
    let mut worker = rastok::worker::Worker::new(
        &compilation.plan,
        Some(compilation.hash),
        version,
        time.origin,
        boot,
        world,
    );
    if let Err(error) = recording_checkpoint(
        &mut recorder,
        worker.journal(),
        worker.execution().logical_time(),
    ) {
        eprintln!("error: {error}");
        return ExitCode::FAILURE;
    }
    let inbound_path = match node::Path::new(["cli"]) {
        Ok(path) => path,
        Err(_) => return ExitCode::FAILURE,
    };
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
    let result = worker.run(&mut CliSupervision {
        time,
        input,
        inbound_path,
        recorder: &mut recorder,
        log_projector: &mut log_projector,
        printed_outputs: 0,
    });
    if let Err(error) = result {
        if matches!(error, rastok::worker::RunError::Processing(_))
            && let Err(record_error) = recording_checkpoint(
                &mut recorder,
                worker.journal(),
                worker.execution().logical_time(),
            )
        {
            eprintln!("error: {record_error}");
        }
        eprintln!("error: {error}");
        return ExitCode::FAILURE;
    }
    let logical_time = worker.execution().logical_time();
    let journal = worker.shutdown();
    if let Some(recorder) = recorder {
        match recorder.finish(&compilation, &journal, logical_time) {
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

/// Appends the journal's new records to the diagnostic recording, when one
/// was requested with `--record`.
fn recording_checkpoint(
    recorder: &mut Option<rastok::recording::Recorder>,
    journal: &MemJournal,
    now: rastok::time::LogicalTime,
) -> Result<(), rastok::recording::Error> {
    if let Some(recorder) = recorder {
        recorder.checkpoint(journal, now)?;
    }
    Ok(())
}

/// Reconstructs a recorded run from its `.rkrec` file without any Driver
/// I/O, then prints its boundary outputs or serves the read-only debugger.
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
        Ok((_, execution)) => {
            print_new_outputs(&execution, &mut 0);
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

/// The debugger's description of a transaction fault: nothing was committed
/// and the step may be retried.
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

/// The debugger's description of a physical boundary failure: earlier
/// transactions stay committed and the run is not retried.
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

/// Wraps one stdin line as a `text/plain` ingress proposal on the App's
/// `in` port.
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

/// Prints the boundary outputs produced since the previous call, one line
/// per envelope as `port: data`.
fn print_new_outputs(execution: &Execution<'_>, printed_outputs: &mut usize) {
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    for output in &execution.outputs()[*printed_outputs..] {
        let _ = writeln!(
            out,
            "{}: {}",
            output.port,
            render_data(
                output.envelope.data(),
                execution.boundary_annotations(&output.port),
            )
        );
    }
    *printed_outputs = execution.outputs().len();
}

/// Runs every lab of the unit; one line per scenario, non-zero exit
/// on any red.
fn simulate(
    paths: &[String],
    golden: Option<&GoldenAction>,
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
        return simulate_with_ui(&compilation, golden, boot, ui);
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
                compare_or_save_golden(report, &run.records, golden, &compilation.plan);
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

/// Serves the paused simulation debugger for the unit's first scenario and
/// applies its commands (step, next action, run, replay, reset, scenario
/// selection) to one `Simulator` at a time.
#[cfg(feature = "hosted-ui")]
fn simulate_with_ui(
    compilation: &Compilation,
    golden: Option<&GoldenAction>,
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
        execution: session.execution(),
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
        let observations_before = session.execution().envelope_views().len();
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
                        || session.execution().awaiting_outbound_calls() == 0
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
                && session.execution().awaiting_outbound_calls() == 0
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
            golden_failures = compare_or_save_golden(
                &report,
                session.journal().records(),
                golden,
                &compilation.plan,
            );
            let passed = report.passed() && golden_failures.is_empty();
            any_failed |= !passed;
            print_scenario_verdict(&report, &golden_failures);
            reported = true;
            dirty = true;
        }
        dirty |= records_before != session.journal().records().len()
            || observations_before != session.execution().envelope_views().len()
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
                execution: session.execution(),
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

/// Builds a fresh `Simulator` for a named simulation and scenario of the
/// unit, as the debugger's reset and scenario selection require.
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

/// The scenario part of a debugger snapshot: available simulations, the
/// current scenario's progress, next injection, failures and expectation.
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

/// The JSON name of one authored scenario action kind.
#[cfg(feature = "hosted-ui")]
const fn scenario_action_name(action: lab::ScenarioActionKind) -> &'static str {
    match action {
        lab::ScenarioActionKind::Inject => "inject",
        lab::ScenarioActionKind::Advance => "advance",
        lab::ScenarioActionKind::Expect => "expect",
    }
}

/// Compares the scenario's causal trace with its stored golden, or saves it
/// as the new golden; returns the failures to report, if any.
fn compare_or_save_golden(
    report: &lab::ScenarioReport,
    records: &[rastok::journal::Record],
    golden: Option<&GoldenAction>,
    plan: &rastok::plan::ValidatedExecutionPlan,
) -> Vec<String> {
    let Some(action) = golden else {
        return Vec::new();
    };
    let dir = match action {
        GoldenAction::Compare(dir) | GoldenAction::Save(dir) => dir,
    };
    let trace = rastok::golden::project(records);
    let rendered = render_trace(&trace);
    let path = dir.join(format!("{}.{}.trace", report.simulation, report.scenario));
    if matches!(action, GoldenAction::Save(_)) {
        return std::fs::create_dir_all(dir)
            .and_then(|()| std::fs::write(&path, &rendered))
            .err()
            .map(|error| vec![format!("cannot save `{}`: {error}", path.display())])
            .unwrap_or_default();
    }
    match std::fs::read_to_string(&path) {
        Err(_) => vec![format!(
            "no golden at `{}`; save references with --save <dir>",
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

/// Prints one PASS or FAIL line for a scenario, then its failures.
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
