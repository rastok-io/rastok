//! Opening examples must execute their assertions and match their own goldens.
#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    clippy::unwrap_used
)]

use std::path::Path;
use std::process::Command;

#[test]
fn opening_examples_match_versioned_scenario_goldens() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for (graph, directory, count) in [
        ("echo", "echo", 3),
        ("file-pipeline", "file-pipeline", 4),
        ("keyed-retry", "two-tenants", 2),
        ("webhook-to-partner", "webhook-to-partner", 6),
        ("sensor-alarm", "sensor-alarm", 3),
        ("agent-dialogue", "agent-gateway", 2),
        ("agent-gateway", "agent-gateway", 2),
        ("agent-http", "agent-http", 6),
        ("countdown", "countdown", 2),
    ] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_rastok"));
        command.current_dir(&root).args([
            "simulate",
            &format!("examples/{graph}.rkg"),
            "--compare",
            &format!("examples/traces/{directory}"),
        ]);
        if graph == "webhook-to-partner" {
            command
                .args([
                    "--deployment",
                    "examples/webhook-to-partner.deployment.json",
                ])
                .env("RASTOK_SHOWCASE_PARTNER_TOKEN", "lab-only-not-a-credential")
                .env("RASTOK_SHOWCASE_PARTNER_TOKEN_VERSION", "fixture-v1");
        } else if graph == "agent-http" {
            command
                .args(["--deployment", "examples/agent-http.deployment.json"])
                .env("RASTOK_SHOWCASE_MODEL_TOKEN", "lab-only-not-a-credential")
                .env("RASTOK_SHOWCASE_MODEL_TOKEN_VERSION", "fixture-v1");
        }
        let output = command.output().unwrap();
        assert!(output.status.success(), "{graph}: {output:?}");
        assert!(
            String::from_utf8_lossy(&output.stdout)
                .contains(&format!("{count} scenario(s), 0 failed")),
            "{output:?}"
        );
    }
}

#[test]
fn each_gallery_break_it_edit_exposes_a_record_difference_without_saving() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let temporary =
        std::env::temp_dir().join(format!("rastok-gallery-break-it-{}", std::process::id()));
    std::fs::create_dir_all(&temporary).unwrap();
    for (graph, directory, before, after) in [
        ("echo", "echo", "out = .", "out = \"changed\""),
        (
            "file-pipeline",
            "file-pipeline",
            "size(data) > 0",
            "size(data) > 20",
        ),
        (
            "keyed-retry",
            "two-tenants",
            "Retry.attempts = 2",
            "Retry.attempts = 1",
        ),
        (
            "webhook-to-partner",
            "webhook-to-partner",
            "Retry.attempts = 2",
            "Retry.attempts = 1",
        ),
        (
            "sensor-alarm",
            "sensor-alarm",
            "data.value > 120",
            "data.value > 130",
        ),
        (
            "agent-dialogue",
            "agent-gateway",
            "answer = \"Plant tomatoes.\"",
            "answer = \"Plant carrots.\"",
        ),
        (
            "agent-gateway",
            "agent-gateway",
            "Hello from the simulated agent.",
            "A different answer.",
        ),
        (
            "agent-http",
            "agent-http",
            "Hello from the simulated model.",
            "A different model answer.",
        ),
        (
            "countdown",
            "countdown",
            "out = \"liftoff\"",
            "out = \"changed\"",
        ),
    ] {
        let source = std::fs::read_to_string(root.join(format!("examples/{graph}.rkg"))).unwrap();
        assert_eq!(source.matches(before).count(), 1, "one edit in {graph}");
        let path = temporary.join(format!("{graph}.rkg"));
        std::fs::write(&path, source.replacen(before, after, 1)).unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_rastok"));
        command
            .current_dir(&root)
            .arg("simulate")
            .arg(&path)
            .arg("--compare")
            .arg(format!("examples/traces/{directory}"));
        if graph == "webhook-to-partner" {
            command
                .args([
                    "--deployment",
                    "examples/webhook-to-partner.deployment.json",
                ])
                .env("RASTOK_SHOWCASE_PARTNER_TOKEN", "lab-only-not-a-credential")
                .env("RASTOK_SHOWCASE_PARTNER_TOKEN_VERSION", "fixture-v1");
        } else if graph == "agent-http" {
            command
                .args(["--deployment", "examples/agent-http.deployment.json"])
                .env("RASTOK_SHOWCASE_MODEL_TOKEN", "lab-only-not-a-credential")
                .env("RASTOK_SHOWCASE_MODEL_TOKEN_VERSION", "fixture-v1");
        }
        let output = command.output().unwrap();
        assert_eq!(output.status.code(), Some(1), "{graph}: {output:?}");
        let text = String::from_utf8_lossy(&output.stdout);
        for expected in [
            "golden divergence: record ",
            "expected ",
            "; got ",
            "cause:",
        ] {
            assert!(text.contains(expected), "{graph}: {text}");
        }
        std::fs::remove_file(path).unwrap();
    }
    std::fs::remove_dir(temporary).unwrap();
}
