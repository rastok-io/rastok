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

use rastok::boundary::{Draft, InboundEnvelopeProvenance, InboundSubmission, SubmissionSequence};
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

const USAGE: &str = "usage: rastok check <file.rkg>...\n       rastok run <file.rkg>... [--file-root <dir>] [--deployment <profile.json>] [--json-logs] [--ui [--ui-port <port>]]\n       rastok simulate <file.rkg>... [--deployment <profile.json>] [--golden <dir>] [--bless] [--ui [--ui-port <port>]]\n       rastok --resolve-open-api <project-dir> [--check]";

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
        "--resolve-open-api" => resolve_open_api(paths),
        "check" => {
            if paths
                .iter()
                .any(|argument| matches!(argument.as_str(), "--ui" | "--ui-port"))
            {
                eprintln!("error: --ui is available only for `run` and `simulate`");
                return ExitCode::from(2);
            }
            check(paths)
        }
        "run" => {
            let mut files = Vec::new();
            let mut file_root: Option<std::path::PathBuf> = None;
            let mut deployment: Option<std::path::PathBuf> = None;
            let mut json_logs = false;
            let mut ui = UiOptions::default();
            let mut rest = paths.iter();
            while let Some(argument) = rest.next() {
                match argument.as_str() {
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
            let ui = match ui.validate() {
                Ok(ui) => ui,
                Err(error) => {
                    eprintln!("error: {error}");
                    return ExitCode::from(2);
                }
            };
            run(&files, file_root.as_deref(), boot.as_ref(), json_logs, ui)
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

/// Physical UI suspension excluded from the hosted-to-logical time mapping.
///
/// The VM remains the sole owner of logical time. This adapter only prevents
/// time spent under debugger control from being presented as elapsed runtime
/// when execution resumes.
#[cfg(feature = "hosted-ui")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct UiLogicalClock {
    hosted_offset: i64,
    frozen_at: Option<i64>,
}

#[cfg(feature = "hosted-ui")]
impl UiLogicalClock {
    fn start_paused(hosted_now: i64, logical_now: i64) -> Self {
        Self {
            hosted_offset: hosted_now.saturating_sub(logical_now),
            frozen_at: Some(logical_now),
        }
    }

    fn logical_now(self, hosted_now: i64) -> i64 {
        self.frozen_at
            .unwrap_or_else(|| hosted_now.saturating_sub(self.hosted_offset))
    }

    fn pause(&mut self, hosted_now: i64) {
        if self.frozen_at.is_none() {
            self.frozen_at = Some(self.logical_now(hosted_now));
        }
    }

    fn resume(&mut self, hosted_now: i64) {
        if let Some(logical_now) = self.frozen_at.take() {
            self.hosted_offset = hosted_now.saturating_sub(logical_now);
        }
    }
}

/// Runs the App against the real Drivers: each stdin line is one
/// text submission; boundary envelopes print to stdout, root errors
/// to stderr.
fn run(
    paths: &[String],
    file_root: Option<&std::path::Path>,
    boot: Option<&rastok::deployment::BootBindings>,
    json_logs: bool,
    ui: UiOptions,
) -> ExitCode {
    let Ok((compilation, _)) = load(paths) else {
        return ExitCode::FAILURE;
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
    let mut vm = boot.map_or_else(
        || Vm::boot_at(&compilation.plan, version, time.logical_now()),
        |boot| Vm::boot_at_with_deployment(&compilation.plan, version, time.logical_now(), boot),
    );
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
    let mut printed_errors = 0;
    #[cfg(feature = "hosted-ui")]
    if ui.enabled {
        return run_with_ui(
            &compilation,
            boot,
            ui,
            &time,
            &mut vm,
            &mut journal,
            &mut world,
            &mut log_projector,
            &inbound_path,
            polls_inbound,
            &input,
            &mut printed_outputs,
            &mut printed_errors,
        );
    }
    #[cfg(not(feature = "hosted-ui"))]
    let _ = ui;
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
                eprintln!("error: {error}");
                return ExitCode::FAILURE;
            }
        }
        if let Some(projector) = &mut log_projector {
            let _ = projector.project_journal(&journal);
        }
        print_new_outputs(&vm, &mut printed_outputs, &mut printed_errors);

        if !stdin_open && !polls_inbound && !vm.has_open_obligations() {
            break;
        }
        if !progressed {
            // The next pump translates monotonic elapsed time at the one
            // hosted boundary. Timers remain deterministic logical dates and
            // Drivers never manufacture timeout outcomes themselves.
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
    ExitCode::SUCCESS
}

fn drive_boundary<'plan>(
    vm: &mut Vm<'plan>,
    journal: &mut MemJournal,
    world: &mut connectors::RuntimeBindings<'plan>,
) -> Result<bool, String> {
    let mut progressed = false;
    for _ in 0..64 {
        match world
            .pump_inbound(vm, journal)
            .map_err(|error| error.to_string())?
        {
            Some(_) => progressed = true,
            None => break,
        }
    }
    vm.drain_with_bindings(journal, world)
        .map_err(|error| error.to_string())?;
    let responses = world
        .pump_inbound_call_responses(vm)
        .map_err(|error| error.to_string())?;
    let timeouts = world
        .pump_inbound_call_timeouts(vm)
        .map_err(|error| error.to_string())?;
    let inbound_events = world
        .pump_inbound_call_events(vm)
        .map_err(|error| error.to_string())?;
    let outbound = vm.pump(journal, world).map_err(|error| error.to_string())?;
    for (_, reason) in &outbound.rejected {
        eprintln!("err: a reply was refused before closure: {reason}");
    }
    progressed |= responses.started > 0
        || timeouts.started > 0
        || inbound_events.queued > 0
        || outbound.settled > 0;
    vm.drain_with_bindings(journal, world)
        .map_err(|error| error.to_string())?;
    Ok(progressed)
}

#[cfg(feature = "hosted-ui")]
#[allow(clippy::too_many_arguments)]
fn run_with_ui<'plan>(
    compilation: &'plan Compilation,
    boot: Option<&rastok::deployment::BootBindings>,
    options: UiOptions,
    time: &HostedTimeSource,
    vm: &mut Vm<'plan>,
    journal: &mut MemJournal,
    world: &mut connectors::RuntimeBindings<'plan>,
    log_projector: &mut Option<rastok::observability::JournalLogProjector>,
    inbound_path: &node::Path,
    polls_inbound: bool,
    input: &std::sync::mpsc::Receiver<Option<std::io::Result<String>>>,
    printed_outputs: &mut usize,
    printed_errors: &mut usize,
) -> ExitCode {
    use rastok::ui::{ActivityView, ExecutionState, FaultView, UiCommand, UiServer};

    let mut state = ExecutionState::Paused;
    let mut ui_time = UiLogicalClock::start_paused(time.logical_now(), vm.now());
    let mut revision = 0_u64;
    let mut fault: Option<FaultView> = None;
    let mut activity: Option<ActivityView> = None;
    let mut boundary_ordinal = 0_u64;
    let initial = match rastok::ui::snapshot(rastok::ui::SnapshotInput {
        revision,
        mode: "run",
        state,
        plan: &compilation.plan,
        definitions: &compilation.source_definitions,
        vm,
        journal,
        cost_profile: boot.and_then(rastok::deployment::BootBindings::cost_profile),
        fault: None,
        activity: None,
        scenario: None,
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

    let mut stdin_open = true;
    let mut step_requested = false;
    let mut close_requested = false;
    loop {
        let records_before = journal.records().len();
        let observations_before = vm.observations().len();
        let mut dirty = false;
        let mut stepped_boundary = false;
        while let Some(command) = server.try_command() {
            match command {
                UiCommand::Resume | UiCommand::RunToCompletion
                    if !matches!(state, ExecutionState::Faulted | ExecutionState::Completed) =>
                {
                    ui_time.resume(time.logical_now());
                    state = ExecutionState::Running;
                    step_requested = false;
                    dirty = true;
                }
                UiCommand::Pause if state == ExecutionState::Running => {
                    state = if vm.awaiting_outbound_calls() == 0 {
                        ExecutionState::Paused
                    } else {
                        ExecutionState::PauseRequested
                    };
                    dirty = true;
                }
                UiCommand::Step if state == ExecutionState::Paused => {
                    step_requested = true;
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
                UiCommand::Close => close_requested = true,
                UiCommand::NextScenarioAction
                | UiCommand::RunUntilAssertion
                | UiCommand::ResetScenario
                | UiCommand::SelectScenario { .. }
                | UiCommand::EditNextInjection { .. }
                | UiCommand::Pause
                | UiCommand::Step
                | UiCommand::RetryStep
                | UiCommand::Resume
                | UiCommand::RunToCompletion => {}
            }
        }
        if close_requested {
            return if state == ExecutionState::Faulted {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            };
        }

        let mut progressed = false;
        let executing_one = step_requested && state == ExecutionState::Paused;
        if state == ExecutionState::Running || executing_one {
            match admit_one_run_input(vm, journal, world, inbound_path, input, &mut stdin_open) {
                Ok(admitted) => progressed |= admitted,
                Err(error) => {
                    fault = Some(FaultView {
                        message: error,
                        help: String::from(
                            "The refused admission committed nothing. Correct the input or close this run.",
                        ),
                        commit: String::from("the refused admission committed no input fact"),
                        retryable: false,
                    });
                    state = ExecutionState::Faulted;
                    step_requested = false;
                    dirty = true;
                }
            }
            if state != ExecutionState::Faulted {
                match vm.step_with_bindings(journal, world) {
                    Ok(ran) => {
                        progressed |= ran;
                        dirty |= ran;
                        stepped_boundary |= ran;
                        if !ran && state == ExecutionState::Running {
                            let target = ui_time.logical_now(time.logical_now());
                            if target > vm.now() {
                                match vm.advance(journal, world, target) {
                                    Ok(()) => {}
                                    Err(error) => {
                                        fault = Some(boundary_fault(error.to_string()));
                                        state = ExecutionState::Faulted;
                                    }
                                }
                            }
                        }
                    }
                    Err(error) => {
                        fault = Some(transaction_fault(error.to_string()));
                        state = ExecutionState::Faulted;
                        dirty = true;
                    }
                }
            }
            if state != ExecutionState::Faulted {
                match pump_run_boundaries(vm, journal, world, state == ExecutionState::Running) {
                    Ok(boundary) => {
                        progressed |= boundary;
                        dirty |= boundary;
                    }
                    Err(error) => {
                        fault = Some(boundary_fault(error));
                        state = ExecutionState::Faulted;
                        dirty = true;
                    }
                }
            }
            if executing_one && state != ExecutionState::Faulted {
                step_requested = false;
                state = if vm.awaiting_outbound_calls() == 0 {
                    ExecutionState::Paused
                } else {
                    ExecutionState::PauseRequested
                };
                dirty = true;
            }
        } else if matches!(
            state,
            ExecutionState::Paused | ExecutionState::PauseRequested
        ) {
            // A durable open must be offered to its Driver, and Session
            // transports retain their bounded contract, even while scheduling
            // is paused. No completion transaction runs here.
            match vm.pump(journal, world) {
                Ok(report) => {
                    if report.settled > 0
                        || !report.rejected.is_empty()
                        || !report.rejected_sessions.is_empty()
                    {
                        dirty = true;
                        progressed = true;
                    }
                    for (_, reason) in &report.rejected {
                        eprintln!("err: a reply was refused before closure: {reason}");
                    }
                }
                Err(error) => {
                    fault = Some(boundary_fault(error.to_string()));
                    state = ExecutionState::Faulted;
                    dirty = true;
                }
            }
            if state == ExecutionState::PauseRequested && vm.awaiting_outbound_calls() == 0 {
                state = ExecutionState::Paused;
                dirty = true;
            }
        }

        if state == ExecutionState::Running
            && !stdin_open
            && !polls_inbound
            && !vm.has_open_obligations()
            && !progressed
        {
            state = ExecutionState::Completed;
            dirty = true;
        }
        if state != ExecutionState::Running {
            ui_time.pause(time.logical_now());
        }
        if let Some(projector) = log_projector {
            let batch = projector.project_journal(journal);
            dirty |= batch.queued > 0 || batch.dropped > 0;
        }
        let before_outputs = *printed_outputs;
        let before_errors = *printed_errors;
        print_new_outputs(vm, printed_outputs, printed_errors);
        dirty |= before_outputs != *printed_outputs || before_errors != *printed_errors;
        dirty |= records_before != journal.records().len()
            || observations_before != vm.observations().len();
        if records_before != journal.records().len() {
            let ordinal = stepped_boundary.then(|| {
                boundary_ordinal = boundary_ordinal.saturating_add(1);
                boundary_ordinal
            });
            activity = ActivityView::journal_change(journal, records_before, ordinal);
        }

        if dirty {
            revision = revision.saturating_add(1);
            match rastok::ui::snapshot(rastok::ui::SnapshotInput {
                revision,
                mode: "run",
                state,
                plan: &compilation.plan,
                definitions: &compilation.source_definitions,
                vm,
                journal,
                cost_profile: boot.and_then(rastok::deployment::BootBindings::cost_profile),
                fault: fault.as_ref(),
                activity: activity.as_ref(),
                scenario: None,
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

#[cfg(feature = "hosted-ui")]
fn admit_one_run_input<'plan>(
    vm: &mut Vm<'plan>,
    journal: &mut MemJournal,
    world: &mut connectors::RuntimeBindings<'plan>,
    inbound_path: &node::Path,
    input: &std::sync::mpsc::Receiver<Option<std::io::Result<String>>>,
    stdin_open: &mut bool,
) -> Result<bool, String> {
    if *stdin_open {
        match input.try_recv() {
            Ok(Some(Ok(line))) => {
                vm.inject(journal, cli_submission(inbound_path, line))
                    .map_err(|error| format!("the submission was refused: {error}"))?;
                return Ok(true);
            }
            Ok(Some(Err(error))) => return Err(format!("stdin: {error}")),
            Ok(None) => *stdin_open = false,
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => *stdin_open = false,
        }
    }
    world
        .pump_inbound(vm, journal)
        .map(|report| report.is_some())
        .map_err(|error| error.to_string())
}

#[cfg(feature = "hosted-ui")]
fn pump_run_boundaries<'plan>(
    vm: &mut Vm<'plan>,
    journal: &mut MemJournal,
    world: &mut connectors::RuntimeBindings<'plan>,
    poll_inbound_protocols: bool,
) -> Result<bool, String> {
    let mut progressed = false;
    if poll_inbound_protocols {
        let responses = world
            .pump_inbound_call_responses(vm)
            .map_err(|error| error.to_string())?;
        let timeouts = world
            .pump_inbound_call_timeouts(vm)
            .map_err(|error| error.to_string())?;
        let events = world
            .pump_inbound_call_events(vm)
            .map_err(|error| error.to_string())?;
        progressed |= responses.started > 0 || timeouts.started > 0 || events.queued > 0;
    }
    let report = vm.pump(journal, world).map_err(|error| error.to_string())?;
    for (_, reason) in &report.rejected {
        eprintln!("err: a reply was refused before closure: {reason}");
    }
    progressed |=
        report.settled > 0 || !report.rejected.is_empty() || !report.rejected_sessions.is_empty();
    Ok(progressed)
}

fn cli_submission(inbound_path: &node::Path, line: String) -> InboundSubmission {
    InboundSubmission {
        provenance: InboundEnvelopeProvenance::single(inbound_path.clone()),
        port: node::Port::new("in").unwrap_or_else(|_| unreachable!("`in` is a valid port")),
        draft: Draft {
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
        sequence: SubmissionSequence::Next,
    }
}

fn print_new_outputs(vm: &Vm<'_>, printed_outputs: &mut usize, printed_errors: &mut usize) {
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
    let err_annotations = node::Port::new("err")
        .ok()
        .and_then(|port| vm.boundary_annotations(&port));
    for envelope in &vm.errors()[*printed_errors..] {
        eprintln!("err: {}", render_data(envelope.data(), err_annotations));
    }
    *printed_errors = vm.errors().len();
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
        let runs = boot.map_or_else(
            || lab::run(&compilation.plan, GraphVersion(1), lab_decl),
            |boot| lab::run_with_boot(&compilation.plan, GraphVersion(1), lab_decl, boot),
        );
        for run in runs {
            let report = &run.report;
            total += 1;
            // The golden seam: a completed run projects to its
            // canonical trace, compared against — or blessing — the
            // stored one.
            let mut golden_failures = Vec::new();
            if let Some(dir) = golden_dir {
                let trace = rastok::golden::project(&run.records);
                let rendered = render_trace(&trace);
                let path = dir.join(format!("{}.{}.trace", report.simulation, report.scenario));
                if bless {
                    if let Err(error) =
                        std::fs::create_dir_all(dir).and_then(|()| std::fs::write(&path, &rendered))
                    {
                        eprintln!("error: cannot bless `{}`: {error}", path.display());
                        return ExitCode::FAILURE;
                    }
                } else {
                    match std::fs::read_to_string(&path) {
                        Err(_) => golden_failures.push(format!(
                            "no golden at `{}`; bless with --bless",
                            path.display()
                        )),
                        Ok(stored) if stored != rendered => {
                            let divergence = parse_stored(&stored)
                                .and_then(|stored| rastok::golden::diff(&trace, &stored));
                            golden_failures.push(match divergence {
                                Some(divergence) => format!("golden divergence: {divergence}"),
                                None => {
                                    String::from("the stored golden is unreadable by this build")
                                }
                            });
                        }
                        Ok(_) => {}
                    }
                }
            }
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
    let mut session = match lab::ScenarioSession::try_new(
        &compilation.plan,
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
    let initial = match rastok::ui::snapshot(rastok::ui::SnapshotInput {
        revision,
        mode: "simulate",
        state,
        plan: &compilation.plan,
        definitions: &compilation.source_definitions,
        vm: session.vm(),
        journal: session.journal(),
        cost_profile: boot.and_then(rastok::deployment::BootBindings::cost_profile),
        fault: None,
        activity: None,
        scenario: Some(simulation_view(compilation, &session, &[])),
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
    let mut close_requested = false;
    let mut reported = false;
    let mut any_failed = false;
    let mut golden_failures = Vec::new();
    loop {
        let records_before = session.journal().records().len();
        let observations_before = session.vm().observations().len();
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
                    step_requested = false;
                    fault = None;
                    dirty = true;
                }
                UiCommand::RunUntilAssertion
                    if !matches!(state, ExecutionState::Faulted | ExecutionState::Completed) =>
                {
                    state = ExecutionState::Running;
                    until_assertion = true;
                    step_requested = false;
                    fault = None;
                    dirty = true;
                }
                UiCommand::Pause if state == ExecutionState::Running => {
                    state = if session.vm().awaiting_outbound_calls() == 0 {
                        ExecutionState::Paused
                    } else {
                        ExecutionState::PauseRequested
                    };
                    until_assertion = false;
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
                                    "Use Step until the current action reaches its next transaction boundary.",
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
                            state = ExecutionState::Paused;
                            fault = None;
                            activity = None;
                            boundary_ordinal = 0;
                            activity_decided = true;
                            reported = false;
                            any_failed = false;
                            golden_failures.clear();
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
                            state = ExecutionState::Paused;
                            fault = None;
                            activity = None;
                            boundary_ordinal = 0;
                            activity_decided = true;
                            reported = false;
                            any_failed = false;
                            golden_failures.clear();
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
                UiCommand::EditNextInjection { data }
                    if matches!(state, ExecutionState::Paused | ExecutionState::Faulted) =>
                {
                    let edited = session
                        .next_injection_draft()
                        .and_then(|draft| {
                            let draft = draft.ok_or_else(|| {
                                String::from("the next scenario action is not an injection")
                            })?;
                            rastok::ui::parse_edited_data(&data, &draft.ty)
                        })
                        .and_then(|value| session.replace_next_injection_data(value));
                    match edited {
                        Ok(()) => {
                            fault = None;
                            state = ExecutionState::Paused;
                        }
                        Err(message) => {
                            fault = Some(FaultView {
                                message,
                                help: String::from(
                                    "Correct the typed DATA tree or reset the scenario to restore its authored input.",
                                ),
                                commit: String::from(
                                    "the draft was not changed and no envelope was admitted",
                                ),
                                retryable: false,
                            });
                            state = ExecutionState::Faulted;
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
                | UiCommand::EditNextInjection { .. }
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

        if step_requested && state == ExecutionState::Paused {
            let transaction_records_before = session.journal().records().len();
            match session.step_transaction() {
                Ok(ran) => {
                    if ran {
                        boundary_ordinal = boundary_ordinal.saturating_add(1);
                        activity = ActivityView::journal_change(
                            session.journal(),
                            transaction_records_before,
                            Some(boundary_ordinal),
                        );
                        activity_decided = true;
                    }
                    state = if session.vm().awaiting_outbound_calls() == 0 {
                        ExecutionState::Paused
                    } else {
                        ExecutionState::PauseRequested
                    };
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
            step_requested = false;
            dirty = true;
        } else if state == ExecutionState::PauseRequested {
            match session.poll_world() {
                Ok(progressed) => dirty |= progressed,
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
        } else if state == ExecutionState::Paused {
            // Scripted Session transports retain their normal bounded
            // boundary behavior while the scheduler is paused. Polling may
            // retain a terminal observation, but cannot consume its handler.
            match session.poll_world() {
                Ok(progressed) => dirty |= progressed,
                Err(error) => {
                    fault = Some(boundary_fault(error));
                    state = ExecutionState::Faulted;
                    dirty = true;
                }
            }
        } else if state == ExecutionState::Running {
            if until_assertion
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
                        if ran {
                            boundary_ordinal = boundary_ordinal.saturating_add(1);
                            activity = ActivityView::journal_change(
                                session.journal(),
                                transaction_records_before,
                                Some(boundary_ordinal),
                            );
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
            golden_failures =
                check_or_bless_golden(&report, session.journal().records(), golden_dir, bless);
            let passed = report.passed() && golden_failures.is_empty();
            any_failed |= !passed;
            print_scenario_verdict(&report, &golden_failures);
            reported = true;
            dirty = true;
        }
        dirty |= records_before != session.journal().records().len()
            || observations_before != session.vm().observations().len()
            || failures_before != session.failures().len();
        if records_before != session.journal().records().len() && !activity_decided {
            activity = ActivityView::journal_change(session.journal(), records_before, None);
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
                cost_profile: boot.and_then(rastok::deployment::BootBindings::cost_profile),
                fault: fault.as_ref(),
                activity: activity.as_ref(),
                scenario: Some(simulation_view(compilation, &session, &golden_failures)),
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
) -> Result<lab::ScenarioSession<'plan, 'plan>, String> {
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
    lab::ScenarioSession::try_new(
        &compilation.plan,
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
    session: &lab::ScenarioSession<'_, '_>,
    golden_failures: &[String],
) -> serde_json::Value {
    let draft = match session.next_injection_draft() {
        Ok(Some(draft)) => rastok::ui::scenario_injection_json(&draft),
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
            "label": session.label(),
            "next_action_index": session.next_action_index(),
            "action_count": session.action_count(),
            "next_action": session.next_action_kind().map(scenario_action_name),
            "last_action": session.last_action_kind().map(scenario_action_name),
            "quiescent": session.is_quiescent(),
            "complete": session.is_complete(),
            "draft": draft,
            "failures": session.failures(),
            "golden_failures": golden_failures,
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

#[cfg(feature = "hosted-ui")]
fn check_or_bless_golden(
    report: &lab::ScenarioReport,
    records: &[rastok::journal::Record],
    golden_dir: Option<&std::path::Path>,
    bless: bool,
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
            let divergence =
                parse_stored(&stored).and_then(|stored| rastok::golden::diff(&trace, &stored));
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
        let mut bytes = Vec::new();
        let mut at = 0;
        while at + 2 <= line.len() {
            bytes.push(u8::from_str_radix(&line[at..at + 2], 16).ok()?);
            at += 2;
        }
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

#[cfg(all(test, feature = "hosted-ui"))]
mod ui_time_tests {
    use super::UiLogicalClock;

    #[test]
    fn debugger_suspension_never_becomes_logical_elapsed_time() {
        let mut clock = UiLogicalClock::start_paused(100, 90);
        assert_eq!(clock.logical_now(400), 90);

        clock.resume(400);
        assert_eq!(clock.logical_now(400), 90);
        assert_eq!(clock.logical_now(450), 140);

        clock.pause(450);
        assert_eq!(clock.logical_now(900), 140);

        clock.resume(900);
        assert_eq!(clock.logical_now(930), 170);
    }
}
