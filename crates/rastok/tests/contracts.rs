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

/// Mutates the compiled plan's connector spec and BINDS the adapter:
/// every configuration defect must refuse at `Process::try_new` —
/// before any journaled open — never as a `SpawnFailed`
/// impersonating the OS (D-2026-08-12-1, review rounds).
fn bind_error(mutate: impl Fn(&mut rastok::plan::ConnectorSpec)) -> String {
    let compilation = compile_one(&process_graph(FULL)).expect("valid");
    let mut plan: rastok::plan::ExecutionPlan = (*compilation.plan).clone();
    for node in &mut plan.nodes {
        if let rastok::plan::NodeKind::Connector(spec) = &mut node.kind {
            mutate(spec);
        }
    }
    let validated =
        rastok::plan::ValidatedExecutionPlan::try_from(plan).expect("structurally valid");
    rastok::connectors::Process::try_new(&validated)
        .expect_err("the binding refuses")
        .to_string()
}

#[test]
fn the_binding_refuses_every_stripped_default_individually() {
    // Each Defaulted attribute is defended on its own, at BIND time.
    for name in ["mode", "on_timeout", "args", "max_output"] {
        let message = bind_error(|spec| {
            spec.attributes.retain(|(attr, _)| attr != name);
        });
        assert!(
            message.contains("missing from the travelling sheet"),
            "`{name}` names the violated invariant: {message}"
        );
    }
}

#[test]
fn the_binding_drops_no_argument_silently() {
    let message = bind_error(|spec| {
        for (name, setting) in &mut spec.attributes {
            if name == "args" {
                *setting = rastok::plan::AttrSetting::Value(rastok::cel::Value::List(vec![
                    rastok::cel::Value::Text(String::from("ok")),
                    rastok::cel::Value::Int(7),
                ]));
            }
        }
    });
    assert!(message.contains("not text"), "{message}");
}

#[test]
fn the_binding_refuses_another_contract_and_unknown_codecs() {
    // The Process boundary is structural AND pre-journal: another
    // contract's spec refuses by NAME at bind.
    let message = bind_error(|spec| {
        spec.contract = String::from("Custom");
    });
    assert!(message.contains("`Custom` connector"), "{message}");
    // An unknown codec cannot even reach the binding: the plan
    // validator refuses it first (the adapter's exhaustive match is
    // the belt BEHIND that door).
    let compilation = compile_one(&process_graph(FULL)).expect("valid");
    let mut plan: rastok::plan::ExecutionPlan = (*compilation.plan).clone();
    for node in &mut plan.nodes {
        if let rastok::plan::NodeKind::Connector(spec) = &mut node.kind {
            for (name, setting) in &mut spec.attributes {
                if name == "codec" {
                    *setting = rastok::plan::AttrSetting::Word(String::from("bogus"));
                }
            }
        }
    }
    assert!(
        matches!(
            rastok::plan::ValidatedExecutionPlan::try_from(plan),
            Err(rastok::plan::PlanValidationError::UnknownCodec { .. })
        ),
        "the validator speaks before the binding ever could"
    );
}

#[test]
fn an_effectless_callee_never_validates() {
    // The review's blocker: a delivery-reachable connector with
    // Effect::None would hit the machine's dispatch — the validator
    // refuses it, and the VM keeps a typed belt, never a panic.
    let compilation = compile_one(&process_graph(FULL)).expect("valid");
    let mut plan: rastok::plan::ExecutionPlan = (*compilation.plan).clone();
    for node in &mut plan.nodes {
        if let rastok::plan::NodeKind::Connector(spec) = &mut node.kind {
            spec.effect = rastok::node::Effect::None;
        }
    }
    assert!(
        matches!(
            rastok::plan::ValidatedExecutionPlan::try_from(plan),
            Err(rastok::plan::PlanValidationError::EffectlessCallee { .. })
        ),
        "a callee needs an effect class"
    );
}

#[test]
fn byte_size_is_target_independent_and_bounded() {
    use rastok::contract::{MAX_BYTE_SIZE, byte_size};
    assert_eq!(byte_size("4294967295B"), Some(u64::from(u32::MAX)));
    assert_eq!(
        byte_size("4294967296B"),
        Some(4 * 1024 * 1024 * 1024),
        "u32::MAX + 1 fits — the parse is u64 on EVERY machine"
    );
    assert_eq!(byte_size("4GiB"), Some(MAX_BYTE_SIZE), "the bound itself");
    assert_eq!(byte_size("4294967297B"), None, "one beyond refuses");
    assert_eq!(byte_size("5GiB"), None);
    assert_eq!(
        byte_size("999999999999999999999GiB"),
        None,
        "overflow refuses, never wraps"
    );
    assert_eq!(
        byte_size("8 MiB"),
        Some(8 * 1024 * 1024),
        "interior space tolerated"
    );
    assert_eq!(byte_size("0B"), Some(0), "a zero budget is declarable");
}

#[test]
fn an_unsatisfiable_sheet_never_enters_the_registry() {
    // An empty choice, a selector with no profile, a `nothing`
    // choice word, an inexpressible contract name — each refuses at
    // the door.
    const EMPTY: &[AttributeSpec] = &[AttributeSpec {
        name: "x",
        presence: Presence::Optional,
        domain: ValueDomain::Choice(&[]),
    }];
    assert_eq!(
        Registry::m0().try_with(sheet(EMPTY, None)).unwrap_err(),
        SheetDefect::EmptyChoice("x")
    );
    const NOBODY: ProfileSelector = ProfileSelector {
        attribute: "mode",
        default: None,
        profiles: &[],
    };
    assert_eq!(
        Registry::m0()
            .try_with(sheet(&[], Some(NOBODY)))
            .unwrap_err(),
        SheetDefect::NoProfiles("mode")
    );
    const SENTINEL: &[AttributeSpec] = &[AttributeSpec {
        name: "x",
        presence: Presence::Optional,
        domain: ValueDomain::Choice(&["nothing"]),
    }];
    assert_eq!(
        Registry::m0().try_with(sheet(SENTINEL, None)).unwrap_err(),
        SheetDefect::InexpressibleName("nothing"),
        "the parser turns `nothing` into the no-emission sentinel"
    );
    let unnameable = ConnectorContract {
        name: "Not A Name",
        input_ports: &["in"],
        output_ports: &["out", "err"],
        common: &[],
        selector: None,
        error_set: &[],
    };
    assert_eq!(
        Registry::m0().try_with(unnameable).unwrap_err(),
        SheetDefect::InexpressibleName("Not A Name")
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
