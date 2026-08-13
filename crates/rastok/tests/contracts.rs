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
    compile(
        &[source],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
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
        Registry::try_m0()
            .expect("the standard catalogue holds")
            .try_with(sheet(DUP, None))
            .unwrap_err(),
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
        Registry::try_m0()
            .expect("the standard catalogue holds")
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
        Registry::try_m0()
            .expect("the standard catalogue holds")
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
        Registry::try_m0()
            .expect("the standard catalogue holds")
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
        Registry::try_m0()
            .expect("the standard catalogue holds")
            .try_with(sheet(BAD, None))
            .unwrap_err(),
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
        Registry::try_m0()
            .expect("the standard catalogue holds")
            .try_with(sheet(MISFIT, None))
            .unwrap_err(),
        SheetDefect::InvalidDefault { attribute: "x", .. }
    ));
    // A Choice default must be a word of the domain.
    const OUTSIDE: &[AttributeSpec] = &[AttributeSpec {
        name: "y",
        presence: Presence::Defaulted(DefaultValue::Choice("ghost")),
        domain: ValueDomain::Choice(&["real"]),
    }];
    assert!(matches!(
        Registry::try_m0()
            .expect("the standard catalogue holds")
            .try_with(sheet(OUTSIDE, None))
            .unwrap_err(),
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
        Registry::try_m0()
            .expect("the standard catalogue holds")
            .try_with(sheet(DUP, None))
            .unwrap_err(),
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
        Registry::try_m0()
            .expect("the standard catalogue holds")
            .try_with(sheet(RAW, None))
            .unwrap_err(),
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
fn the_binding_refuses_every_undeclared_adapter_error() {
    // The review's blocker: an error the adapter REALLY produces but
    // the spec does not declare would be refused at the PlanGate
    // after consumption — a stranded open call. The binding proves
    // the subset up front.
    for kind in [
        "SpawnFailed",
        "DecodeError",
        "CaptureFailed",
        "ExecFailed",
        "OutputOverflow",
        // Machine-produced, still part of the bound world's
        // vocabulary — a travelling error_set may not contradict
        // what the scheduler emits.
        "TimeoutExpired",
    ] {
        let message = bind_error(|spec| {
            spec.error_set.retain(|declared| declared != kind);
        });
        assert!(
            message.contains(kind) && message.contains("error_set"),
            "`{kind}` must be declared: {message}"
        );
    }
}

#[test]
fn a_callable_foreign_connector_refuses_at_bind() {
    // The review's blocker: in a single-Partner world, a CALLABLE
    // connector of another contract would receive its intents at
    // the Process adapter and strand them open — the binding
    // refuses it by name. A PURE SOURCE (no incoming edge) is
    // merely skipped: nothing will ever call it.
    let message = bind_error(|spec| {
        spec.contract = String::from("Custom");
    });
    assert!(
        message.contains("callable `Custom` connector has no adapter"),
        "{message}"
    );
    let compilation = compile_one(&process_graph(FULL)).expect("valid");
    let mut plan: rastok::plan::ExecutionPlan = (*compilation.plan).clone();
    for node in &mut plan.nodes {
        if let rastok::plan::NodeKind::Connector(spec) = &mut node.kind {
            spec.contract = String::from("Custom");
        }
    }
    // Sever every edge DELIVERING to the connector: a pure source.
    // The slot is DERIVED from the node, never hard-coded to the
    // lowering order.
    let slot = rastok::plan::Slot::new(
        u32::try_from(
            plan.nodes
                .iter()
                .position(|node| matches!(&node.kind, rastok::plan::NodeKind::Connector(_)))
                .expect("the connector exists"),
        )
        .expect("a small plan"),
    );
    plan.edges
        .retain(|edge| edge.to.as_ref().is_none_or(|to| to.node != Some(slot)));
    let validated = rastok::plan::ValidatedExecutionPlan::try_from(plan)
        .expect("the foreign pure source remains structurally valid");
    assert!(
        rastok::connectors::Process::try_new(&validated).is_ok(),
        "a pure foreign source binds as a no-op"
    );
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
        Registry::try_m0()
            .expect("the standard catalogue holds")
            .try_with(sheet(EMPTY, None))
            .unwrap_err(),
        SheetDefect::EmptyChoice("x")
    );
    const NOBODY: ProfileSelector = ProfileSelector {
        attribute: "mode",
        default: None,
        profiles: &[],
    };
    assert_eq!(
        Registry::try_m0()
            .expect("the standard catalogue holds")
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
        Registry::try_m0()
            .expect("the standard catalogue holds")
            .try_with(sheet(SENTINEL, None))
            .unwrap_err(),
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
        Registry::try_m0()
            .expect("the standard catalogue holds")
            .try_with(unnameable)
            .unwrap_err(),
        SheetDefect::InexpressibleName("Not A Name")
    );
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
    let registry = Registry::try_m0()
        .expect("the standard catalogue holds")
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
    assert!(
        Registry::try_m0()
            .expect("the standard catalogue holds")
            .try_with(sheet(&[], Some(SELECTOR)))
            .is_ok()
    );
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
        Registry::try_m0()
            .expect("the standard catalogue holds")
            .try_with(bad_port)
            .unwrap_err(),
        SheetDefect::InvalidPortName("bad port")
    );
    const BAD_ATTR: &[AttributeSpec] = &[AttributeSpec {
        name: "not a name",
        presence: Presence::Optional,
        domain: ValueDomain::Text,
    }];
    assert_eq!(
        Registry::try_m0()
            .expect("the standard catalogue holds")
            .try_with(sheet(BAD_ATTR, None))
            .unwrap_err(),
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
        Registry::try_m0()
            .expect("the standard catalogue holds")
            .try_with(sheet(&[], Some(LIE)))
            .unwrap_err(),
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
fn the_recovery_form_is_silent_in_every_position() {
    // The Value::Error arm matrix (D-2026-08-12-1, review round):
    // var initializer, local val, state write, tag argument, asset
    // argument — the parser's one voice, no wildcard second.
    for (label, source) in [
        (
            "var initializer",
            "graph g : App {\n  operator o {\n    var v : int = measure { x = 1 }\n    on in { out = data }\n  }\n  . -> o -> .\n}\n",
        ),
        (
            "local val",
            "graph g : App {\n  operator o {\n    on in {\n      val x = on_crash { y = 1 }\n      out = data\n    }\n  }\n  . -> o -> .\n}\n",
        ),
        (
            "state write",
            "graph g : App {\n  operator o {\n    var v : int = 0\n    on in {\n      v = exponential { y = 1 }\n      out = data\n    }\n  }\n  . -> o -> .\n}\n",
        ),
        (
            "tag argument",
            "graph g : App {\n  operator o {\n    on in { out = base64(measure { z = 1 }) { SGVsbG8= } }\n  }\n  . -> o -> .\n}\n",
        ),
        (
            "asset argument",
            "graph g : App {\n  operator o {\n    on in { out = asset(fibonacci { z = 1 }) }\n  }\n  . -> o -> .\n}\n",
        ),
    ] {
        let bag = compile_one(source).expect_err(label);
        assert_eq!(bag.diagnostics().len(), 1, "{label}: {bag:?}");
        assert_eq!(
            bag.diagnostics()[0].code().as_str(),
            "E-PARSE-EXPECTED",
            "{label}: the parser's voice alone"
        );
    }
}

#[test]
fn json_and_nothing_are_unmaskable() {
    // D-2026-08-13-1 (review round 2): the value-form words are
    // RESERVED in every binding-introducing position — a masked
    // `json` would make `json[0]` ambiguous between a document and
    // an indexing. The refusal lands AT THE DECLARATION.
    for (label, source) in [
        (
            "operator val json",
            "graph g : App {\n  operator o {\n    val json = [42]\n    on in { out = data }\n  }\n  . -> o -> .\n}\n",
        ),
        (
            "operator val nothing",
            "graph g : App {\n  operator o {\n    val nothing = 42\n    on in { out = data }\n  }\n  . -> o -> .\n}\n",
        ),
        (
            "handler-local val json",
            "graph g : App {\n  operator o {\n    on in { val json = 1\n      out = data }\n  }\n  . -> o -> .\n}\n",
        ),
        (
            "operator var json",
            "graph g : App {\n  operator o {\n    var json : int = 0\n    on in { out = data }\n  }\n  . -> o -> .\n}\n",
        ),
        (
            "operator var nothing",
            "graph g : App {\n  operator o {\n    var nothing : int = 0\n    on in { out = data }\n  }\n  . -> o -> .\n}\n",
        ),
        (
            "graph val json",
            "graph g : App {\n  val json = 1\n  operator o {\n    on in { out = data }\n  }\n  . -> o -> .\n}\n",
        ),
        (
            "node named json",
            "graph g : App {\n  operator json {\n    on in { out = data }\n  }\n  . -> json -> .\n}\n",
        ),
        (
            "node named nothing",
            "graph g : App {\n  operator nothing {\n    on in { out = data }\n  }\n  . -> nothing -> .\n}\n",
        ),
        (
            "type named json",
            "type json = int\ngraph g : App {\n  operator o {\n    on in { out = data }\n  }\n  . -> o -> .\n}\n",
        ),
        (
            "declared port json",
            "graph g : App {\n  operator o {\n    output json\n    on in { out = data }\n  }\n  . -> o -> .\n}\n",
        ),
    ] {
        let bag = compile_one(source).expect_err(label);
        assert!(
            bag.diagnostics().iter().any(|d| {
                d.code().as_str() == "E-NAME-RESERVED" && d.message().contains("reserved word")
            }),
            "{label}: {bag:?}"
        );
    }
    // The value forms themselves stay whole.
    compile_one("graph g : App {\n  operator o {\n    on in { out = json {\"a\": 1} }\n  }\n  . -> o -> .\n}\n")
        .expect("json object lives");
    compile_one(
        "graph g : App {\n  operator o {\n    on in { out = json [1, 2] }\n  }\n  . -> o -> .\n}\n",
    )
    .expect("json array lives");
    compile_one(
        "graph g : App {\n  operator o {\n    on in { out = nothing }\n  }\n  . -> o -> .\n}\n",
    )
    .expect("nothing lives");
    // With no maskable binding, `json[0]` IS the document `[0]`.
    let compilation = compile_one(
        "graph g : App {\n  operator o {\n    on in { out = json[0] }\n  }\n  . -> o -> .\n}\n",
    )
    .expect("an unspaced document array");
    let _ = compilation;
}

#[test]
fn a_json_recovery_follows_the_position_terminators() {
    // The review's blocker: `;`-separated statements, struct
    // fields, paren tuples, and MULTILINE faulty json all recover
    // at their own terminator — one voice, siblings preserved.
    for (label, source) in [
        (
            "semicolon statement",
            "graph g : App {\n  operator o {\n    on in { out = json {\"a\": ???}; out = data }\n  }\n  . -> o -> .\n}\n",
        ),
        (
            "struct field",
            "graph g : App {\n  operator o {\n    on in { out = { bad = json {\"a\": ???}, ok = 1 } }\n  }\n  . -> o -> .\n}\n",
        ),
        (
            "multiline json",
            "graph g : App {\n  operator o {\n    on in {\n      out = json {\n        \"a\": ???\n      }\n      out = data\n    }\n  }\n  . -> o -> .\n}\n",
        ),
        (
            "paren position (comma boundary)",
            "graph g : App {\n  operator o {\n    on in { out = asset(json {\"a\": ???}, \"text/plain\"); out = data }\n  }\n  . -> o -> .\n}\n",
        ),
        (
            "paren position (close-paren boundary)",
            "graph g : App {\n  operator o {\n    on in { out = asset(json {\"a\": ???}); out = data }\n  }\n  . -> o -> .\n}\n",
        ),
        // A follower that opens no document and spells no scalar is
        // a CEL continuation beginning with the word `json`
        // (D-2026-08-13-1) — CEL's voice, CEL's recovery.
        (
            "host-alien first byte",
            "graph g : App {\n  operator o {\n    on in { out = json @; out = data }\n  }\n  . -> o -> .\n}\n",
        ),
        (
            "quoted CEL follower",
            "graph g : App {\n  operator o {\n    on in { out = json 'x'; out = data }\n  }\n  . -> o -> .\n}\n",
        ),
        // A former scalar spelling is CEL too (`{`/`[` are the ONLY
        // special followers, D-2026-08-13-1 review round): CEL's
        // voice, CEL's recovery, sibling preserved.
        (
            "scalar number",
            "graph g : App {\n  operator o {\n    on in { out = json 19.99; out = data }\n  }\n  . -> o -> .\n}\n",
        ),
        (
            "scalar string",
            "graph g : App {\n  operator o {\n    on in { out = json \"s\"; out = data }\n  }\n  . -> o -> .\n}\n",
        ),
        (
            "scalar word",
            "graph g : App {\n  operator o {\n    on in { out = json true; out = data }\n  }\n  . -> o -> .\n}\n",
        ),
    ] {
        let expected = match label {
            "host-alien first byte"
            | "quoted CEL follower"
            | "scalar number"
            | "scalar string"
            | "scalar word" => "E-CEL-EXPR",
            _ => "E-JSON-VALUE",
        };
        let bag = compile_one(source).expect_err(label);
        assert_eq!(bag.diagnostics().len(), 1, "{label}: {bag:?}");
        assert_eq!(bag.diagnostics()[0].code().as_str(), expected, "{label}");
    }
    // The positive twin proves the SIBLING really lives there: with
    // the fault healed, the handler carries BOTH emissions.
    let healed = "graph g : App {\n  operator o {\n    on in { out = json {\"a\": 1}; out = data }\n  }\n  . -> o -> .\n}\n";
    let compilation = compile_one(healed).expect("valid");
    let handler = &compilation.plan.plan().handlers[0];
    let emissions = handler
        .body
        .iter()
        .filter(|step| matches!(step, rastok::plan::Step::Emit { .. }))
        .count();
    assert_eq!(emissions, 2, "both emissions live in the tree");
}

#[test]
fn a_semicolon_at_json_depth_is_a_strong_boundary() {
    // `;` cannot appear in JSON outside a string: the walk stops at
    // ANY depth, the sibling survives, and the two residual closer
    // errors are the file's REAL brace imbalance (the unterminated
    // json `{` — accepted as necessarily ambiguous).
    let source = "graph g : App {\n  operator o {\n    on in { out = json { \"a\": ???; out = data } }\n  }\n  . -> o -> .\n}\n";
    let bag = compile_one(source).expect_err("must refuse");
    let errors: Vec<_> = bag
        .diagnostics()
        .iter()
        .filter(|d| d.code().as_str().starts_with("E-"))
        .collect();
    assert_eq!(errors.len(), 3, "{bag:?}");
    assert_eq!(errors[0].code().as_str(), "E-JSON-VALUE");
    assert!(
        errors[1..]
            .iter()
            .all(|d| d.code().as_str() == "E-PARSE-EXPECTED"),
        "the tail is the real imbalance: {bag:?}"
    );
}

#[test]
fn json_trivia_belongs_to_the_host() {
    // A same-line comment between the tag and the body is HOST
    // trivia — the reader never sees it, and the identity is the
    // same as the uncommented spelling.
    let commented = "graph g : App {\n  operator o {\n    on in { out = json /* c */ {\"a\": 1} }\n  }\n  . -> o -> .\n}\n";
    let plain = "graph g : App {\n  operator o {\n    on in { out = json {\"a\": 1} }\n  }\n  . -> o -> .\n}\n";
    let a = compile_one(commented).expect("valid");
    let b = compile_one(plain).expect("valid");
    assert_eq!(
        a.hash.to_string(),
        b.hash.to_string(),
        "host trivia is not identity"
    );
    // An NBSP is NOT host trivia: it opens no document and spells
    // no scalar, so the value is a CEL continuation beginning with
    // the word `json` — the boundary never depends on Unicode
    // trim_start (D-2026-08-13-1).
    let nbsp = "graph g : App {\n  operator o {\n    on in { out = json \u{a0}1 }\n  }\n  . -> o -> .\n}\n";
    let bag = compile_one(nbsp).expect_err("must refuse");
    assert_eq!(bag.diagnostics().len(), 1, "{bag:?}");
    assert_eq!(bag.diagnostics()[0].code().as_str(), "E-CEL-EXPR");
    // A comment crossing the line means NO body on the line: bare
    // word — never a truncated JSON literal.
    let crossing = "graph g : App {\n  operator o {\n    on in { out = json /* c\n*/ 1 }\n  }\n  . -> o -> .\n}\n";
    let bag = compile_one(crossing).expect_err("must refuse");
    assert!(
        bag.diagnostics()
            .iter()
            .all(|d| d.code().as_str() != "E-JSON-VALUE"),
        "bare word, not a JSON literal: {bag:?}"
    );
}

#[test]
fn dead_and_reserved_parens_are_opaque_too() {
    // Including DELIMITERS inside both quote styles: a quoted paren
    // is content, never structure (the review's probe).
    for (label, source) in [
        (
            "dead initializer with alien parens",
            "graph g : App {\n  operator o {\n    on in { out = on_crash('@') { weird = '@' } }\n  }\n  . -> o -> .\n}\n",
        ),
        (
            "reserved format with alien parens",
            "graph g : App {\n  operator o {\n    on in { out = protobuf('@') { message X {} } }\n  }\n  . -> o -> .\n}\n",
        ),
        (
            "single-quoted open paren",
            "graph g : App {\n  operator o {\n    on in { out = on_crash('(') { weird = '@' }; out = data }\n  }\n  . -> o -> .\n}\n",
        ),
        (
            "single-quoted close paren",
            "graph g : App {\n  operator o {\n    on in { out = protobuf(')') { message X {} }; out = data }\n  }\n  . -> o -> .\n}\n",
        ),
        (
            "double-quoted open paren",
            "graph g : App {\n  operator o {\n    on in { out = on_crash(\"(\") { weird = '@' }; out = data }\n  }\n  . -> o -> .\n}\n",
        ),
        (
            "double-quoted close paren",
            "graph g : App {\n  operator o {\n    on in { out = protobuf(\")\") { message X {} }; out = data }\n  }\n  . -> o -> .\n}\n",
        ),
    ] {
        let bag = compile_one(source).expect_err(label);
        assert_eq!(bag.diagnostics().len(), 1, "{label}: {bag:?}");
    }
}

#[test]
fn an_invalid_selector_selects_no_implicit_profile() {
    // `mode =` is PresentInvalid — the compiler never pretends
    // `mode = job` about a mode the author wrote and got wrong: one
    // parser voice, no belongs-to-job claim.
    let source = "graph g : App {\n  connector shell : Process {\n    mode =\n    restart = whatever\n    command = \"printf\"\n    codec = lines\n    effect = idempotent\n    behavior = deterministic\n  }\n  . -> shell -> .\n}\n";
    let bag = compile_one(source).expect_err("must refuse");
    assert_eq!(bag.diagnostics().len(), 1, "{bag:?}");
    assert_eq!(bag.diagnostics()[0].code().as_str(), "E-PARSE-EXPECTED");
}

#[test]
fn a_duplicate_attribute_is_syntactic() {
    // `command =` then `command = "ok"`: the first value failed, the
    // NAME was still written — the duplicate refuses.
    let source = "graph g : App {\n  connector shell : Process {\n    command =\n    command = \"printf\"\n    codec = lines\n    effect = idempotent ; behavior = deterministic\n  }\n  . -> shell -> .\n}\n";
    let bag = compile_one(source).expect_err("must refuse");
    assert!(
        bag.diagnostics()
            .iter()
            .any(|d| d.message().contains("declared twice")),
        "{bag:?}"
    );
}

#[test]
fn an_omitted_value_never_absorbs_the_next_line() {
    // The review's blocker: `command =` must diagnose EXACTLY ONCE,
    // at the empty position, and the NEXT attribute must survive —
    // written-but-invalid is neither "missing" nor defaultable, so
    // no Required cascade and no MissingCodec artifact.
    let source = "graph g : App {\n  connector shell : Process {\n    command =\n    codec = lines\n    effect = idempotent\n    behavior = deterministic\n  }\n  . -> shell -> .\n}\n";
    let bag = compile_one(source).expect_err("must refuse");
    assert_eq!(bag.diagnostics().len(), 1, "{bag:?}");
    let diagnostic = &bag.diagnostics()[0];
    assert_eq!(diagnostic.code().as_str(), "E-PARSE-EXPECTED");
    assert!(
        diagnostic
            .message()
            .contains("expected a value before the end of the line"),
        "{diagnostic:?}"
    );
    // The span sits on the `command =` line (line 3 starts at 39).
    let span = diagnostic.spans()[0].range();
    let line = source[..span.start].matches('\n').count() + 1;
    assert_eq!(line, 3, "the diagnostic points at the EMPTY position");
    // In a handler: `val x =` keeps the following emission.
    let source = "graph g : App {\n  operator o {\n    on in {\n      val x =\n      out = data\n    }\n  }\n  . -> o -> .\n}\n";
    let bag = compile_one(source).expect_err("must refuse");
    assert_eq!(bag.diagnostics().len(), 1, "{bag:?}");
    assert_eq!(bag.diagnostics()[0].code().as_str(), "E-PARSE-EXPECTED");
}

#[test]
fn a_json_error_resynchronizes_at_the_line_boundary() {
    // The faulty value is consumed to its line's end — one voice,
    // and the next emission survives in the tree.
    let source = "graph g : App {\n  operator o {\n    on in {\n      out = json {\"a\": ???}\n      out = data\n    }\n  }\n  . -> o -> .\n}\n";
    let bag = compile_one(source).expect_err("must refuse");
    assert_eq!(bag.diagnostics().len(), 1, "{bag:?}");
    assert!(
        bag.diagnostics()[0]
            .message()
            .contains("the JSON value does not read"),
        "{bag:?}"
    );
}

#[test]
fn an_omitted_expression_always_diagnoses() {
    // The review's blocker: the recovery form is Value::Error, never
    // an empty CelExpr a user could also produce — a genuinely
    // omitted expression must diagnose, everywhere.
    for (label, source) in [
        (
            "out =",
            "graph g : App {\n  operator o {\n    on in {\n      out =\n    }\n  }\n  . -> o -> .\n}\n",
        ),
        (
            "val x =",
            "graph g : App {\n  val x =\n  operator o {\n    on in { out = data }\n  }\n  . -> o -> .\n}\n",
        ),
        (
            "attribute without a value",
            "graph g : App {\n  connector shell : Process {\n    command =\n    codec = lines\n    effect = idempotent ; behavior = deterministic\n  }\n  . -> shell -> .\n}\n",
        ),
        (
            "guard without an expression",
            "graph g : App {\n  operator o {\n    on in where {\n      out = data\n    }\n  }\n  . -> o -> .\n}\n",
        ),
    ] {
        let bag = compile_one(source).expect_err(label);
        assert!(
            !bag.diagnostics().is_empty(),
            "`{label}` must speak: {bag:?}"
        );
    }
}

#[test]
fn a_reserved_format_body_defers_once_and_opaquely() {
    // `avro { … }` is a reserved word with no living form: ONE
    // named deferral, the body consumed opaquely — never five
    // struct-parse errors.
    let source = "graph g : App {\n  operator o {\n    on in { out = avro { \"type\": \"record\", \"name\": \"X\" } }\n  }\n  . -> o -> .\n}\n";
    let bag = compile_one(source).expect_err("reserved defers");
    assert_eq!(bag.diagnostics().len(), 1, "{bag:?}");
    assert!(
        bag.diagnostics()[0]
            .message()
            .contains("a reserved data-format body"),
        "{bag:?}"
    );
}

#[test]
fn a_reserved_format_body_is_lexically_opaque() {
    // The body never meets the host lexer: characters RKG cannot
    // tokenize, braces inside quotes, and nesting all pass under
    // the ONE deferral; an unterminated block is the raw lexer's
    // own fault.
    for (label, body) in [
        ("host-unknown characters", "option strange = '@';"),
        ("braces inside strings", r#"x = "{{{" ; y = '}'"#),
        ("nested blocks", "outer { inner { '@' } }"),
    ] {
        let source = format!(
            "graph g : App {{\n  operator o {{\n    on in {{ out = protobuf {{ {body} }} }}\n  }}\n  . -> o -> .\n}}\n"
        );
        let bag = compile_one(&source).expect_err(label);
        assert_eq!(bag.diagnostics().len(), 1, "{label}: {bag:?}");
        assert!(
            bag.diagnostics()[0]
                .message()
                .contains("a reserved data-format body"),
            "{label}: {bag:?}"
        );
    }
    // Unterminated: the named deferral is present; the truncated
    // remainder ALSO surfaces its missing closers (pinned as-is —
    // an unterminated block genuinely breaks everything after it).
    let source = "graph g : App {\n  operator o {\n    on in { out = avro { never closed\n";
    let bag = compile_one(source).expect_err("unterminated refuses");
    assert!(
        bag.diagnostics()
            .iter()
            .any(|d| d.message().contains("a reserved data-format body")),
        "{bag:?}"
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
