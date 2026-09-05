//! Source-to-field transport evidence; a field is not an executable package.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use rastok::{
    compiler,
    contract::ContractCatalog,
    datatype::DataType,
    diagnostics::{Bag, SourceId},
    expr, plan,
};
use std::collections::BTreeMap;

fn checked(source: &str) -> compiler::Hir {
    let registry = ContractCatalog::try_m0().expect("the standard contracts are consistent");
    let source_id = SourceId::new(0);
    let mut bag = Bag::new();
    let tree = compiler::parser::parse(source_id, source, &registry, &mut bag);
    let unit = compiler::Unit {
        files: vec![(source_id, tree)],
    };
    let hir = compiler::check::check(&unit, &registry, &mut bag, None)
        .expect("the source contains one App");
    assert!(!bag.has_errors(), "{bag:?}");
    hir
}

#[test]
fn source_decisions_and_broadcasts_cross_the_field_reader_without_a_v15_escape() {
    let source = r#"
graph app : App {
  input in: bool
  output out: text
  operator choose {
    input in: bool
    output sms: text
    output email: text
    on in match data {
      false => (sms, email) = "no"
      true => match {
        data => (sms, email) = "yes"
        _ => nothing
      }
    }
  }
  in -> choose
  choose.sms -> out
  choose.email -> nothing
}
"#;
    let hir = checked(source);
    let original = &hir.app.nodes[0].handlers[0].dispatch;
    let signature = expr::Signature::new().with("data", DataType::Bool);
    let bytes = plan::fields::encode_dispatch(original, &signature).unwrap();
    assert_eq!(plan::decode(&bytes), Err(plan::DecodeError::BadMagic));
    let decoded = plan::fields::decode_dispatch(&bytes, &signature).unwrap();
    assert_eq!(&decoded, original);
    for (value, expected) in [(false, "no"), (true, "yes")] {
        let bindings = BTreeMap::from([("data".into(), expr::EvalValue::Bool(value))]);
        let selected = rastok::runtime::select_dispatch(&decoded, &bindings).unwrap();
        let [
            plan::DispatchStep::MultiEmit {
                ports,
                values: plan::MultiEmitValues::Broadcast(plan::Emission::Data(tree)),
            },
        ] = selected.statements
        else {
            panic!("broadcast leaf")
        };
        assert_eq!(
            ports.iter().map(|port| port.name()).collect::<Vec<_>>(),
            ["sms", "email"]
        );
        assert_eq!(
            expr::eval(tree, &bindings).unwrap(),
            expr::EvalValue::Text(expected.into())
        );
    }
    let error = compiler::lower::lower(&hir).unwrap_err();
    assert!(error.detail.contains("categorical v16"));
    assert_eq!(
        plan::FORMAT_VERSION,
        15,
        "the field must not publish a partial package"
    );
}

#[test]
fn where_desugaring_and_closed_arm_permutations_keep_their_field_bytes() {
    let graph = |handler: &str| {
        format!(
            r#"
graph app : App {{
  input in: bool
  output out: bool
  operator filter {{ input in: bool; output out: bool; {handler} }}
  in -> filter -> out
}}
"#
        )
    };
    let signature = expr::Signature::new().with("data", DataType::Bool);
    for (left, right) in [
        (
            "on in where data { out = data }",
            "on in match { data => out = data; _ => nothing }",
        ),
        (
            "on in match data { false => nothing; true => out = data }",
            "on in match data { true => out = data; false => nothing }",
        ),
    ] {
        let first = checked(&graph(left));
        let second = checked(&graph(right));
        let bytes = |hir: &compiler::Hir| {
            plan::fields::encode_dispatch(&hir.app.nodes[0].handlers[0].dispatch, &signature)
                .unwrap()
        };
        assert_eq!(bytes(&first), bytes(&second));
        assert_eq!(
            compiler::lower::semantic_hash(&first),
            compiler::lower::semantic_hash(&second)
        );
    }
}

#[test]
fn source_route_cases_keep_one_physical_edge_per_destination() {
    let hir = checked(
        r#"
graph app : App {
  input in: bool
  output out: bool
  operator relay { input in: bool; output out: bool; on in { out = data } }
  operator sink { input in: bool; on in { nothing } }
  in -> relay
  relay.out -> match {
    true -> { out; sink }
    false -> sink
  }
}
"#,
    );
    let mut rules = hir.app.routes.clone();
    rules.sort_by_key(|rule| {
        plan::fields::encode_routing(std::slice::from_ref(rule), |_| Some(DataType::Bool)).unwrap()
    });
    let bytes = plan::fields::encode_routing(&rules, |_| Some(DataType::Bool)).unwrap();
    let decoded = plan::fields::decode_routing(&bytes, |_| Some(DataType::Bool)).unwrap();
    assert_eq!(decoded, rules);
    let routes: Vec<_> = decoded
        .iter()
        .filter(|rule| rule.source.port.name() == "out")
        .collect();
    assert_eq!(routes.len(), 1);
    let plan::RouteSelection::Cases(arms) = &routes[0].selection else {
        panic!("case routing")
    };
    assert_eq!(arms.len(), 2);
    let is_sink = |endpoint: &plan::Endpoint| matches!(&endpoint.owner, plan::EndpointOwner::Node(path) if path.to_string() == "sink");
    assert!(arms.iter().all(|arm| {
        arm.destinations.iter().any(|destination| {
            matches!(destination, plan::RouteDestination::Port(endpoint) if is_sink(endpoint))
        })
    }));
    assert_eq!(
        hir.app
            .edges
            .iter()
            .filter(|edge| {
                matches!(&edge.to, compiler::hir::EdgeTarget::Port(endpoint) if is_sink(endpoint))
            })
            .count(),
        1,
    );
    assert!(
        compiler::lower::lower(&hir)
            .unwrap_err()
            .detail
            .contains("categorical v16")
    );
}
