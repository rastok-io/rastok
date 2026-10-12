//! Audit candidate packages before approving fixtures. This does not modify a
//! golden: optional output goes only to an explicitly supplied directory.
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]

use rastok::{
    canonical_cbor::{self, Value as Cbor},
    compiler,
    contract::ContractCatalog,
    plan,
};
use std::path::Path;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn unhex(text: &str) -> Result<Vec<u8>, String> {
    let text = text.trim();
    if !text.len().is_multiple_of(2) {
        return Err("odd hex length".into());
    }
    text.as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let pair = std::str::from_utf8(pair).map_err(|error| error.to_string())?;
            u8::from_str_radix(pair, 16).map_err(|error| error.to_string())
        })
        .collect()
}

fn describe(name: &str, side: &str, bytes: &[u8]) -> Result<Vec<u8>, String> {
    let body = bytes.strip_prefix(&plan::MAGIC).ok_or("bad magic")?;
    let (Cbor::Map(fields), used) =
        canonical_cbor::decode(body).map_err(|error| error.to_string())?
    else {
        return Err("package header is not a map".into());
    };
    if used != body.len() {
        return Err("trailing package bytes".into());
    }
    let Some(Cbor::Uint(version)) = fields.get(&1) else {
        return Err("missing version".into());
    };
    let Some(Cbor::Bytes(hash)) = fields.get(&2) else {
        return Err("missing hash".into());
    };
    let Some(Cbor::Bytes(body)) = fields.get(&3) else {
        return Err("missing body".into());
    };
    let Some(Cbor::Bytes(checksum)) = fields.get(&4) else {
        return Err("missing checksum".into());
    };
    println!(
        "{name} {side}: v{version}; body={} B; hash={}; checksum={}",
        body.len(),
        hex(hash),
        hex(checksum)
    );
    Ok(body.clone())
}

fn main() -> Result<(), String> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    if std::env::args_os()
        .nth(1)
        .is_some_and(|arg| arg == "--gallery")
    {
        return gallery(manifest);
    }
    let candidate_dir = std::env::args_os().nth(1);
    if let Some(directory) = &candidate_dir
        && !Path::new(directory).is_dir()
    {
        return Err("candidate directory must already exist".into());
    }
    let catalog = ContractCatalog::try_m0().map_err(|error| error.to_string())?;
    for name in ["echo", "jsonl", "gauge"] {
        let source = std::fs::read_to_string(manifest.join(format!("../../examples/{name}.rkg")))
            .map_err(|error| error.to_string())?;
        let compiled =
            compiler::compile(&[&source], &catalog).map_err(|error| format!("{error:?}"))?;
        let candidate = plan::encode(&compiled.plan, &compiled.hash);
        let decoded = plan::decode(&candidate).map_err(|error| error.to_string())?;
        if decoded != (compiled.plan.clone(), compiled.hash) {
            return Err("candidate does not round-trip".into());
        }
        let stored = std::fs::read_to_string(manifest.join(format!("tests/golden/{name}.rkp")))
            .map_err(|error| error.to_string())?;
        let stored_body = describe(name, "stored", &unhex(&stored)?)?;
        let candidate_body = describe(name, "candidate", &candidate)?;
        println!(
            "{name} body byte-identical: {}",
            stored_body == candidate_body
        );
        for lab in &compiled.labs {
            for run in rastok::lab::run(&compiled.plan, plan::GraphVersion(1), lab) {
                if !run.report.passed() {
                    return Err(format!("{name}: {:?}", run.report));
                }
                let trace = rastok::golden::project(&run.records);
                println!(
                    "{name} trace: {} records; seal={}",
                    trace.len(),
                    trace.seal_hex()
                );
            }
        }
        if let Some(directory) = &candidate_dir {
            std::fs::write(Path::new(directory).join(format!("{name}.rkp")), &candidate)
                .map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

/// Compare existing evidence as generic CBOR, without a previous-format
/// journal reader and without overwriting any witness.
fn gallery(manifest: &Path) -> Result<(), String> {
    let catalog = ContractCatalog::try_m0().map_err(|error| error.to_string())?;
    for (name, directory) in [
        ("echo", "echo"),
        ("file-pipeline", "file-pipeline"),
        ("keyed-retry", "two-tenants"),
        ("webhook-to-partner", "webhook-to-partner"),
        ("sensor-alarm", "sensor-alarm"),
        ("agent-dialogue", "agent-gateway"),
        ("agent-gateway", "agent-gateway"),
        ("agent-http", "agent-http"),
    ] {
        let source = std::fs::read_to_string(manifest.join(format!("../../examples/{name}.rkg")))
            .map_err(|error| error.to_string())?;
        let compiled = compiler::compile(&[&source], &catalog)
            .map_err(|error| format!("{name}: {error:?}"))?;
        let package = plan::encode(&compiled.plan, &compiled.hash);
        describe(name, "candidate", &package)?;
        let deployment = manifest.join(format!("../../examples/{name}.deployment.json"));
        let boot = deployment
            .is_file()
            .then(|| rastok::deployment_config::load(&deployment))
            .transpose()
            .map_err(|error| error.to_string())?;
        for lab in &compiled.labs {
            for run in rastok::lab::run_program(
                &compiled.plan,
                compiled.hash,
                plan::GraphVersion(1),
                lab,
                boot.as_ref(),
            ) {
                if !run.report.passed() {
                    return Err(format!("{name}: {:?}", run.report));
                }
                let trace = rastok::golden::project(&run.records);
                let witness = format!(
                    "gallery/{directory}/{}.{}.trace",
                    run.report.simulation, run.report.scenario,
                );
                let path = manifest.join("tests/golden").join(&witness);
                let (old_count, old_seal) = if path.is_file() {
                    let stored =
                        std::fs::read_to_string(&path).map_err(|error| error.to_string())?;
                    let records = stored
                        .lines()
                        .filter(|line| !line.starts_with("seal ") && !line.is_empty())
                        .map(unhex)
                        .collect::<Result<Vec<_>, _>>()?;
                    let old = rastok::golden::Trace::from_stored(&records)
                        .ok_or_else(|| format!("{witness}: not canonical CBOR"))?;
                    if !stored
                        .lines()
                        .any(|line| line == format!("seal {}", old.seal_hex()))
                    {
                        return Err(format!("{witness}: stored seal disagrees with its bytes"));
                    }
                    (old.len().to_string(), old.seal_hex())
                } else {
                    ("new".into(), "none".into())
                };
                println!(
                    "| {witness} | {old_count} → {} | `{old_seal}` | `{}` |",
                    trace.len(),
                    trace.seal_hex()
                );
            }
        }
    }
    Ok(())
}
