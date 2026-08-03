//! The `rastok` binary: `check`, `run`, `simulate`.
//!
//! One deployed unit in, evidence out. `check` compiles and prints
//! the semantic hash; `run` executes the App against the real
//! adapters, stdin lines in, boundary envelopes out; `simulate` runs
//! the unit's lab and reports every scenario. Diagnostics render in
//! the rustc shape through [`rastok::render`].

use rastok::boundary::{Draft, InboundSubmission, IngressAuthority, SubmissionSequence};
use rastok::compiler::{Compilation, compile};
use rastok::envelope::{Data, Namespace, Scalar, Value};
use rastok::journal::MemJournal;
use rastok::plan::GraphVersion;
use rastok::render::{Sources, render_all};
use rastok::runtime::vm::Vm;
use rastok::stdlib::Registry;
use rastok::{connectors, lab, node};
use std::io::{BufRead, Write as _};
use std::process::ExitCode;

const USAGE: &str = "usage: rastok <check|run|simulate> <file.rkg>...";

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
        "simulate" => simulate(paths),
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
    match compile(&refs, &Registry::m0()) {
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
            draft: Draft {
                data: Data::materialized(Value::Scalar(Scalar::Text(line))),
                mime_type: Some(String::from("text/plain")),
                app: Namespace::new(),
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
                Ok(report) if report.settled == 0 => break,
                Ok(_) => {}
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
        for envelope in &vm.outputs()[printed_outputs..] {
            let _ = writeln!(out, "{}", render_data(envelope.data()));
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
fn simulate(paths: &[String]) -> ExitCode {
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
            let verdict = if report.passed() { "PASS" } else { "FAIL" };
            match &report.label {
                Some(label) => {
                    println!(
                        "{verdict} {}.{} — {label}",
                        report.simulation, report.scenario
                    );
                }
                None => println!("{verdict} {}.{}", report.simulation, report.scenario),
            }
            if !report.passed() {
                failed += 1;
                for failure in &report.failures {
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
