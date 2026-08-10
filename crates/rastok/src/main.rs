// The binary wires the OS to the machine: filesystem and process
// are its business — the frontier lints stop at this file.
#![allow(clippy::disallowed_types, clippy::disallowed_methods)]
//! The `rastok` binary: `check`, `run`, `simulate`.
//!
//! One deployed unit in, evidence out. `check` compiles and prints
//! the semantic hash; `run` executes the App against the real
//! adapters, stdin lines in, boundary envelopes out; `simulate` runs
//! the unit's lab and reports every scenario. Diagnostics render in
//! the rustc shape through [`rastok::render`].

use rastok::boundary::{Draft, InboundSubmission, IngressAuthority, SubmissionSequence};
use rastok::compiler::Compilation;
use rastok::envelope::{Data, Namespace, Scalar, Value};
use rastok::journal::MemJournal;
use rastok::plan::GraphVersion;
use rastok::render::{Sources, render_all};
use rastok::runtime::vm::Vm;
use rastok::stdlib::Registry;
use rastok::{connectors, lab, node};
use std::io::{BufRead, Write as _};
use std::process::ExitCode;

const USAGE: &str = "usage: rastok <check|run> <file.rkg>...\n       rastok simulate <file.rkg>... [--golden <dir>] [--bless]";

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
        "check" => check(paths),
        "run" => run(paths),
        "simulate" => {
            let mut files = Vec::new();
            let mut golden_dir: Option<std::path::PathBuf> = None;
            let mut bless = false;
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
                    _ => files.push(argument.clone()),
                }
            }
            if bless && golden_dir.is_none() {
                eprintln!("error: --bless requires --golden <dir>");
                return ExitCode::from(2);
            }
            simulate(&files, golden_dir.as_deref(), bless)
        }
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
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
    // The App root for `asset(…)` resolution (D-2026-08-10-5): the
    // directory of the file DECLARING the `: App` graph — argument
    // order cannot move it. Fallback: the first file's directory.
    let app_index = texts
        .iter()
        .position(|text| {
            text.lines().any(|line| {
                let line = line.trim_start();
                line.starts_with("graph ") && line.contains(": App") && !line.starts_with("//")
            })
        })
        .unwrap_or(0);
    let asset_root = paths
        .get(app_index)
        .and_then(|path| std::path::Path::new(path).parent())
        .map(std::path::Path::to_path_buf);
    match rastok::compiler::compile_with_assets(&refs, &Registry::m0(), asset_root.as_deref()) {
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

/// Runs the App against the real adapters: each stdin line is one
/// text submission; boundary envelopes print to stdout, root errors
/// to stderr.
fn run(paths: &[String]) -> ExitCode {
    let Ok((compilation, _)) = load(paths) else {
        return ExitCode::FAILURE;
    };
    let version = GraphVersion(1);
    let mut journal = MemJournal::new(version);
    let mut vm = Vm::boot(&compilation.plan, version);
    let mut world = connectors::Process::new(&compilation.plan);
    let authority = match node::Path::new(["cli"]) {
        Ok(path) => path,
        Err(_) => return ExitCode::FAILURE,
    };
    let stdin = std::io::stdin();
    let mut printed_outputs = 0;
    let mut printed_errors = 0;
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(line) => line,
            Err(error) => {
                eprintln!("error: stdin: {error}");
                return ExitCode::FAILURE;
            }
        };
        let submission = InboundSubmission {
            authority: IngressAuthority::single(authority.clone()),
            // The line-fed run enters through the implicit input;
            // named ingress is the Lab's and the future servers'.
            port: node::Port::new("in").unwrap_or_else(|_| unreachable!("`in` is a valid port")),
            draft: Draft {
                data: Data::materialized(Value::Scalar(Scalar::Text(line))),
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
        };
        if let Err(error) = vm.inject(&mut journal, submission) {
            eprintln!("error: the submission was refused: {error}");
            return ExitCode::FAILURE;
        }
        loop {
            if let Err(error) = vm.drain(&mut journal) {
                eprintln!("error: {error}");
                return ExitCode::FAILURE;
            }
            match vm.pump(&mut journal, &mut world) {
                Ok(report) => {
                    for (_, reason) in &report.rejected {
                        eprintln!("err: a reply was refused before closure: {reason}");
                    }
                    if report.settled == 0 {
                        if !vm.has_open_obligations() {
                            break;
                        }
                        // Open calls against real children: the wall
                        // clock maps onto the virtual clock so
                        // declared budgets expire for real. Only the
                        // scheduler mints a timeout.
                        std::thread::sleep(std::time::Duration::from_millis(10));
                        if let Err(error) =
                            vm.advance(&mut journal, &mut world, vm.now() + 10_000_000)
                        {
                            eprintln!("error: {error}");
                            return ExitCode::FAILURE;
                        }
                    }
                }
                Err(error) => {
                    eprintln!("error: {error}");
                    return ExitCode::FAILURE;
                }
            }
        }
        if let Err(error) = vm.drain(&mut journal) {
            eprintln!("error: {error}");
            return ExitCode::FAILURE;
        }
        let stdout = std::io::stdout();
        let mut out = stdout.lock();
        for output in &vm.outputs()[printed_outputs..] {
            let _ = writeln!(
                out,
                "{}: {}",
                output.port,
                render_data(output.envelope.data())
            );
        }
        printed_outputs = vm.outputs().len();
        for envelope in &vm.errors()[printed_errors..] {
            eprintln!("err: {}", render_data(envelope.data()));
        }
        printed_errors = vm.errors().len();
    }
    ExitCode::SUCCESS
}

/// Runs every lab of the unit; one line per scenario, non-zero exit
/// on any red.
fn simulate(paths: &[String], golden_dir: Option<&std::path::Path>, bless: bool) -> ExitCode {
    let Ok((compilation, _)) = load(paths) else {
        return ExitCode::FAILURE;
    };
    if compilation.labs.is_empty() {
        eprintln!("error: this unit declares no lab");
        return ExitCode::FAILURE;
    }
    let mut failed = 0usize;
    let mut total = 0usize;
    for lab_decl in &compilation.labs {
        for run in lab::run(&compilation.plan, GraphVersion(1), lab_decl) {
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
fn render_data(data: &Data) -> String {
    match data {
        Data::Materialized(Value::Scalar(Scalar::Text(text))) => {
            text.trim_end_matches('\n').to_string()
        }
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
