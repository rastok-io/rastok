//! Public Markdown and the editor grammar are data contracts, checked without Node.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic
)]

use serde::Deserialize;
use std::path::{Component, Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Page {
    title: String,
    description: String,
    order: u32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Gallery {
    title: String,
    description: String,
    order: u32,
    teaches: String,
    graph: String,
    golden: String,
    command: String,
    expected: String,
    break_it: String,
    limits: Vec<String>,
    replaces: String,
}

fn markdown(path: &Path) -> (String, String) {
    let text = std::fs::read_to_string(path).unwrap();
    let (front, body) = text
        .strip_prefix("---\n")
        .expect("front-matter first")
        .split_once("\n---\n")
        .expect("closed front-matter");
    assert!(
        !body.lines().any(|line| line.starts_with("# ")
            || line.starts_with("import ")
            || line.starts_with("export ")),
        "one implicit H1, no MDX: {path:?}"
    );
    (front.into(), body.into())
}

fn reference(path: &str, extension: &str) {
    let relative = Path::new(path);
    assert!(
        relative
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
    );
    assert_eq!(
        relative.extension().and_then(|s| s.to_str()),
        Some(extension)
    );
    let resolved = root()
        .join(relative)
        .canonicalize()
        .expect("referenced artifact exists");
    assert!(
        resolved.starts_with(root().canonicalize().unwrap()),
        "reference stays in the checkout"
    );
}

#[test]
fn public_pages_have_the_site_contract_and_existing_artifacts() {
    for name in ["index.md", "quickstart.md"] {
        let (front, _) = markdown(&root().join("docs/site").join(name));
        let page: Page = serde_yaml_ng::from_str(&front).unwrap();
        assert!(
            !page.title.trim().is_empty() && !page.description.trim().is_empty() && page.order > 0
        );
    }
    let mut pages: Vec<_> = std::fs::read_dir(root().join("docs/site/gallery"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    pages.sort();
    assert!(!pages.is_empty());
    let mut orders = std::collections::BTreeSet::new();
    for path in pages {
        let (front, body) = markdown(&path);
        let page: Gallery = serde_yaml_ng::from_str(&front).unwrap();
        for value in [
            &page.title,
            &page.description,
            &page.teaches,
            &page.command,
            &page.expected,
            &page.break_it,
            &page.replaces,
        ] {
            assert!(!value.trim().is_empty(), "required field in {path:?}");
        }
        assert!(page.order > 0 && orders.insert(page.order));
        assert!(!page.limits.is_empty() && page.limits.iter().all(|s| !s.trim().is_empty()));
        assert!(page.graph.starts_with("examples/"));
        reference(&page.graph, "rkg");
        reference(&page.golden, "trace");
        for section in [
            "Run",
            "Read the graph",
            "Expected result",
            "Break it",
            "Read the golden difference",
            "Carry forward",
            "Limits",
            "NiFi / MuleSoft equivalent",
        ] {
            assert!(
                body.contains(&format!("\n## {section}\n")),
                "{path:?}: {section}"
            );
        }
        assert!(
            !body
                .lines()
                .any(|line| line.starts_with("graph ") && line.trim_end().ends_with('{')),
            "the graph is embedded by reference, not copied"
        );
    }
}

#[test]
fn gallery_chapters_form_a_connected_learning_path() {
    let directory = root().join("docs/site/gallery");
    let (_, index) = markdown(&root().join("docs/site/index.md"));
    let mut chapters: Vec<_> = std::fs::read_dir(directory)
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            let (front, body) = markdown(&path);
            let page: Gallery = serde_yaml_ng::from_str(&front).unwrap();
            (
                page.order,
                path.file_name().unwrap().to_str().unwrap().to_owned(),
                body,
            )
        })
        .collect();
    chapters.sort_by_key(|(order, _, _)| *order);
    let mut previous_index_position = None;
    for (position, (order, filename, body)) in chapters.iter().enumerate() {
        assert_eq!(
            *order as usize,
            position + 1,
            "no missing published chapter"
        );
        let index_position = index
            .find(&format!("(gallery/{filename})"))
            .unwrap_or_else(|| panic!("index links {filename}"));
        assert!(
            previous_index_position.is_none_or(|previous| previous < index_position),
            "the index presents chapters in reading order"
        );
        previous_index_position = Some(index_position);
        let carry = body
            .split_once("\n## Carry forward\n")
            .expect("each chapter leaves the reader with a next step")
            .1
            .split_once("\n## Limits\n")
            .expect("limits remain visible after the learning path")
            .0;
        if let Some((_, previous, _)) = position.checked_sub(1).map(|i| &chapters[i]) {
            assert!(
                carry.contains(&format!("({previous})")),
                "{filename}: previous chapter"
            );
        }
        if let Some((_, next, _)) = chapters.get(position + 1) {
            assert!(
                carry.contains(&format!("({next})")),
                "{filename}: next chapter"
            );
        }
    }
}

#[test]
fn incomplete_or_extended_site_frontmatter_refuses() {
    assert!(serde_yaml_ng::from_str::<Page>("title: A\norder: 1\n").is_err());
    assert!(
        serde_yaml_ng::from_str::<Page>("title: A\ndescription: B\norder: 1\ncomponent: X\n")
            .is_err()
    );
    assert!(
        serde_yaml_ng::from_str::<Page>("title: A\ntitle: C\ndescription: B\norder: 1\n").is_err()
    );
    assert!(serde_yaml_ng::from_str::<Gallery>("title: A\ndescription: B\norder: 1\n").is_err());
}

#[test]
fn readme_quickstart_is_a_verbatim_copy_of_the_site_source() {
    let block = |path| {
        let text = std::fs::read_to_string(root().join(path)).unwrap();
        text.split_once("<!-- quickstart:start -->")
            .unwrap()
            .1
            .split_once("<!-- quickstart:end -->")
            .unwrap()
            .0
            .to_owned()
    };
    assert_eq!(block("README.md"), block("docs/site/quickstart.md"));
}

#[test]
fn current_public_status_matches_the_package_and_contracts() {
    let paper = std::fs::read_to_string(root().join("WHITEPAPER.md")).unwrap();
    let version = rastok_core::plan::FORMAT_VERSION;
    assert!(paper.contains(&format!("Format-{version} (")));
    assert!(paper.contains(&format!("**package {version} / diagnostic recording 4**")));
    assert!(!paper.contains("blessing pending review"));
    let contract = rastok_core::connector::stdlib::PROCESS_CONNECTOR_ID;
    assert!(paper.contains(contract), "whitepaper names {contract}");
    for stale in [
        "process@2",
        "process@3",
        "process@4",
        "process@5",
        "`effect` make repetition safe",
        "effect is `none`",
        "port-type, effect, behavior",
    ] {
        assert!(!paper.contains(stale), "whitepaper still teaches {stale}");
    }

    let catalogue = std::fs::read_to_string(root().join("docs/examples/README.md")).unwrap();
    assert!(catalogue.contains(&format!("**format-{version} / diagnostic-recording-4**")));
    assert!(catalogue.contains("connector-rework-identity.md"));
    assert!(!catalogue.contains("unblessed v19 candidate"));
    assert!(!catalogue.contains("v18 is the last qualified"));

    let registry = std::fs::read_to_string(root().join("docs/spec/identity.md")).unwrap();
    assert!(registry.contains(&format!("**Format {version}, diagnostic recording 4.**")));
    for path in [
        "docs/spec/execution-model.md",
        "docs/spec/delivery-semantics.md",
        "docs/spec/loop.md",
    ] {
        let source = std::fs::read_to_string(root().join(path)).unwrap();
        let status = source
            .lines()
            .find(|line| line.starts_with("> **Status"))
            .unwrap();
        assert!(
            status.contains("[current format](identity.md)"),
            "{path}: {status}"
        );
    }
    for path in [
        "docs/architecture/opening-showcase.md",
        "docs/architecture/diagnostic-recording.md",
    ] {
        let source = std::fs::read_to_string(root().join(path)).unwrap();
        let status = source.split("\n## ").next().unwrap();
        assert!(
            status.contains("[current plan package](../spec/identity.md)"),
            "{path}: the live status must refer to the current registry"
        );
        assert!(status.contains("connector-rework-identity.md"), "{path}");
    }
}

#[test]
fn live_loop_diagnostics_do_not_reintroduce_retired_codes() {
    let source = std::fs::read_to_string(root().join("docs/spec/loop.md")).unwrap();
    let registry = std::fs::read_to_string(root().join("docs/spec/diagnostics.md")).unwrap();
    let table = source
        .split_once("## 15. Named diagnostics")
        .unwrap()
        .1
        .split_once("## 16.")
        .unwrap()
        .0;
    let mut codes = 0;
    for row in table.lines() {
        let Some(code) = row.split('|').nth(1) else {
            continue;
        };
        let code = code.trim().trim_matches('`');
        if !(code.starts_with("E-") || code.starts_with("W-")) {
            continue;
        }
        let registered = registry
            .lines()
            .find(|line| line.starts_with(&format!("| {code} |")))
            .unwrap_or_else(|| panic!("unregistered Loop diagnostic {code}"));
        assert_ne!(
            registered.split('|').nth(2).unwrap().trim(),
            "retired",
            "loop.md still presents retired {code} as an active diagnostic"
        );
        codes += 1;
    }
    assert!(codes > 0);
    assert!(table.contains("W-LOOP-REPEAT"));
}

#[test]
fn current_journal_tables_include_retry_retention_without_saga_key_rules() {
    let source = std::fs::read_to_string(root().join("docs/spec/identity.md")).unwrap();
    for (key, name) in [(41, "deadlines"), (44, "closed_at")] {
        assert!(
            source.contains(&format!("| {key} | {name} |")),
            "identity.md omits the unsealed field {key}: {name}"
        );
    }
    let deadline_ids = source
        .lines()
        .find(|line| line.starts_with("| 24 | deadline_ids |"))
        .unwrap();
    assert!(deadline_ids.contains("Loop") && deadline_ids.contains("Retry"));
    let key = source
        .lines()
        .find(|line| line.starts_with("| 29 | call_key |"))
        .unwrap();
    assert!(
        !key.contains("E-COMPENSATE-KEY") && !key.contains("compensable"),
        "the optional partner key is not a Saga admission rule"
    );
}

#[test]
fn whitepaper_retry_example_checks_with_its_real_contract() {
    let paper = std::fs::read_to_string(root().join("WHITEPAPER.md")).unwrap();
    let declaration = paper
        .split("```rastok\n")
        .find(|block| block.starts_with("connector partner : HttpClient + Retry"))
        .unwrap()
        .split_once("```")
        .unwrap()
        .0;
    let source = format!(
        "graph demo : App {{\ninput in: bytes\n{declaration}\nin -> partner\npartner.out -> nothing\npartner.exit -> nothing\n}}"
    );
    rastok::compiler::compile(
        &[&source],
        &rastok::connector::contract::ContractCatalog::try_m0().unwrap(),
    )
    .unwrap();
}

#[test]
fn public_dsl_excerpts_do_not_use_removed_effect_or_cost_syntax() {
    let mut files = ["README.md", "WHITEPAPER.md", "docs/spec/instances.md"]
        .map(|path| root().join(path))
        .to_vec();
    let mut directories = vec![root().join("docs/site")];
    while let Some(directory) = directories.pop() {
        for entry in std::fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                directories.push(path);
            } else if path.extension().is_some_and(|extension| extension == "md") {
                files.push(path);
            }
        }
    }
    files.sort();
    for path in files {
        let source = std::fs::read_to_string(&path).unwrap();
        let mut in_dsl = false;
        for (index, line) in source.lines().enumerate() {
            let line = line.trim();
            if line.starts_with("```") {
                in_dsl = matches!(line, "```rastok" | "```rkg");
            } else if in_dsl {
                // Migration prose may name retired syntax; current DSL excerpts may not use it.
                let words: Vec<_> = line.split_whitespace().collect();
                assert!(
                    !words.windows(2).any(|pair| pair == ["effect", "="])
                        && !line.contains("effect=")
                        && !line.contains("@cost("),
                    "{}:{} contains removed DSL syntax",
                    path.display(),
                    index + 1
                );
            }
        }
    }
}

#[test]
fn ci_compilation_uses_locked_dependency_resolution() {
    let source = std::fs::read_to_string(root().join(".github/workflows/ci.yml")).unwrap();
    let workflow: serde_yaml_ng::Value = serde_yaml_ng::from_str(&source).unwrap();
    let mut commands = 0;
    for (job, value) in workflow["jobs"].as_mapping().unwrap() {
        for step in value["steps"].as_sequence().unwrap() {
            let Some(run) = step["run"].as_str() else {
                continue;
            };
            for line in run.lines() {
                let words: Vec<_> = line.split_whitespace().collect();
                let Some(cargo) = words.iter().position(|word| *word == "cargo") else {
                    continue;
                };
                if matches!(
                    words.get(cargo + 1),
                    Some(&("build" | "check" | "clippy" | "test" | "doc" | "run"))
                ) {
                    commands += 1;
                    assert!(
                        words[cargo + 2..].contains(&"--locked"),
                        "{}: {line}",
                        job.as_str().unwrap()
                    );
                }
            }
        }
    }
    assert!(commands > 0, "the workflow must contain compilation checks");
}

#[test]
fn the_editor_package_is_data_only_and_the_site_grammar_resolves() {
    let editor = root().join("editors/vscode/rastok-dsl");
    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(editor.join("package.json")).unwrap())
            .unwrap();
    assert!(manifest.get("dependencies").is_none() && manifest.get("main").is_none());
    let grammar: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(editor.join("syntaxes/rkg.tmLanguage.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(grammar["scopeName"], "source.rkg");
    fn visit(value: &serde_json::Value, repository: &serde_json::Value) {
        match value {
            serde_json::Value::Object(fields) => {
                if let Some(include) = fields.get("include").and_then(|v| v.as_str()) {
                    assert!(
                        repository.get(include.strip_prefix('#').unwrap()).is_some(),
                        "{include}"
                    );
                }
                for child in fields.values() {
                    visit(child, repository);
                }
            }
            serde_json::Value::Array(values) => {
                for value in values {
                    visit(value, repository);
                }
            }
            _ => {}
        }
    }
    visit(&grammar, &grammar["repository"]);
    let _: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(editor.join("language-configuration.json")).unwrap(),
    )
    .unwrap();
}

#[test]
fn publication_assets_have_the_documented_dimensions_and_retained_provenance() {
    let assets = root().join("docs/site/assets");
    for name in [
        "webhook-before.png",
        "webhook-backoff.png",
        "webhook-completed.png",
    ] {
        let png = std::fs::read(assets.join(name)).unwrap();
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(
            u32::from_be_bytes(png[16..20].try_into().unwrap()),
            1352,
            "{name}"
        );
        assert_eq!(
            u32::from_be_bytes(png[20..24].try_into().unwrap()),
            760,
            "{name}"
        );
    }
    let gif = std::fs::read(assets.join("webhook-tour.gif")).unwrap();
    assert_eq!(&gif[..6], b"GIF89a");
    assert_eq!(u16::from_le_bytes(gif[6..8].try_into().unwrap()), 1352);
    assert_eq!(u16::from_le_bytes(gif[8..10].try_into().unwrap()), 760);
    let evidence: serde_json::Value =
        serde_json::from_slice(&std::fs::read(assets.join("webhook-capture.json")).unwrap())
            .unwrap();
    assert_eq!(evidence["scenario"], "webhook.timeout_then_success");
    assert_eq!(evidence["gif"]["seconds"], 30);
    let transitions = evidence["gif"]["transitions"].as_array().unwrap();
    assert_eq!(transitions.len(), 120);
    assert_eq!(transitions.first().unwrap(), 0);
    assert_eq!(
        transitions.last().unwrap(),
        &evidence["screenshots"]["completed"]
    );
    assert!(
        transitions
            .windows(2)
            .all(|pair| pair[0].as_u64() <= pair[1].as_u64())
    );
    let chapter =
        std::fs::read_to_string(root().join("docs/site/gallery/04-webhook-to-partner.md")).unwrap();
    for checkpoint in ["before", "backoff", "completed"] {
        let transition = evidence["screenshots"][checkpoint].as_u64().unwrap();
        assert!(
            chapter.contains(&format!(
                "transition {transition}](../assets/webhook-{checkpoint}.png)"
            )),
            "{checkpoint} caption must name the transition captured in the manifest"
        );
    }
}
