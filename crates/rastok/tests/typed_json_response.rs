//! A Connector refines an existing codec-owned output, never its port inventory.

#![allow(clippy::expect_used, clippy::panic)]

#[path = "support/call_driver.rs"]
mod call_driver;
use call_driver::CallDriver;

use rastok::compiler::{Compilation, compile};
use rastok::connector::contract::ContractCatalog;
use rastok::diagnostics::Bag;

fn checked(source: &str) -> Result<Compilation, Bag> {
    compile(
        &[source],
        &ContractCatalog::try_m0().expect("standard contracts"),
    )
}

fn source(codec: &str, ports: &str) -> String {
    format!(
        r#"
type Amount = decimal(38,18)
type ModelResponse = struct {{
  usage: struct {{ input_tokens: uint @measure(id = input_tokens, unit = "token") }}
  cost: Amount @measure(id = amount, unit = "USD")
}}
graph app : App {{
  connector model : HttpClient {{
    url = "https://example.test/model"
    codec = {codec}
    behavior = probabilistic
    timeout = duration("1s")
    {ports}
  }}
  in -> model -> out
  model.exit -> nothing
}}
"#
    )
}

#[test]
fn a_json_response_refines_only_the_existing_output_and_retains_its_annotations() {
    let compilation = checked(&source("json", "output out: ModelResponse"))
        .expect("the JSON codec deserializes the declared response type");
    let model = &compilation.plan.plan().nodes[1];
    assert_eq!(model.ports.len(), 3);
    let out = model
        .ports
        .iter()
        .find(|port| port.port.name() == "out")
        .expect("out");
    assert!(matches!(
        out.ty,
        Some(rastok::datatype::DataType::Struct(_))
    ));
    assert!(!out.annotations.is_empty());
    let input = model
        .ports
        .iter()
        .find(|port| port.port.name() == "in")
        .expect("in");
    assert_eq!(input.ty, Some(rastok::datatype::DataType::Variant));
}

#[test]
fn typed_output_refinement_never_adds_ports_or_changes_non_json_contracts() {
    for (codec, ports, needle) in [
        ("json", "input in: ModelResponse", "input"),
        ("json", "output extra: ModelResponse", "extra"),
        ("json", "output exit: ModelResponse", "exit"),
        ("text", "output out: ModelResponse", "codec"),
        ("bytes", "output out: ModelResponse", "codec"),
        ("json", "output out", "type"),
        (
            "json",
            "output out: ModelResponse\noutput out: ModelResponse",
            "already",
        ),
    ] {
        let bag = checked(&source(codec, ports)).expect_err("invalid refinement");
        assert!(
            bag.diagnostics().iter().any(|d| {
                d.code().as_str() == "E-PORT-CONFLICT" && d.message().contains(needle)
            }),
            "{ports}: {bag:?}"
        );
    }
}

#[test]
fn default_json_output_is_identity_neutral_and_refinement_round_trips() {
    let default = checked(&source("json", "")).expect("untyped JSON");
    let explicit = checked(&source("json", "output out: variant")).expect("explicit default");
    assert_eq!(default.hash, explicit.hash);
    assert_eq!(default.plan, explicit.plan);
    let typed = checked(&source("json", "output out: ModelResponse")).expect("typed response");
    assert_ne!(default.hash, typed.hash);
    let alias = checked(
        &source("json", "output out: Alias")
            .replace("graph app", "type Alias = ModelResponse\ngraph app"),
    )
    .expect("alias");
    assert_eq!(typed.hash, alias.hash);
    let bytes = rastok::plan::encode(&typed.plan, &typed.hash);
    let decoded = rastok::plan::decode(&bytes).expect("independent validation");
    assert_eq!(typed.plan, decoded.0);
    assert_eq!(typed.hash, decoded.1);
}

#[test]
fn projected_json_is_an_explicit_plan_profile_not_a_new_struct_type() {
    let strict_source = source("json", "output out: ModelResponse");
    let strict = checked(&strict_source).expect("strict response");
    let projected =
        checked(&strict_source.replace("codec = json", "codec = json\njson_profile = projected"))
            .expect("opt-in projected response");
    assert_ne!(strict.hash, projected.hash);
    assert_eq!(
        strict.plan.plan().nodes[1].ports,
        projected.plan.plan().nodes[1].ports
    );
    let encoded = rastok::plan::encode(&projected.plan, &projected.hash);
    assert_eq!(
        rastok::plan::decode(&encoded).expect("independent profile check"),
        (projected.plan.clone(), projected.hash)
    );
    let mut without_profile = projected.plan.plan().clone();
    let rastok::plan::NodeKind::Connector(spec) = &mut without_profile.nodes[1].body else {
        panic!("connector")
    };
    spec.attributes.retain(|(name, _)| name != "json_profile");
    assert_eq!(
        &without_profile,
        strict.plan.plan(),
        "omission changes no existing plan field"
    );
}

#[test]
fn projected_json_requires_the_http_json_struct_profile() {
    for (codec, ports, profile) in [
        ("json", "", "projected"),
        ("json", "output out: variant", "projected"),
        ("json", "output out: array<int>", "projected"),
        ("json", "output out: int", "projected"),
        ("text", "", "projected"),
        ("bytes", "", "projected"),
        ("json", "output out: ModelResponse", "lenient"),
    ] {
        let program = source(codec, ports).replace(
            "behavior =",
            &format!("json_profile = {profile}\nbehavior ="),
        );
        let bag =
            checked(&program).expect_err("profile is neither guessed nor portable to other codecs");
        assert!(
            bag.diagnostics()
                .iter()
                .any(|d| d.code().as_str() == "E-CONTRACT-ATTR"
                    && d.message().contains("json_profile")),
            "{program}: {bag:?}"
        );
    }
}

#[test]
fn the_independent_validator_rechecks_projected_json_restrictions() {
    use rastok::plan::{AttrSetting, NodeKind, ValidatedExecutionPlan};
    for (codec, ports, profile) in [
        ("json", "", "projected"),
        ("text", "", "projected"),
        ("bytes", "", "projected"),
        ("json", "output out: ModelResponse", "lenient"),
    ] {
        let compilation = checked(&source(codec, ports)).expect("valid base");
        let mut forged = compilation.plan.plan().clone();
        let NodeKind::Connector(spec) = &mut forged.nodes[1].body else {
            panic!("connector")
        };
        spec.attributes.push((
            String::from("json_profile"),
            AttrSetting::Word(profile.into()),
        ));
        spec.attributes.sort_by(|a, b| a.0.cmp(&b.0));
        let error = ValidatedExecutionPlan::try_new(forged).expect_err("hostile profile");
        assert!(error.to_string().contains("json_profile"), "{error}");
    }
}

#[test]
fn unsupported_json_target_encodings_refuse_at_check() {
    for ty in [
        "bytes",
        "timestamp",
        "float",
        "uuid",
        "union { A { x: int }, B }",
    ] {
        let bag = checked(&source(
            "json",
            &format!("output out: struct {{ value: {ty} }}"),
        ))
        .expect_err("no guessed encoding");
        assert!(
            bag.diagnostics()
                .iter()
                .any(|d| d.code().as_str() == "E-TYPE-INVALID"
                    && d.message().contains("JSON deserialization")),
            "{bag:?}"
        );
    }
}

#[test]
fn the_independent_validator_rejects_forged_codec_capabilities() {
    use rastok::{
        datatype::DataType,
        plan::{AttrSetting, ValidatedExecutionPlan},
    };
    let compilation = checked(&source("json", "")).expect("unrefined base");
    for ty in [DataType::BYTES, DataType::FLOAT, DataType::Uuid] {
        let mut forged = compilation.plan.plan().clone();
        forged.nodes[1]
            .ports
            .iter_mut()
            .find(|port| port.port.name() == "out")
            .expect("out")
            .ty = Some(ty);
        assert!(ValidatedExecutionPlan::try_new(forged).is_err());
    }
    let typed = checked(&source("json", "output out: ModelResponse")).expect("typed response");
    let mut forged = typed.plan.plan().clone();
    let rastok::plan::NodeKind::Connector(spec) = &mut forged.nodes[1].body else {
        panic!("connector")
    };
    spec.attributes
        .iter_mut()
        .find(|(name, _)| name == "codec")
        .expect("codec")
        .1 = AttrSetting::Word(String::from("text"));
    assert!(
        ValidatedExecutionPlan::try_new(forged).is_err(),
        "text has no JSON deserialization capability"
    );
}

#[test]
fn malformed_native_or_undeserialized_data_cannot_enter_the_outbound_journal() {
    use rastok::{
        connector::{driver::*, exchange::*, stdlib::*},
        envelope::{Data, Encoding, Namespace, Value},
        journal::MemJournal,
        node,
        plan::GraphVersion,
        runtime::execution::Execution,
    };
    use std::collections::BTreeMap;
    struct Driver {
        data: Data,
        responses: Vec<CallResponse>,
    }
    impl OutboundCall for Driver {
        fn prepare_request(
            &mut self,
            _: &node::Path,
            _: &Data,
            _: &BTreeMap<String, Value>,
        ) -> RequestPreparation {
            RequestPreparation::Prepared(PreparedRequest::seal(b"{}".to_vec()))
        }
        fn start(&mut self, intent: &CallOpen, _: &rastok::envelope::Envelope) -> CallStart {
            self.responses.push(CallResponse {
                call_id: intent.call_id,
                completion: Completion::http_client(
                    self.data.clone(),
                    Namespace::new(),
                    HttpClientExit::Completed { status: 200 },
                ),
            });
            CallStart::Started
        }
        fn poll_responses(&mut self) -> Vec<CallResponse> {
            std::mem::take(&mut self.responses)
        }
        fn scheduler_completion(&mut self, _: &node::Path, _: SchedulerCompletion) -> Completion {
            Completion::http_client_exit(HttpClientExit::TimeoutExpired)
        }
    }
    let compilation =
        checked(&source("json", "output out: ModelResponse")).expect("typed response");
    for data in [
        Data::materialized(Value::Map(BTreeMap::from([(
            String::from("bad"),
            Value::Text(String::from("NEVER-RETAIN")),
        )]))),
        Data::encoded(Encoding::Raw, b"NEVER-RETAIN".to_vec()),
    ] {
        let mut driver = Driver {
            data,
            responses: Vec::new(),
        };
        let mut execution = Execution::new(&compilation.plan, GraphVersion(1));
        let mut journal = MemJournal::new(GraphVersion(1));
        execution
            .accept_input(
                &mut journal,
                IngressProposal {
                    provenance: IngressProvenance::single(
                        node::Path::new(["probe"]).expect("path"),
                    ),
                    draft: EnvelopeDraft {
                        port: node::Port::new("in").expect("port"),
                        data: Data::materialized(Value::Map(BTreeMap::new())),
                        ext: Namespace::new(),
                    },
                    sequence: IngressSequence::Next,
                    session: None,
                },
            )
            .expect("admission");
        execution
            .run_ready_with_drivers(&mut journal, &mut CallDriver(&mut driver))
            .expect("Call opened");
        let retained = journal.records().to_vec();
        let report = execution
            .poll_drivers(&mut journal, &mut CallDriver(&mut driver))
            .expect("the Driver fault is a rejected observation");
        assert_eq!(report.settled, 0);
        assert_eq!(report.rejected.len(), 1);
        assert!(!report.rejected[0].1.contains("NEVER-RETAIN"));
        assert_eq!(journal.records(), retained);
        assert!(!format!("{:?}", journal.records()).contains("NEVER-RETAIN"));
    }
}

#[test]
fn the_lab_emits_the_same_declared_struct_and_needs_no_downstream_extractor() {
    let source = source("json", "output out: ModelResponse")
        + r#"
lab app {
  simulation nominal on app {
    mock model {
      on in {
        out = { usage = { input_tokens = 120u } ; cost = decimal("0.0042") as Amount }
        exit = Completed { status = 200 }
      }
    }
    scenario response {
      app.in = json! {}
      expect model.out count == 1 where data.usage.input_tokens == 120u
    }
  }
}
"#;
    let compilation = checked(&source).expect("typed Lab");
    let first = rastok::lab::run(
        &compilation.plan,
        rastok::plan::GraphVersion(1),
        &compilation.labs[0],
    );
    assert!(first[0].report.passed(), "{:?}", first[0].report);
    let second = rastok::lab::run(
        &compilation.plan,
        rastok::plan::GraphVersion(1),
        &compilation.labs[0],
    );
    assert_eq!(first, second);
    let observations =
        rastok::accounting::accounting_observations(&compilation.plan, &first[0].records, None)
            .expect("typed source evidence");
    assert_eq!(observations.len(), 1, "no second extraction emission");
    assert_eq!(observations[0].measures().len(), 2);
    assert_eq!(
        observations[0]
            .measures()
            .iter()
            .find(|sample| sample.id().as_str() == "input_tokens")
            .unwrap()
            .quantity(),
        rastok::accounting::ExactDecimal::new(120, 0).unwrap()
    );
    assert_eq!(
        observations[0]
            .measures()
            .iter()
            .find(|sample| sample.id().as_str() == "amount")
            .unwrap()
            .quantity(),
        rastok::accounting::ExactDecimal::new(42, 4).expect("exact 0.0042")
    );
}
