//! Compile the C3 memory fixture through the staged checker and validator.
//! The public compiler remains fail-closed until the runtime is qualified.
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]

use rastok::{
    compiler,
    connector::contract::ContractCatalog,
    diagnostics::{Bag, SourceId},
    plan,
};
use std::{io::Write, path::PathBuf};

fn main() -> Result<(), String> {
    let mut args = std::env::args_os().skip(1);
    let path = PathBuf::from(args.next().ok_or("expected a new output path")?);
    let attempts: u32 = args
        .next()
        .ok_or("expected total attempts")?
        .to_str()
        .ok_or("invalid attempts")?
        .parse()
        .map_err(|_| "invalid attempts")?;
    if args.next().is_some() || attempts == 0 {
        return Err("usage: retry_spike_package <new-package-path> <positive-attempts>".into());
    }
    if path.exists() {
        return Err("the candidate path already exists; never overwrite a fixture".into());
    }
    let source = format!(
        r#"
graph app : App {{
  input in: text
  connector call : Process + Retry {{
    command = "cat"; codec = lines; behavior = deterministic; idempotent = true
    max_output = "64KiB"; timeout = duration("1s")
    Retry.attempts = {attempts}; Retry.on = [TimeoutExpired]
    Retry.delay = {{ kind = fixed; after = duration("1ms") }}
  }}
  in -> call
  call.out -> nothing; call.err -> nothing; call.exit -> nothing
}}
"#
    );
    let catalog = ContractCatalog::try_m0().map_err(|e| e.to_string())?;
    let id = SourceId::new(0);
    let mut diagnostics = Bag::new();
    let file = compiler::parser::parse(id, &source, &catalog, &mut diagnostics);
    let hir = compiler::check::check(
        &compiler::Unit {
            files: vec![(id, file)],
        },
        &catalog,
        &mut diagnostics,
        None,
    )
    .ok_or_else(|| format!("{diagnostics:?}"))?;
    if diagnostics.has_errors() {
        return Err(format!("{diagnostics:?}"));
    }
    let hash = compiler::lower::semantic_hash(&hir);
    let (raw, _) = compiler::lower::lower(&hir).map_err(|e| format!("{e:?}"))?;
    let validated = plan::ValidatedExecutionPlan::try_new(raw).map_err(|e| e.to_string())?;
    let bytes = plan::encode(&validated, &hash);
    if plan::decode(&bytes).map_err(|e| e.to_string())? != (validated, hash) {
        return Err("candidate package does not round-trip".into());
    }
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .and_then(|mut file| file.write_all(&bytes))
        .map_err(|e| e.to_string())?;
    println!("{} · {attempts} attempts · {hash}", path.display());
    Ok(())
}
