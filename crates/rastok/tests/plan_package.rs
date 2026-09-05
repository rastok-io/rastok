//! The plan package's verification contract, per
//! `docs/architecture/plan-package.md`: the round trip preserves the
//! plan, its declared identity, AND its execution — the boundary-value
//! matrix runs on both sides of the trip; the example's package is
//! pinned like a golden; and every refusal is typed — a bumped
//! version, a corrupted body byte, a corrupted declared hash, an
//! out-of-bounds slot, a non-canonical envelope, a truncated package,
//! and foreign bytes.

#![allow(clippy::disallowed_types, clippy::disallowed_methods)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use rastok::canonical_cbor::{Value as Cbor, decode, encode};
use rastok::compiler::compile;
use rastok::contract::ContractCatalog;
use rastok::plan::{self, DecodeError};
use std::path::PathBuf;

fn example() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/echo.rkg")
}

fn packaged_example() -> (
    rastok::plan::ValidatedExecutionPlan,
    rastok::plan::SemanticHash,
    Vec<u8>,
) {
    let source = std::fs::read_to_string(example()).expect("the example reads");
    let compilation = compile(
        &[source.as_str()],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    let bytes = plan::encode(&compilation.plan, &compilation.hash);
    (compilation.plan, compilation.hash, bytes)
}

fn packaged_loop() -> (
    rastok::plan::ValidatedExecutionPlan,
    rastok::plan::SemanticHash,
    Vec<u8>,
) {
    let source = r#"
graph countdown : App + Loop {
  input in: text
  output out: text
  Loop.fuel = 3

  operator relay {
    input in: text
    output out: text
    on in { out = data }
  }

  . -> relay
  on relay.out { out = Loop.state.data }
}
"#;
    let compilation = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("the first categorical Loop crosses source lowering");
    assert!(compilation.warnings.is_empty());
    let bytes = plan::encode(&compilation.plan, &compilation.hash);
    (compilation.plan, compilation.hash, bytes)
}

fn packaged_measure_loop() -> (
    rastok::plan::ValidatedExecutionPlan,
    rastok::plan::SemanticHash,
    Vec<u8>,
) {
    let source = r#"
graph countdown : App + Loop {
  input in: int
  output out: int
  Loop.measure = data
  on in where data <= 0 { out = data }
  on in where data > 0 { next = data - 1 }
}
"#;
    let compilation = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("the checked measure is a valid categorical Loop");
    let bytes = plan::encode(&compilation.plan, &compilation.hash);
    (compilation.plan, compilation.hash, bytes)
}

#[test]
fn the_round_trip_preserves_the_plan_and_its_identity() {
    let (plan, hash, bytes) = packaged_example();
    let (decoded, declared) = plan::decode(&bytes).expect("the package opens");
    assert_eq!(decoded, plan, "structural equality across the trip");
    assert_eq!(declared, hash, "the identity travels intact");
}

#[test]
fn every_emission_form_preserves_its_ordered_meaning_through_the_package() {
    let source = r#"
graph builders : App {
  input in: struct { n: int }
  output copied: struct { n: int }
  output patched: struct { n: int }
  output fresh: struct { n: int }
  output scalar: int
  output quiet: int
  operator build {
    input in: struct { n: int }
    val bump = 2
    on in {
      copied = .
      patched = { data.n = data.n + 1; data.n = data.n + bump; meta.app.mark = "patched" }
      fresh = { n = data.n + bump }
      scalar = data.n + bump
      quiet = nothing
    }
  }
  in -> build
  build.copied -> copied
  build.patched -> patched
  build.fresh -> fresh
  build.scalar -> scalar
  build.quiet -> quiet
}
lab builders {
  simulation construction on app {
    scenario forms "all constructors cross the same package boundary" {
      app.in = { n = 5 }
      expect app.copied count == 1 where data.n == 5
      expect app.patched count == 1 where data.n == 7 && meta.app.mark == "patched"
      expect app.fresh count == 1 where data.n == 7
      expect app.scalar count == 1 where data == 7
      expect nothing app.quiet
    }
  }
}
"#;
    let compilation = compile(&[source], &ContractCatalog::try_m0().unwrap()).unwrap();
    let bytes = plan::encode(&compilation.plan, &compilation.hash);
    let (decoded, hash) = plan::decode(&bytes).unwrap();
    assert_eq!(decoded, compilation.plan);
    assert_eq!(hash, compilation.hash);
    assert_eq!(plan::encode(&decoded, &hash), bytes);

    let emissions: Vec<_> = decoded.handlers[0]
        .body
        .iter()
        .filter_map(|step| match step {
            plan::DispatchStep::Emit { value, .. } => Some(value),
            _ => None,
        })
        .collect();
    assert!(matches!(
        emissions.as_slice(),
        [
            plan::Emission::Identity,
            plan::Emission::Patch(_),
            plan::Emission::FreshData(_),
            plan::Emission::Data(_),
            plan::Emission::Nothing,
        ]
    ));

    let original = rastok::lab::run(
        &compilation.plan,
        plan::GraphVersion(1),
        &compilation.labs[0],
    );
    let restored = rastok::lab::run(&decoded, plan::GraphVersion(1), &compilation.labs[0]);
    assert_eq!(original.len(), 1);
    assert_eq!(restored.len(), 1);
    assert!(
        original[0].report.passed(),
        "{:?}",
        original[0].report.failures
    );
    assert!(
        restored[0].report.passed(),
        "{:?}",
        restored[0].report.failures
    );
    assert_eq!(original[0].records, restored[0].records);
}

#[test]
fn v15_retains_one_closed_loop_and_its_spatial_graph_semantics() {
    let (compiled, hash, bytes) = packaged_loop();
    let root = compiled.owners.first().expect("the root owner");
    let [rastok::plan::AspectSpec::Loop(spec)] = root.aspects.as_slice() else {
        panic!("the root carries exactly one closed Loop specification");
    };
    assert_eq!(
        spec.fuel.as_ref().expect("fuel").resolved,
        rastok::value::Value::Int(3)
    );
    assert!(spec.delay.is_none());
    assert!(compiled.boundary.iter().any(|port| {
        port.port.name() == "next"
            && port.direction == rastok::node::Direction::Output
            && port.ty.as_ref() == Some(&rastok::datatype::DataType::Text)
    }));
    assert!(matches!(
        compiled.handlers.as_slice(),
        [
            rastok::plan::Handler {
                owner: rastok::plan::HandlerOwner::Node(_),
                ..
            },
            rastok::plan::Handler {
                owner: rastok::plan::HandlerOwner::Graph(_),
                activation: rastok::node::HandlerActivation::ChildPort { .. },
                ..
            }
        ]
    ));

    let (decoded, declared) = plan::decode(&bytes).expect("v15 revalidates Loop Registry-free");
    assert_eq!(decoded, compiled);
    assert_eq!(declared, hash);

    use rastok::boundary::{
        Draft, InboundEnvelopeProvenance, InboundSubmission, SubmissionSequence,
    };
    use rastok::envelope::{Data, Namespace};
    use rastok::journal::MemJournal;
    use rastok::plan::GraphVersion;
    use rastok::runtime::vm::Vm;
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(&decoded, GraphVersion(1));
    vm.inject(
        &mut journal,
        InboundSubmission {
            provenance: InboundEnvelopeProvenance::single(
                rastok::node::Path::new(["probe"]).unwrap(),
            ),
            port: rastok::node::Port::new("in").unwrap(),
            draft: Draft {
                data: Data::materialized(rastok::value::Value::Text(String::from("go"))),
                ext: Namespace::new(),
            },
            sequence: SubmissionSequence::Next,
        },
    )
    .expect("the spatial Loop root is admitted");
    vm.drain(&mut journal)
        .expect("the relayed child output reaches its Graph handler");
    assert_eq!(vm.outputs().len(), 1);
    assert_eq!(
        rastok::runtime::data_value(vm.outputs()[0].envelope.data()).unwrap(),
        rastok::value::Value::Text(String::from("go"))
    );

    let registry = rastok::connectors::DriverRegistry::default();
    let mut bindings = rastok::connectors::RuntimeBindings::try_new(&decoded, &registry)
        .expect("a connector-free Loop needs no Driver");
    vm.pump(&mut journal, &mut bindings)
        .expect("a connector-free boundary pump remains inert");
}

#[test]
fn v15_round_trips_the_checked_loop_proof() {
    let (compiled, hash, bytes) = packaged_measure_loop();
    let (decoded, declared) = plan::decode(&bytes).expect("the proof-bearing package opens");
    assert_eq!(decoded, compiled, "the certificate survives categorically");
    assert_eq!(declared, hash, "the proof-bearing identity travels intact");

    let root = decoded.owners.first().expect("the Loop owner");
    let [rastok::plan::AspectSpec::Loop(spec)] = root.aspects.as_slice() else {
        panic!("the root carries the Loop aspect");
    };
    assert!(spec.fuel.is_none());
    assert_eq!(
        spec.proof.as_ref().expect("the proof travels").decreases,
        [1]
    );
}

#[test]
fn a_partial_foreign_or_false_loop_spec_never_enters_a_v15_plan() {
    let (_, _, bytes) = packaged_loop();
    let mutate_aspect = |body: &mut std::collections::BTreeMap<u64, Cbor>,
                         edit: &dyn Fn(&mut Vec<Cbor>)| {
        let Cbor::Array(owners) = body.get_mut(&9).expect("the owner table") else {
            panic!("owners are an array");
        };
        let Cbor::Map(root) = &mut owners[0] else {
            panic!("the root owner is a map");
        };
        let Cbor::Array(aspects) = root.get_mut(&7).expect("the aspect table") else {
            panic!("aspects are an array");
        };
        let Cbor::Array(aspect) = &mut aspects[0] else {
            panic!("the Loop aspect is tagged");
        };
        edit(aspect);
    };

    let partial = reframe_body(&bytes, |body| {
        mutate_aspect(body, &|aspect| {
            let Cbor::Map(spec) = &mut aspect[1] else {
                panic!("the Loop specification is a map");
            };
            spec.remove(&1);
        });
    });
    assert!(matches!(
        plan::decode(&partial).unwrap_err(),
        DecodeError::Invalid(rastok::plan::PlanValidationError::InvalidGraphContract {
            detail: "Loop requires positive fuel or a verified measure",
            ..
        })
    ));

    let foreign = reframe_body(&bytes, |body| {
        mutate_aspect(body, &|aspect| aspect[0] = Cbor::Uint(99));
    });
    assert_eq!(
        plan::decode(&foreign).unwrap_err(),
        DecodeError::Malformed(String::from("aspect specification"))
    );

    let (_, _, measured) = packaged_measure_loop();
    let lying = reframe_body(&measured, |body| {
        mutate_aspect(body, &|aspect| {
            let Cbor::Map(spec) = &mut aspect[1] else {
                panic!("the Loop specification is a map");
            };
            let Cbor::Map(proof) = spec.get_mut(&3).expect("the proof") else {
                panic!("the proof is a map");
            };
            proof.insert(3, Cbor::Array(vec![Cbor::Uint(2)]));
        });
    });
    assert!(matches!(
        plan::decode(&lying).unwrap_err(),
        DecodeError::Invalid(rastok::plan::PlanValidationError::InvalidLoopControl {
            detail: "the Loop proof certificate disagrees with the shipped handlers",
            ..
        })
    ));
}

#[test]
fn v15_round_trips_tagged_structural_and_narrowing_expressions() {
    let sources = [
        r#"
type Failure = union { Broken { message: text } }
graph g : App {
  input in: variant
  operator o {
    output failure: Failure
    on in { failure = Broken { message = "no" } }
  }
  . -> o
}
"#,
        r#"
type Customer = struct { id: int }
graph g : App {
  input in: variant
  output out: variant
  operator o { on in where data is Customer { out = data.id } }
  . -> o -> .
}
"#,
        r#"
graph g : App {
  input in: int
  output out: int | text
  operator o {
    input in: int
    output out: int | text
    on in { out = 5 }
  }
  . -> o -> .
}
"#,
    ];
    for source in sources {
        let compilation = compile(
            &[source],
            &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .expect("a valid v15 program");
        let bytes = plan::encode(&compilation.plan, &compilation.hash);
        let (decoded, hash) = plan::decode(&bytes).expect("the v15 package opens");
        assert_eq!(decoded, compilation.plan);
        assert_eq!(hash, compilation.hash);
    }
}

#[test]
fn v15_round_trips_the_categorical_instance_contract() {
    let source = r#"
graph g : App {
  input in: text
  output out: text
  operator relay {
    instance = { key = data }
    lifetime = { kind = idle ; after = duration("30s") }
    input in: text
    output out: text
    var visits: int = 0
    on start { visits = visits + 1 }
    on in { out = . }
    on stop { visits = visits + 1 }
  }
  . -> relay -> .
}
"#;
    let compilation = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("the keyed program checks");
    let before = compilation
        .plan
        .plan()
        .node_instance(rastok::plan::Slot::new(0))
        .expect("the lowered node has an owner");
    assert_eq!(
        before.key.as_ref().expect("the node is keyed").ty,
        rastok::datatype::DataType::Text
    );
    assert!(matches!(
        before.lifetime,
        rastok::plan::InstanceLifetime::Idle(30_000_000_000)
    ));
    assert!(compilation.plan.plan().handlers.iter().any(|handler| {
        handler.activation
            == rastok::node::HandlerActivation::Lifecycle(rastok::node::Lifecycle::Start)
    }));

    let bytes = plan::encode(&compilation.plan, &compilation.hash);
    let (decoded, hash) = plan::decode(&bytes).expect("the v15 package opens");
    assert_eq!(decoded, compilation.plan);
    assert_eq!(hash, compilation.hash);
}

#[test]
fn a_nested_keyed_graph_crosses_the_whole_public_compiler_path() {
    let source = r#"
graph g : App {
  input in: text
  output out: text
  val prefix = "hello "

  graph sessions {
    input in: text
    output out: text
    instance = { key = data }

    operator greet {
      input in: text
      output out: text
      on in { out = prefix + data }
    }

    . -> greet -> .
  }

  . -> sessions -> .
}
"#;
    let compilation = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("the nested keyed graph checks");
    let bytes = plan::encode(&compilation.plan, &compilation.hash);
    let (decoded, hash) = plan::decode(&bytes).expect("the nested package opens");
    assert_eq!(decoded, compilation.plan);
    assert_eq!(hash, compilation.hash);

    let plan = decoded.plan();
    assert!(
        plan.owners
            .iter()
            .any(|owner| owner.path.to_string() == "sessions")
    );
    assert!(
        plan.nodes
            .iter()
            .any(|node| node.path.to_string() == "sessions.greet")
    );
    assert_eq!(plan.edges.len(), 2, "the two composite seams flatten away");
    assert!(
        plan.edges
            .iter()
            .all(|edge| plan.owners[edge.scope.index()].path.is_root())
    );
}

#[test]
fn v15_round_trips_a_bounded_join_activation_without_a_second_key_authority() {
    let source = r#"
type JoinExpiry = union { JoinExpired {} }
graph g : App {
  input left: int
  input right: int
  output out: int
  output expired: JoinExpiry
  operator pair {
    instance = { key = data }
    input left: int
    input right: int
    output out: int
    output expired: JoinExpiry
    on (left, right) where left.data == right.data within duration("2s") last 3 {
      out = left.data
    }
  }
  .left -> pair.left
  .right -> pair.right
  pair.out -> .out
  pair.expired -> .expired
}
"#;
    let compilation = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("the public bounded join checks");
    let bytes = plan::encode(&compilation.plan, &compilation.hash);
    let (decoded, hash) = plan::decode(&bytes).expect("the bounded join package opens");
    assert_eq!(decoded, compilation.plan);
    assert_eq!(hash, compilation.hash);
    assert!(matches!(
        &decoded.plan().handlers[0].activation,
        rastok::node::HandlerActivation::Join {
            ports,
            within_ns: Some(2_000_000_000),
            last: Some(3),
        } if ports.iter().map(|port| port.name()).collect::<Vec<_>>() == ["left", "right"]
    ));
}

#[test]
fn the_example_package_matches_its_golden() {
    // M1.3 final review removes the lowering-only Operator runtime-fault
    // `err` row. The source hash is intentionally unchanged: that invented
    // row never belonged to canonical HIR identity. M1.4 R2 then moves only
    // the package to categorical v9: the root/Node owner table and scoped
    // edges are derived plan structure, not a semantic change to this graph.
    // M1.4 R5 deliberately moves both hash and package because the exact
    // ProcessExit union gains the empty OutcomeUnknown settlement case.
    // M1.5 C1 deliberately moves both again: stable `rastok/process@1` and
    // its outbound role replace the source symbol in identity, while v10
    // carries those categorical facts. The causal trace remains unchanged.
    // M1.6 H1 adds the explicit Call exchange tag and categorically moves the
    // container to v11. Existing Call causal records remain byte-identical.
    // M1.6 H4.1 adds `media_type` to the type/expression registry and moves
    // every container categorically to v12. M1.7 L2 moves containers to v13
    // so Graph aspect contracts and their ordinary handlers can travel. L6
    // moves containers to v14 so optional checked Loop proofs are explicit.
    // M1.8 A2 moves containers to v15 for sparse schema-policy tables. This
    // example has none, so only the categorical version/checksum change.
    let (_, _, bytes) = packaged_example();
    let rendered: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/echo.rkp");
    if std::env::var("RASTOK_BLESS").is_ok() {
        std::fs::write(&path, format!("{rendered}\n")).expect("a writable golden dir");
        return;
    }
    let golden = std::fs::read_to_string(&path)
        .expect("no package golden; run once with RASTOK_BLESS=1 to bless");
    assert_eq!(golden.trim(), rendered, "the packaged bytes are pinned");
}

/// Re-frames a package after editing its envelope — the mutation helper
/// the refusal tests share.
fn reframe(bytes: &[u8], edit: impl FnOnce(&mut std::collections::BTreeMap<u64, Cbor>)) -> Vec<u8> {
    let rest = bytes.strip_prefix(b"RKPB").expect("the magic");
    let (envelope, _) = decode(rest).expect("the envelope decodes");
    let Cbor::Map(mut pairs) = envelope else {
        panic!("the envelope is a map");
    };
    edit(&mut pairs);
    let mut out = b"RKPB".to_vec();
    out.extend_from_slice(&encode(&Cbor::Map(pairs)));
    out
}

/// Re-seals a package after deliberately forging its categorical body.
fn reframe_body(
    bytes: &[u8],
    edit: impl FnOnce(&mut std::collections::BTreeMap<u64, Cbor>),
) -> Vec<u8> {
    let rest = bytes.strip_prefix(b"RKPB").expect("the magic");
    let (Cbor::Map(mut envelope), _) = decode(rest).expect("the envelope decodes") else {
        panic!("the envelope is a map");
    };
    let Cbor::Bytes(old_body) = envelope.get(&3).expect("the body") else {
        panic!("the body is bytes");
    };
    let (Cbor::Map(mut body), _) = decode(old_body).expect("the body decodes") else {
        panic!("the body is a map");
    };
    edit(&mut body);
    let body = encode(&Cbor::Map(body));
    let Cbor::Bytes(hash) = envelope.get(&2).expect("the semantic hash") else {
        panic!("the semantic hash is bytes");
    };
    let mut checksum = [0_u8; 32];
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"rastok.plan-package\0");
    hasher.update(&plan::FORMAT_VERSION.to_le_bytes());
    hasher.update(hash);
    hasher.update(&body);
    hasher.finalize_xof().fill(&mut checksum);
    envelope.insert(3, Cbor::Bytes(body));
    envelope.insert(4, Cbor::Bytes(checksum.to_vec()));
    let mut out = b"RKPB".to_vec();
    out.extend_from_slice(&encode(&Cbor::Map(envelope)));
    out
}

fn connector_spec_mut(body: &mut std::collections::BTreeMap<u64, Cbor>) -> &mut Cbor {
    let Cbor::Array(nodes) = body.get_mut(&1).expect("the node table") else {
        panic!("nodes are an array");
    };
    let Cbor::Map(shell) = &mut nodes[1] else {
        panic!("shell is a node map");
    };
    let Cbor::Array(kind) = shell.get_mut(&2).expect("the node kind") else {
        panic!("the connector kind is an array");
    };
    &mut kind[1]
}

#[test]
fn connector_identity_roles_and_exchange_are_categorical_plan_facts() {
    let (_, _, bytes) = packaged_example();

    let forged_id = reframe_body(&bytes, |body| {
        let Cbor::Map(spec) = connector_spec_mut(body) else {
            panic!("the connector spec is a map");
        };
        spec.insert(1, Cbor::Text(String::from("Process")));
    });
    assert_eq!(
        plan::decode(&forged_id).unwrap_err(),
        DecodeError::Malformed(String::from(
            "invalid connector id: expected exactly `<namespace>/<name>@<major>`"
        ))
    );

    let forged_role = reframe_body(&bytes, |body| {
        let Cbor::Map(spec) = connector_spec_mut(body) else {
            panic!("the connector spec is a map");
        };
        spec.insert(6, Cbor::Uint(1));
    });
    assert_eq!(
        plan::decode(&forged_role).unwrap_err(),
        DecodeError::Invalid(rastok::plan::PlanValidationError::ContractRoleMismatch {
            node: rastok::node::Path::new(["shell"]).unwrap(),
        })
    );

    let forged_exchange = reframe_body(&bytes, |body| {
        let Cbor::Map(spec) = connector_spec_mut(body) else {
            panic!("the connector spec is a map");
        };
        spec.insert(7, Cbor::Uint(3));
    });
    assert_eq!(
        plan::decode(&forged_exchange).unwrap_err(),
        DecodeError::Invalid(
            rastok::plan::PlanValidationError::ContractExchangeMismatch {
                node: rastok::node::Path::new(["shell"]).unwrap(),
            }
        )
    );
}

#[test]
fn every_refusal_is_typed() {
    let (_, _, bytes) = packaged_example();

    // Foreign bytes.
    let mut foreign = bytes.clone();
    foreign[0] = b'X';
    assert_eq!(plan::decode(&foreign).unwrap_err(), DecodeError::BadMagic);

    // A future format version — refused by name.
    let future = u64::from(plan::FORMAT_VERSION) + 1;
    let bumped = reframe(&bytes, |pairs| {
        pairs.insert(1, Cbor::Uint(future));
    });
    assert_eq!(
        plan::decode(&bumped).unwrap_err(),
        DecodeError::UnsupportedVersion(future)
    );

    // Every earlier categorical body is refused by number, never migrated
    // silently. In particular, v8 was the unreleased named-partition draft;
    // v9 replaced it rather than assigning two meanings to one version; v10
    // then replaced source Connector names with stable ids, v11 adds the
    // mandatory exchange lifecycle, v12 adds the media-type registry, and
    // v13 adds categorical Graph-aspect transport, v14 adds checked Loop
    // proof transport, and v15 adds schema-policy side-tables.
    for draft in [2u64, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14] {
        let retired = reframe(&bytes, |pairs| {
            pairs.insert(1, Cbor::Uint(draft));
        });
        assert_eq!(
            plan::decode(&retired).unwrap_err(),
            DecodeError::UnsupportedVersion(draft)
        );
    }

    // A version beyond u32 is reported AS READ — never saturated
    // into a number that was not in the bytes.
    let huge = reframe(&bytes, |pairs| {
        pairs.insert(1, Cbor::Uint(u64::MAX));
    });
    assert_eq!(
        plan::decode(&huge).unwrap_err(),
        DecodeError::UnsupportedVersion(u64::MAX)
    );

    // A corrupted DECLARED HASH — the checksum covers the whole
    // association, not the body alone (the v1 hole: this byte flip
    // was accepted).
    let lying = reframe(&bytes, |pairs| {
        let Some(Cbor::Bytes(hash)) = pairs.get_mut(&2) else {
            panic!("the hash is bytes");
        };
        hash[0] ^= 0x01;
    });
    assert_eq!(
        plan::decode(&lying).unwrap_err(),
        DecodeError::BrokenChecksum
    );

    // One corrupted body byte — the seal catches it before parsing.
    let corrupted = reframe(&bytes, |pairs| {
        let Some(Cbor::Bytes(body)) = pairs.get_mut(&3) else {
            panic!("the body is bytes");
        };
        let middle = body.len() / 2;
        body[middle] ^= 0x01;
    });
    assert_eq!(
        plan::decode(&corrupted).unwrap_err(),
        DecodeError::BrokenChecksum
    );

    // A truncated package — diagnosed as exactly that: the reader's
    // own category says the bytes END EARLY, not that they are
    // foreign.
    let truncated = &bytes[..bytes.len() / 2];
    assert_eq!(
        plan::decode(truncated).unwrap_err(),
        DecodeError::Read(rastok::canonical_cbor::DecodeError::UnexpectedEof)
    );

    // A syntactically foreign head (an indefinite-length map, 0xbf)
    // is a DIFFERENT category from truncation.
    let mut foreign_head = bytes.clone();
    foreign_head[4] = 0xbf;
    assert_eq!(
        plan::decode(&foreign_head).unwrap_err(),
        DecodeError::Read(rastok::canonical_cbor::DecodeError::UnsupportedHead(0xbf))
    );
}

#[test]
fn the_boundary_value_matrix_executes_identically_across_the_trip() {
    // The refuting counter-example first: v1 decoded the long literal
    // as a uint, and the sum stopped evaluating. Then the rest of the
    // integer boundary.
    for literal in [
        "2147483648 + -2147483648",
        "2147483647",
        "2147483648",
        "-2147483648",
        "-2147483649",
        "9223372036854775807",
        "-9223372036854775807",
        "0 - 9223372036854775807",
    ] {
        let source = format!(
            "graph probe : App {{\n  input in: text\n  output out: variant\n  operator o {{\n    on in {{ out = {literal} }}\n  }}\n\n  . -> o -> .\n}}\n"
        );
        let compilation = compile(
            &[source.as_str()],
            &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .expect("a valid plan");
        let bytes = plan::encode(&compilation.plan, &compilation.hash);
        let (decoded, _) = plan::decode(&bytes).expect("the package opens");
        assert_eq!(
            decoded, compilation.plan,
            "structural equality for `{literal}`"
        );
        let compiled_run = run_boundary_probe(&compilation.plan);
        let decoded_run = run_boundary_probe(&decoded);
        assert_eq!(
            compiled_run, decoded_run,
            "`{literal}` must execute identically across the trip"
        );
        assert_eq!(
            compiled_run.1, 0,
            "`{literal}` is a valid program; the run has no errors"
        );
    }
}

/// One injection through a boundary-only plan; returns the rendered
/// outputs and the error count — the execution fingerprint the matrix
/// compares.
fn run_boundary_probe(plan: &rastok::plan::ValidatedExecutionPlan) -> (Vec<String>, usize) {
    use rastok::boundary::{
        Draft, InboundEnvelopeProvenance, InboundSubmission, SubmissionSequence,
    };
    use rastok::envelope::{Data, Namespace, Value};
    use rastok::journal::MemJournal;
    use rastok::plan::GraphVersion;
    use rastok::runtime::vm::Vm;
    let mut journal = MemJournal::new(GraphVersion(1));
    let mut vm = Vm::boot(plan, GraphVersion(1));
    let submission = InboundSubmission {
        provenance: InboundEnvelopeProvenance::single(rastok::node::Path::new(["probe"]).unwrap()),
        port: rastok::node::Port::new("in").unwrap(),
        draft: Draft {
            data: Data::materialized(Value::Text(String::from("go"))),
            ext: Namespace::new(),
        },
        sequence: SubmissionSequence::Next,
    };
    vm.inject(&mut journal, submission)
        .expect("the injection lands");
    vm.drain(&mut journal).expect("the drain completes");
    let outputs = vm
        .outputs()
        .iter()
        .map(|output| format!("{}: {:?}", output.port, output.envelope.data()))
        .collect();
    (outputs, vm.errors().len())
}

#[test]
fn an_out_of_bounds_slot_cannot_become_a_validated_plan() {
    let (plan, _, _) = packaged_example();
    // A well-shaped plan naming impossible structure: the machine
    // would index past the node table (the review's Slot(999)). The
    // validity claim does not travel through into_inner — the raw
    // plan must re-earn the type, and cannot.
    let mut raw = plan.into_inner();
    raw.edges[0].to = Some(rastok::plan::EndpointRef {
        node: Some(rastok::plan::Slot::new(999)),
        port: rastok::node::Port::new("in").unwrap(),
    });
    let refused = rastok::plan::ValidatedExecutionPlan::try_from(raw);
    assert!(refused.is_err(), "Slot(999) never earns the validated type");
    // And since `plan::encode` only accepts the validated type, the
    // property holds by construction: what encode accepts, decode
    // reads back — there is no way to package the invalid plan.
}

#[test]
fn a_non_canonical_envelope_refuses() {
    let (_, _, bytes) = packaged_example();

    // Reordered header keys: same fields, same checksum coverage,
    // different bytes — not the canonical encoding. Raw splice,
    // because the canonical encoder itself refuses to write an
    // unsorted map: entry (1, uint 15) is bytes 5..7, entry
    // (2, bytes32) is the 35 bytes after it; swap them.
    assert_eq!(
        &bytes[4..7],
        &[0xa4, 0x01, 0x0f],
        "the envelope head is stable"
    );
    assert_eq!(&bytes[7..10], &[0x02, 0x58, 0x20], "the hash entry follows");
    let mut reordered = bytes.clone();
    let version_entry: Vec<u8> = bytes[5..7].to_vec();
    let hash_entry: Vec<u8> = bytes[7..42].to_vec();
    reordered.splice(5..42, hash_entry.into_iter().chain(version_entry));
    // The reader itself is strict now: unsorted keys refuse at the
    // parse, in the reader's own non-canonical category.
    assert!(matches!(
        plan::decode(&reordered).unwrap_err(),
        DecodeError::Read(rastok::canonical_cbor::DecodeError::NonCanonical(_))
    ));

    // A non-minimal integer in the header: uint 15 written on two
    // bytes (0x18 0x0f) instead of one. The checksum still holds —
    // the VALUE is unchanged — so only the canonicity check refuses.
    assert_eq!(
        &bytes[4..7],
        &[0xa4, 0x01, 0x0f],
        "the envelope head is stable"
    );
    let mut widened = bytes.clone();
    widened.splice(6..7, [0x18, 0x0f]);
    assert!(matches!(
        plan::decode(&widened).unwrap_err(),
        DecodeError::Read(rastok::canonical_cbor::DecodeError::NonCanonical(_))
    ));
}

#[test]
fn overflowing_declared_lengths_refuse_without_panic() {
    // A byte string announcing u64::MAX bytes: 0x5b + 8 length
    // bytes. The decoder must refuse arithmetically — LengthOverflow
    // or an early end — never a panicking add.
    let mut byte_bomb = vec![0x5b];
    byte_bomb.extend_from_slice(&u64::MAX.to_be_bytes());
    assert!(matches!(
        decode(&byte_bomb).unwrap_err(),
        rastok::canonical_cbor::DecodeError::LengthOverflow
    ));

    // The same bomb as a text string (0x7b).
    let mut text_bomb = vec![0x7b];
    text_bomb.extend_from_slice(&u64::MAX.to_be_bytes());
    assert!(matches!(
        decode(&text_bomb).unwrap_err(),
        rastok::canonical_cbor::DecodeError::LengthOverflow
    ));

    // A large-but-addressable declared length with no data behind
    // it is truncation, not overflow — the categories stay honest.
    let mut hungry = vec![0x5a];
    hungry.extend_from_slice(&0x1000_0000u32.to_be_bytes());
    assert!(matches!(
        decode(&hungry).unwrap_err(),
        rastok::canonical_cbor::DecodeError::UnexpectedEof
    ));
}

#[test]
fn nesting_beyond_the_budget_refuses_before_the_stack_does() {
    // 512 nested single-element arrays: a legal-looking prefix that
    // would recurse past any reasonable stack — refused by budget,
    // never by overflow.
    let mut deep = vec![0x81u8; 512];
    deep.push(0x00);
    assert_eq!(
        decode(&deep).unwrap_err(),
        rastok::canonical_cbor::DecodeError::NestingLimitExceeded
    );
}

fn nested_additions(count: usize) -> String {
    let additions = "+ 1 ".repeat(count);
    format!(
        "graph deep : App {{\n  input in: text\n  operator o {{\n    var seen : int = 0\n\n    on in {{\n      \
         seen = 1 {additions}\n      out = data\n    }}\n  }}\n\n  . -> o -> .\n}}\n"
    )
}

#[test]
fn a_program_beyond_the_nesting_budget_refuses_by_name() {
    // The reviewer's counter-example: a source the checker accepts
    // whose plan outgrows the reader's budget. It must die as a NAMED
    // user diagnostic — the budget is a source-level limit — never as
    // an internal-plan error, and never as an encoded package that
    // no decoder reads.
    let source = nested_additions(200);
    let bag = compile(
        &[source.as_str()],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect_err("beyond the budget");
    assert!(
        bag.diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code().as_str() == "E-PLAN-DEPTH"),
        "{bag:?}"
    );
    assert!(
        bag.diagnostics()
            .iter()
            .all(|diagnostic| diagnostic.code().as_str() != "E-INTERNAL-PLAN"),
        "{bag:?}"
    );
}

#[test]
fn a_nan_plan_equals_itself_across_the_trip() {
    // `0.0 / 0.0` is a legal constant NaN. Structural plan equality
    // is canonical-bits equality, so the round trip preserves the
    // claim the header test makes — "structural equality across the
    // trip" — instead of a plan that stops equalling itself.
    let source = "graph g : App {\n  input in: text\n  operator o {\n    var seen : double = 0.0 / 0.0\n\n    on in {\n      out = data\n    }\n  }\n\n  . -> o -> .\n}\n";
    let compilation = compile(
        &[source],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    let bytes = plan::encode(&compilation.plan, &compilation.hash);
    let (decoded, _) = plan::decode(&bytes).expect("the package opens");
    assert_eq!(decoded, compilation.plan, "NaN included");
}

#[test]
fn the_zero_signs_are_two_plans() {
    // `+0.0` and `-0.0` have different canonical bytes; two plans
    // differing only there must compare UNEQUAL structurally — the
    // packages differ, so equality may not say otherwise.
    let with_zero = |zero: &str| {
        let source = format!(
            "graph g : App {{\n  input in: text\n  operator o {{\n    var seen : double = {zero}\n\n    on in {{\n      out = data\n    }}\n  }}\n\n  . -> o -> .\n}}\n"
        );
        compile(
            &[source.as_str()],
            &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .expect("a valid plan")
    };
    let positive = with_zero("0.0");
    let negative = with_zero("-0.0");
    assert_ne!(
        positive.plan, negative.plan,
        "different canonical bytes, different plans"
    );
    assert_ne!(
        plan::encode(&positive.plan, &positive.hash),
        plan::encode(&negative.plan, &negative.hash),
    );
}

#[test]
fn a_hostile_depth_source_refuses_and_the_process_survives() {
    // Re-exec pattern: the deep compiles run in a CHILD process, so a
    // regression (a recursive pass before the gate, a recursive drop
    // of the refused tree) kills the child and fails the assertion —
    // never the harness. The reviewer's counter-example was exactly
    // this: E-PLAN-DEPTH at 200 additions, a dead process at 5 000.
    if std::env::var("RASTOK_DEPTH_PROBE").is_ok() {
        // A left-fold chain: the parser builds it with NO recursion,
        // so only the post-parse gate can catch it — and the refused
        // 30 000-deep tree must be dismantled iteratively.
        let chain = nested_additions(30_000);
        let bag = compile(
            &[chain.as_str()],
            &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .expect_err("beyond the budget");
        assert!(
            bag.diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code().as_str() == "E-PLAN-DEPTH"),
            "{bag:?}"
        );
        // A parenthesis tower: pure parser recursion, zero AST depth
        // — the parser's own guard must refuse it.
        let tower = format!(
            "graph g : App {{\n  input in: text\n  operator o {{\n    var seen : int = 0\n\n    on in {{\n      \
             seen = {}1{}\n      out = data\n    }}\n  }}\n\n  . -> o -> .\n}}\n",
            "(".repeat(5_000),
            ")".repeat(5_000),
        );
        let bag = compile(
            &[tower.as_str()],
            &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        )
        .expect_err("beyond the guard");
        assert!(
            bag.diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code().as_str() == "E-PLAN-DEPTH"),
            "{bag:?}"
        );
        println!("DEPTH_PROBE_SURVIVED");
        return;
    }
    let exe = std::env::current_exe().expect("the test binary");
    let output = std::process::Command::new(exe)
        .args([
            "--exact",
            "a_hostile_depth_source_refuses_and_the_process_survives",
            "--nocapture",
        ])
        .env("RASTOK_DEPTH_PROBE", "1")
        .output()
        .expect("the child spawns");
    assert!(
        output.status.success(),
        "the child died instead of refusing by name:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("DEPTH_PROBE_SURVIVED"));
}

#[test]
fn a_deep_but_budgeted_program_rides_the_round_trip() {
    let source = nested_additions(100);
    let compilation = compile(
        &[source.as_str()],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("within the budget");
    let bytes = plan::encode(&compilation.plan, &compilation.hash);
    let (decoded, declared) = plan::decode(&bytes).expect("what encode accepts, decode reads");
    assert_eq!(decoded, compilation.plan);
    assert_eq!(declared, compilation.hash);
}

#[test]
fn the_depth_boundaries_are_exact() {
    // One unit per SYNTACTIC level, so the visible budget is the
    // real one. A parenthesis tower spends parser recursion without
    // creating AST depth: 127 pairs fit under the 128-entry guard
    // (the expression itself is the first entry), the 128th refuses
    // at CHECK with the structured `depth` payload.
    let parens = |n: usize| {
        format!(
            "graph g : App {{\n  input in: text\n  operator o {{\n    var seen : int = 0\n\n    on in {{\n      \
             seen = {}1{}\n      out = data\n    }}\n  }}\n\n  . -> o -> .\n}}\n",
            "(".repeat(n),
            ")".repeat(n),
        )
    };
    compile(
        &[parens(127).as_str()],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("127 pairs fit the parser guard");
    let bag = compile(
        &[parens(128).as_str()],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect_err("the 128th refuses");
    let depth = bag
        .diagnostics()
        .iter()
        .find(|diagnostic| diagnostic.code().as_str() == "E-PLAN-DEPTH")
        .expect("refused by name");
    assert_eq!(depth.phase(), rastok::diagnostics::Phase::Check);
    assert!(
        depth
            .fields()
            .iter()
            .any(|(name, value)| *name == "depth" && value == "129"),
        "the structured payload, not the sentence: {:?}",
        depth.fields()
    );

    // An addition chain builds real serialized depth: 119 additions
    // reach level 127 and ride the round trip; the 120th reaches the
    // reader's exact budget and refuses at PLAN, same payload.
    let deep_ok = nested_additions(119);
    let compilation = compile(
        &[deep_ok.as_str()],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect("119 additions fit");
    let bytes = plan::encode(&compilation.plan, &compilation.hash);
    plan::decode(&bytes).expect("the deepest accepted program reads back");
    let bag = compile(
        &[nested_additions(120).as_str()],
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
    )
    .expect_err("the 120th refuses");
    let depth = bag
        .diagnostics()
        .iter()
        .find(|diagnostic| diagnostic.code().as_str() == "E-PLAN-DEPTH")
        .expect("refused by name");
    assert_eq!(depth.phase(), rastok::diagnostics::Phase::Plan);
    assert!(
        depth
            .fields()
            .iter()
            .any(|(name, value)| *name == "depth" && value == "128"),
        "{:?}",
        depth.fields()
    );
}
