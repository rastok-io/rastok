//! The contract metamodel and the effective sheet (D-2026-08-12-1):
//! presence, profiles, defaults injected before the hash, the plan
//! carrying effective attributes, and the adapter consuming resolved
//! values only.

#![allow(clippy::disallowed_types, clippy::disallowed_methods)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use rastok::compiler::compile;
use rastok::contract::{
    AttributeSpec, ConnectorContract, DefaultValue, Presence, Profile, ProfileSelector,
    ProfileSupport, Registry, SheetDefect, ValueDomain,
};

fn compile_one(source: &str) -> Result<rastok::compiler::Compilation, rastok::diagnostics::Bag> {
    compile(&[source], &Registry::m0())
}

fn process_graph(body: &str) -> String {
    format!(
        "graph g : App {{\n  connector shell : Process {{\n{body}\n  }}\n  . -> shell -> .\n}}\n"
    )
}

const FULL: &str =
    "    command = \"cat\" ; codec = lines\n    effect = idempotent ; behavior = deterministic";

fn refusal_with(source: &str, code: &str, needle: &str) {
    let bag = compile_one(source).expect_err("must refuse");
    assert!(
        bag.diagnostics()
            .iter()
            .any(|d| d.code().as_str() == code && d.message().contains(needle)),
        "wanted {code} containing {needle:?}, got: {bag:?}"
    );
}

// ---------- the metamodel holds its own door ----------

fn sheet(common: &'static [AttributeSpec], selector: Option<ProfileSelector>) -> ConnectorContract {
    ConnectorContract {
        name: "Probe",
        input_ports: &["in"],
        output_ports: &["out", "err"],
        common,
        selector,
        error_set: &["ProbeFailed"],
    }
}

#[test]
fn a_duplicate_attribute_is_a_sheet_defect() {
    const DUP: &[AttributeSpec] = &[
        AttributeSpec {
            name: "x",
            presence: Presence::Optional,
            domain: ValueDomain::Text,
        },
        AttributeSpec {
            name: "x",
            presence: Presence::Optional,
            domain: ValueDomain::Text,
        },
    ];
    assert_eq!(
        Registry::m0().try_with(sheet(DUP, None)).unwrap_err(),
        SheetDefect::DuplicateAttribute("x")
    );
}

#[test]
fn a_duplicate_profile_is_a_sheet_defect() {
    const TWICE: ProfileSelector = ProfileSelector {
        attribute: "mode",
        default: Some("a"),
        profiles: &[
            Profile {
                word: "a",
                support: ProfileSupport::Available,
                attributes: &[],
            },
            Profile {
                word: "a",
                support: ProfileSupport::Available,
                attributes: &[],
            },
        ],
    };
    assert_eq!(
        Registry::m0()
            .try_with(sheet(&[], Some(TWICE)))
            .unwrap_err(),
        SheetDefect::DuplicateProfile("a")
    );
}

#[test]
fn an_unknown_default_profile_is_a_sheet_defect() {
    const GHOST: ProfileSelector = ProfileSelector {
        attribute: "mode",
        default: Some("ghost"),
        profiles: &[Profile {
            word: "a",
            support: ProfileSupport::Available,
            attributes: &[],
        }],
    };
    assert_eq!(
        Registry::m0()
            .try_with(sheet(&[], Some(GHOST)))
            .unwrap_err(),
        SheetDefect::UnknownDefaultProfile("ghost")
    );
}

#[test]
fn a_selector_colliding_with_an_attribute_is_a_sheet_defect() {
    const MODE: &[AttributeSpec] = &[AttributeSpec {
        name: "mode",
        presence: Presence::Optional,
        domain: ValueDomain::Text,
    }];
    const SELECTOR: ProfileSelector = ProfileSelector {
        attribute: "mode",
        default: Some("a"),
        profiles: &[Profile {
            word: "a",
            support: ProfileSupport::Available,
            attributes: &[],
        }],
    };
    assert_eq!(
        Registry::m0()
            .try_with(sheet(MODE, Some(SELECTOR)))
            .unwrap_err(),
        SheetDefect::SelectorCollision("mode")
    );
}

#[test]
fn an_uncompilable_cel_default_is_a_sheet_defect() {
    const BAD: &[AttributeSpec] = &[AttributeSpec {
        name: "x",
        presence: Presence::Defaulted(DefaultValue::CelExpression("] not cel [")),
        domain: ValueDomain::Text,
    }];
    assert!(matches!(
        Registry::m0().try_with(sheet(BAD, None)).unwrap_err(),
        SheetDefect::InvalidDefault { attribute: "x", .. }
    ));
}

#[test]
fn a_default_outside_its_domain_is_a_sheet_defect() {
    // `1` compiles and reduces — but it is no Text.
    const MISFIT: &[AttributeSpec] = &[AttributeSpec {
        name: "x",
        presence: Presence::Defaulted(DefaultValue::CelExpression("1")),
        domain: ValueDomain::Text,
    }];
    assert!(matches!(
        Registry::m0().try_with(sheet(MISFIT, None)).unwrap_err(),
        SheetDefect::InvalidDefault { attribute: "x", .. }
    ));
    // A Choice default must be a word of the domain.
    const OUTSIDE: &[AttributeSpec] = &[AttributeSpec {
        name: "y",
        presence: Presence::Defaulted(DefaultValue::Choice("ghost")),
        domain: ValueDomain::Choice(&["real"]),
    }];
    assert!(matches!(
        Registry::m0().try_with(sheet(OUTSIDE, None)).unwrap_err(),
        SheetDefect::InvalidDefault { attribute: "y", .. }
    ));
}

#[test]
fn a_duplicate_choice_word_is_a_sheet_defect() {
    const DUP: &[AttributeSpec] = &[AttributeSpec {
        name: "x",
        presence: Presence::Optional,
        domain: ValueDomain::Choice(&["a", "a"]),
    }];
    assert_eq!(
        Registry::m0().try_with(sheet(DUP, None)).unwrap_err(),
        SheetDefect::DuplicateChoice("x")
    );
}

#[test]
fn a_deferred_only_domain_needs_a_deferred_profile() {
    const RAW: &[AttributeSpec] = &[AttributeSpec {
        name: "correlate",
        presence: Presence::Optional,
        domain: ValueDomain::CelPath,
    }];
    assert_eq!(
        Registry::m0().try_with(sheet(RAW, None)).unwrap_err(),
        SheetDefect::UnimplementedDomain("correlate")
    );
}

// ---------- the dead spellings refuse with their migration ----------

#[test]
fn a_configuration_sub_block_names_its_migration() {
    let source = process_graph(&format!("{FULL}\n    env {{ PATH = \"/bin\" }}"));
    refusal_with(
        &source,
        "E-CONTRACT-ATTR",
        "`env { … }` is no longer a configuration form; write `env = { … }`",
    );
}

#[test]
fn a_tagged_initializer_names_its_migration() {
    let source = process_graph(&format!(
        "{FULL}\n    restart = on_crash {{ backoff = \"1s\" }}"
    ));
    refusal_with(
        &source,
        "E-PARSE-EXPECTED",
        "dead tagged-initializer spelling",
    );
}

#[test]
fn a_dotted_attribute_is_not_a_composite_patch() {
    // `tls.verify` names no owner — it refuses whole, never patches
    // a `tls` composite.
    let source = process_graph(&format!("{FULL}\n    tls.verify = true"));
    refusal_with(&source, "E-CONTRACT-ATTR", "not on the contract sheet");
}

// ---------- the Process profiles ----------

#[test]
fn mode_omitted_and_mode_job_are_one_identity() {
    let omitted = compile_one(&process_graph(FULL)).expect("valid");
    let written = compile_one(&process_graph(&format!("    mode = job\n{FULL}"))).expect("valid");
    assert_eq!(
        omitted.hash.to_string(),
        written.hash.to_string(),
        "omission of a Defaulted selector IS its exact spelling"
    );
    assert_eq!(*omitted.plan, *written.plan, "one effective plan");
}

#[test]
fn every_default_travels_in_the_plan() {
    let compilation = compile_one(&process_graph(FULL)).expect("valid");
    let spec = compilation.plan.plan().nodes[0]
        .connector()
        .expect("a connector");
    let get = |name: &str| spec.attribute(name).cloned();
    assert_eq!(
        get("mode"),
        Some(rastok::plan::AttrSetting::Word(String::from("job")))
    );
    assert_eq!(
        get("on_timeout"),
        Some(rastok::plan::AttrSetting::Word(String::from("kill")))
    );
    assert_eq!(
        get("max_output"),
        Some(rastok::plan::AttrSetting::Value(rastok::cel::Value::Text(
            String::from("8MiB")
        )))
    );
    assert_eq!(
        get("args"),
        Some(rastok::plan::AttrSetting::Value(rastok::cel::Value::List(
            Vec::new()
        )))
    );
}

#[test]
fn defaults_survive_the_package_round_trip() {
    let compilation = compile_one(&process_graph(FULL)).expect("valid");
    let bytes = rastok::plan::encode(&compilation.plan, &compilation.hash);
    let (decoded, _) = rastok::plan::decode(&bytes).expect("the package decodes");
    let spec = decoded.plan().nodes[0].connector().expect("a connector");
    assert!(
        spec.attribute("max_output").is_some(),
        "the default travels"
    );
    assert!(spec.attribute("args").is_some());
    assert!(spec.attribute("on_timeout").is_some());
}

#[test]
fn writing_a_default_exactly_is_the_same_identity() {
    let omitted = compile_one(&process_graph(FULL)).expect("valid");
    let written = compile_one(&process_graph(&format!(
        "{FULL}\n    args = []\n    max_output = \"8MiB\"\n    on_timeout = kill\n    mode = job"
    )))
    .expect("valid");
    assert_eq!(omitted.hash.to_string(), written.hash.to_string());
    assert_eq!(*omitted.plan, *written.plan);
}

#[test]
fn args_null_refuses_as_a_type() {
    // Omission resolves to `[]`; `null` is a real, distinct CEL
    // value and the domain is not nullable.
    let source = process_graph(&format!("{FULL}\n    args = null"));
    refusal_with(
        &source,
        "E-CONTRACT-ATTR",
        "does not fit the TextList domain",
    );
}

#[test]
fn codec_stays_required() {
    let source =
        process_graph("    command = \"cat\"\n    effect = idempotent ; behavior = deterministic");
    let bag = compile_one(&source).expect_err("must refuse");
    assert!(
        bag.diagnostics()
            .iter()
            .any(|d| d.code().as_str() == "E-CONTRACT-REQUIRED" && d.message().contains("`codec`")),
        "{bag:?}"
    );
}

#[test]
fn a_choice_written_as_cel_text_names_the_bare_word() {
    let source = process_graph(
        "    mode = \"job\"\n    command = \"cat\" ; codec = lines\n    effect = idempotent ; behavior = deterministic",
    );
    refusal_with(&source, "E-CONTRACT-ATTR", "bare word");
}

#[test]
fn a_cel_domain_refuses_a_bare_word() {
    let source = process_graph(
        "    command = cat ; codec = lines\n    effect = idempotent ; behavior = deterministic",
    );
    refusal_with(&source, "E-CONTRACT-ATTR", "`cat` is a bare word");
}

#[test]
fn on_timeout_bypass_says_fixed_to_kill() {
    let source = process_graph(&format!("{FULL}\n    on_timeout = bypass"));
    refusal_with(
        &source,
        "E-CONTRACT-ATTR",
        "`on_timeout` is fixed to `kill` for `Process(mode = job)`",
    );
}

#[test]
fn a_service_attribute_on_a_job_names_the_owning_profile() {
    let source = process_graph(&format!("{FULL}\n    restart = {{ max_attempts = 5 }}"));
    refusal_with(
        &source,
        "E-CONTRACT-ATTR",
        "`restart` belongs to `Process(mode = service)` and is not applicable to `Process(mode = job)`",
    );
}

#[test]
fn mode_service_defers_before_validating_its_attributes() {
    let source = process_graph(&format!(
        "    mode = service\n{FULL}\n    correlate = data.id"
    ));
    let bag = compile_one(&source).expect_err("service defers");
    assert!(
        bag.diagnostics()
            .iter()
            .any(|d| d.code().as_str() == "E-CHECK-DEFERRED"
                && d.message().contains("the Process service mode")),
        "{bag:?}"
    );
    // The deferred profile's own attribute is NOT judged by a
    // domain that does not exist yet.
    assert!(
        !bag.diagnostics()
            .iter()
            .any(|d| d.code().as_str() == "E-CONTRACT-ATTR" && d.message().contains("correlate")),
        "{bag:?}"
    );
}

/// Mutates the compiled plan's connector spec, runs it against the
/// REAL adapter, and returns the one typed error payload's message —
/// the shared probe for every violated-invariant case.
fn run_mutated(mutate: impl Fn(&mut rastok::plan::ConnectorSpec)) -> (String, String) {
    let compilation = compile_one(&process_graph(FULL)).expect("valid");
    let mut plan: rastok::plan::ExecutionPlan = (*compilation.plan).clone();
    for node in &mut plan.nodes {
        if let rastok::plan::NodeKind::Connector(spec) = &mut node.kind {
            mutate(spec);
        }
    }
    let validated =
        rastok::plan::ValidatedExecutionPlan::try_from(plan).expect("structurally valid");
    let mut journal = rastok::journal::MemJournal::new(rastok::plan::GraphVersion(1));
    let mut vm = rastok::runtime::vm::Vm::boot(&validated, rastok::plan::GraphVersion(1));
    let mut world = rastok::connectors::Process::new(&validated);
    vm.inject(
        &mut journal,
        rastok::boundary::InboundSubmission {
            authority: rastok::boundary::IngressAuthority::single(
                rastok::node::Path::new(["probe"]).unwrap(),
            ),
            port: rastok::node::Port::new("in").unwrap(),
            draft: rastok::boundary::Draft {
                data: rastok::envelope::Data::materialized(rastok::envelope::Value::Scalar(
                    rastok::envelope::Scalar::Text(String::from("x")),
                )),
                ext: rastok::envelope::Namespace::new(),
            },
            sequence: rastok::boundary::SubmissionSequence::Next,
        },
    )
    .expect("injection lands");
    vm.drain(&mut journal).expect("the request opens");
    for _ in 0..200 {
        if vm.is_stable() && !vm.has_open_obligations() {
            break;
        }
        vm.pump(&mut journal, &mut world)
            .expect("the world answers");
        vm.drain(&mut journal).expect("the reply routes");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let errors = vm.errors();
    assert_eq!(errors.len(), 1, "one typed violation");
    let rastok::envelope::Data::Materialized(rastok::envelope::Value::Map(payload)) =
        errors[0].data()
    else {
        panic!("a typed error payload");
    };
    let Some(rastok::envelope::Value::Scalar(rastok::envelope::Scalar::Text(kind))) =
        payload.get("kind")
    else {
        panic!("a kind: {payload:?}");
    };
    let Some(rastok::envelope::Value::Map(detail)) = payload.get("detail") else {
        panic!("the violated invariant is named: {payload:?}");
    };
    let Some(rastok::envelope::Value::Scalar(rastok::envelope::Scalar::Text(message))) =
        detail.get("message")
    else {
        panic!("a message: {detail:?}");
    };
    (kind.clone(), message.clone())
}

#[test]
fn the_adapter_refuses_every_stripped_default_individually() {
    // Each Defaulted attribute is defended on its own — the review
    // caught `mode` and `on_timeout` unguarded, and a joint strip
    // exercising only the first refusal.
    for name in ["mode", "on_timeout", "args", "max_output"] {
        let (kind, message) = run_mutated(|spec| {
            spec.attributes.retain(|(attr, _)| attr != name);
        });
        assert_eq!(kind, "SpawnFailed", "{name}");
        assert!(
            message.contains("unresolved"),
            "`{name}` names the violated invariant: {message}"
        );
    }
}

#[test]
fn the_adapter_drops_no_argument_silently() {
    // A non-text element in `args` is a refusal, never a
    // filter_map hole.
    let (kind, message) = run_mutated(|spec| {
        for (name, setting) in &mut spec.attributes {
            if name == "args" {
                *setting = rastok::plan::AttrSetting::Value(rastok::cel::Value::List(vec![
                    rastok::cel::Value::Text(String::from("ok")),
                    rastok::cel::Value::Int(7),
                ]));
            }
        }
    });
    assert_eq!(kind, "SpawnFailed");
    assert!(message.contains("not text"), "{message}");
}

#[test]
fn the_adapter_refuses_an_incomplete_plan_instead_of_defaulting() {
    // A crafted plan stripped of its Defaulted attributes must
    // surface a typed invariant violation — the adapter never
    // reintroduces "8MiB" silently.
    let compilation = compile_one(&process_graph(FULL)).expect("valid");
    let mut plan: rastok::plan::ExecutionPlan = (*compilation.plan).clone();
    for node in &mut plan.nodes {
        if let rastok::plan::NodeKind::Connector(spec) = &mut node.kind {
            spec.attributes
                .retain(|(name, _)| name != "max_output" && name != "args");
        }
    }
    let validated =
        rastok::plan::ValidatedExecutionPlan::try_from(plan).expect("structurally valid");
    let mut journal = rastok::journal::MemJournal::new(rastok::plan::GraphVersion(1));
    let mut vm = rastok::runtime::vm::Vm::boot(&validated, rastok::plan::GraphVersion(1));
    let mut world = rastok::connectors::Process::new(&validated);
    vm.inject(
        &mut journal,
        rastok::boundary::InboundSubmission {
            authority: rastok::boundary::IngressAuthority::single(
                rastok::node::Path::new(["probe"]).unwrap(),
            ),
            port: rastok::node::Port::new("in").unwrap(),
            draft: rastok::boundary::Draft {
                data: rastok::envelope::Data::materialized(rastok::envelope::Value::Scalar(
                    rastok::envelope::Scalar::Text(String::from("x")),
                )),
                ext: rastok::envelope::Namespace::new(),
            },
            sequence: rastok::boundary::SubmissionSequence::Next,
        },
    )
    .expect("injection lands");
    vm.drain(&mut journal).expect("the request opens");
    for _ in 0..200 {
        if vm.is_stable() && !vm.has_open_obligations() {
            break;
        }
        vm.pump(&mut journal, &mut world)
            .expect("the world answers");
        vm.drain(&mut journal).expect("the reply routes");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let errors = vm.errors();
    assert_eq!(errors.len(), 1, "one typed violation");
    let rastok::envelope::Data::Materialized(rastok::envelope::Value::Map(payload)) =
        errors[0].data()
    else {
        panic!("a typed error payload");
    };
    assert_eq!(
        payload.get("kind"),
        Some(&rastok::envelope::Value::Scalar(
            rastok::envelope::Scalar::Text(String::from("SpawnFailed"))
        ))
    );
    let Some(rastok::envelope::Value::Map(detail)) = payload.get("detail") else {
        panic!("the violated invariant is named: {payload:?}");
    };
    let Some(rastok::envelope::Value::Scalar(rastok::envelope::Scalar::Text(message))) =
        detail.get("message")
    else {
        panic!("a message: {detail:?}");
    };
    assert!(message.contains("unresolved"), "{message}");
}

#[test]
fn the_new_discriminant_words_parse_as_words() {
    // The review's blocker: `exponential` INSIDE a struct is a bare
    // word, never a resurrected dead-initializer refusal — the form
    // (word alone vs word + `(`/`{`) decides.
    let source = process_graph(&format!("{FULL}\n    bogus = {{ curve = exponential }}"));
    let bag = compile_one(&source).expect_err("bogus is not on the sheet");
    assert!(
        !bag.diagnostics()
            .iter()
            .any(|d| d.code().as_str() == "E-PARSE-EXPECTED"),
        "the parser preempts nothing: {bag:?}"
    );
    assert!(
        bag.diagnostics()
            .iter()
            .any(|d| d.code().as_str() == "E-CONTRACT-ATTR"
                && d.message().contains("not on the contract sheet")),
        "{bag:?}"
    );
}

#[test]
fn a_bare_tag_word_is_judged_by_the_contract() {
    // `codec = json`: `json` is a TAG WORD, but the word ALONE is a
    // choice for the contract to judge — it must never eat the next
    // line as a JSON value.
    let source = process_graph(
        "    command = \"cat\" ; codec = json\n    effect = idempotent ; behavior = deterministic",
    );
    refusal_with(
        &source,
        "E-CONTRACT-ATTR",
        "expected one of [\"jsonl\", \"lines\"], got `json`",
    );
}

#[test]
fn tag_words_serve_as_choice_words_on_another_sheet() {
    // A future sheet may spell `text`, `json` or `unbounded` as
    // closed choices — the parser leaves the words to the contract.
    const CHOICES: &[AttributeSpec] = &[
        AttributeSpec {
            name: "behavior",
            presence: Presence::Required,
            domain: ValueDomain::Choice(&["deterministic"]),
        },
        AttributeSpec {
            name: "effect",
            presence: Presence::Required,
            domain: ValueDomain::Choice(&["idempotent"]),
        },
        AttributeSpec {
            name: "framing",
            presence: Presence::Required,
            domain: ValueDomain::Choice(&["text", "json", "unbounded"]),
        },
    ];
    let registry = Registry::m0()
        .try_with(sheet(CHOICES, None))
        .expect("a valid sheet");
    for word in ["text", "json", "unbounded"] {
        let source = format!(
            "graph g : App {{\n  connector p : Probe {{\n    framing = {word}\n    effect = idempotent ; behavior = deterministic\n  }}\n  . -> p -> .\n}}\n"
        );
        let result = compile(&[source.as_str()], &registry);
        assert!(result.is_ok(), "`framing = {word}` compiles: {result:?}");
    }
}

#[test]
fn max_output_is_judged_at_check_not_at_spawn() {
    // The ByteSize domain closes the review's false green: a
    // non-size refuses at CHECK, through the same parser the
    // adapter enforces the budget with.
    let source = process_graph(&format!("{FULL}\n    max_output = \"banana\""));
    refusal_with(&source, "E-CONTRACT-ATTR", "ByteSize");
    assert_eq!(rastok::contract::byte_size("8MiB"), Some(8 * 1024 * 1024));
    assert_eq!(rastok::contract::byte_size("banana"), None);
    assert_eq!(
        rastok::contract::byte_size("1024"),
        None,
        "an explicit unit is required"
    );
}

#[test]
fn a_deferred_profile_still_owes_the_common_sheet() {
    // `mode = service` alone: the deferral speaks AND the common
    // Required attributes are still owed.
    let source = process_graph("    mode = service");
    let bag = compile_one(&source).expect_err("must refuse");
    for name in ["`command`", "`codec`", "`effect`", "`behavior`"] {
        assert!(
            bag.diagnostics()
                .iter()
                .any(|d| d.code().as_str() == "E-CONTRACT-REQUIRED" && d.message().contains(name)),
            "missing required refusal for {name}: {bag:?}"
        );
    }
}

#[test]
fn sibling_profiles_may_share_an_attribute_name() {
    // The selected profile disambiguates — only common-vs-profile
    // and within-scope duplicates are defects.
    const SHARED: &AttributeSpec = &AttributeSpec {
        name: "shared",
        presence: Presence::Optional,
        domain: ValueDomain::Text,
    };
    const SELECTOR: ProfileSelector = ProfileSelector {
        attribute: "mode",
        default: Some("a"),
        profiles: &[
            Profile {
                word: "a",
                support: ProfileSupport::Available,
                attributes: std::slice::from_ref(SHARED),
            },
            Profile {
                word: "b",
                support: ProfileSupport::Available,
                attributes: std::slice::from_ref(SHARED),
            },
        ],
    };
    assert!(Registry::m0().try_with(sheet(&[], Some(SELECTOR))).is_ok());
}

#[test]
fn an_inexpressible_sheet_name_is_a_sheet_defect() {
    let bad_port = ConnectorContract {
        name: "Probe",
        input_ports: &["bad port"],
        output_ports: &["out", "err"],
        common: &[],
        selector: None,
        error_set: &[],
    };
    assert_eq!(
        Registry::m0().try_with(bad_port).unwrap_err(),
        SheetDefect::InvalidPortName("bad port")
    );
    const BAD_ATTR: &[AttributeSpec] = &[AttributeSpec {
        name: "not a name",
        presence: Presence::Optional,
        domain: ValueDomain::Text,
    }];
    assert_eq!(
        Registry::m0().try_with(sheet(BAD_ATTR, None)).unwrap_err(),
        SheetDefect::InexpressibleName("not a name")
    );
}

#[test]
fn a_deferred_domain_carries_no_default() {
    const LIE: ProfileSelector = ProfileSelector {
        attribute: "mode",
        default: Some("a"),
        profiles: &[Profile {
            word: "a",
            support: ProfileSupport::Deferred("someday"),
            attributes: &[AttributeSpec {
                name: "x",
                presence: Presence::Defaulted(DefaultValue::CelExpression("[]")),
                domain: ValueDomain::Struct,
            }],
        }],
    };
    assert!(matches!(
        Registry::m0().try_with(sheet(&[], Some(LIE))).unwrap_err(),
        SheetDefect::InvalidDefault { attribute: "x", .. }
    ));
}

#[test]
fn a_nested_block_entry_names_its_migration() {
    let source = "graph g : App {\n  operator o {\n    on in { out = { parent { child = 1 } } }\n  }\n  . -> o -> .\n}\n";
    refusal_with(
        source,
        "E-CONTRACT-ATTR",
        "`parent { … }` is no longer a form; write `parent = { … }`",
    );
}

#[test]
fn gauge_without_a_connector_is_immobile() {
    // The witness that the identity move is SCOPED: no connector,
    // no injected attribute, no movement.
    let source = std::fs::read_to_string(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/gauge.rkg"),
    )
    .expect("the example reads");
    let compilation = compile_one(&source).expect("valid");
    assert_eq!(
        compilation.hash.to_string(),
        "0d3ad43d9fb09c8626146342f6a50d1f4c387a8de52a47a3134a408cf74cb22e",
        "gauge moved — the injection leaked outside connectors"
    );
}
