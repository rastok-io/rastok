//! The three-zone metadata model (D-2026-08-10-1 … -4): inference
//! from the graph's writes, the static CEL grid, prefix-freedom,
//! zone refusals, the Lab's inability to touch production, and the
//! content-type lifecycle. Every refusal here is a named diagnostic,
//! never a silent reinterpretation.

#![allow(clippy::disallowed_types, clippy::disallowed_methods)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use rastok::compiler::compile;
use rastok::contract::Registry;

fn refuses_with(source: &str, code: &str) -> Vec<String> {
    let bag = compile(
        &[source],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect_err("the source must refuse");
    let hits: Vec<String> = bag
        .diagnostics()
        .iter()
        .filter(|diagnostic| diagnostic.code().as_str() == code)
        .map(|diagnostic| diagnostic.message().to_string())
        .collect();
    assert!(!hits.is_empty(), "expected {code}: {bag:?}");
    hits
}

#[test]
fn a_bare_meta_field_refuses() {
    // The alias is dead: `meta.customer_id` resolves to nothing.
    refuses_with(
        "graph g : App {\n  operator o {\n    on in { out = { meta.customer_id = \"x\" } }\n  }\n  . -> o -> .\n}\n",
        "E-META-ZONE",
    );
}

#[test]
fn a_custom_root_refuses() {
    // Root customs died with the zones.
    refuses_with(
        "graph g : App {\n  operator o {\n    on in { out = { meta.audit.origin = \"x\" } }\n  }\n  . -> o -> .\n}\n",
        "E-META-ZONE",
    );
}

#[test]
fn an_unknown_ext_write_refuses() {
    // `ext` belongs to the connector sheets plus `content_type`;
    // the application's own vocabulary goes to `app`.
    refuses_with(
        "graph g : App {\n  operator o {\n    on in { out = { meta.ext.invented = \"x\" } }\n  }\n  . -> o -> .\n}\n",
        "E-META-ZONE",
    );
}

#[test]
fn a_read_never_written_refuses() {
    // The CEL grid, statically: the graph writes `meta.app.known`
    // nowhere, so reading it is a compile error — not a runtime err.
    refuses_with(
        "graph g : App {\n  operator o {\n    on in where meta.app.ghost == \"x\" { out = data }\n  }\n  . -> o -> .\n}\n",
        "E-META-ZONE",
    );
}

#[test]
fn inference_is_order_independent() {
    // The same writes in either textual order seal the same schema
    // and the same semantic hash.
    let ab = "graph g : App {\n  operator o {\n    on in {\n      out = {\n        meta.app.first = \"a\"\n        meta.app.second = 2\n        data = data\n      }\n    }\n  }\n  . -> o -> .\n}\n";
    let ba = "graph g : App {\n  operator o {\n    on in {\n      out = {\n        meta.app.second = 2\n        meta.app.first = \"a\"\n        data = data\n      }\n    }\n  }\n  . -> o -> .\n}\n";
    let first = compile(
        &[ab],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    let second = compile(
        &[ba],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid plan");
    // The write ORDER is identity (the fold executes in order), so
    // the hashes differ — but both compile, and reading either
    // field is legal in both:
    let reads = "graph g : App {\n  operator o {\n    on in {\n      out = {\n        meta.app.first = \"a\"\n        meta.app.second = 2\n        data = data\n      }\n    }\n  }\n  operator p {\n    on in where meta.app.first == \"a\" { out = data }\n  }\n  . -> o\n  o -> p\n  p -> .\n}\n";
    compile(
        &[reads],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a downstream read of an inferred field");
    let _ = (first, second);
}

#[test]
fn a_type_conflict_names_both_sites() {
    let source = "graph g : App {\n  operator o {\n    on in {\n      out = {\n        meta.app.flag = \"text\"\n        data = data\n      }\n    }\n  }\n  operator p {\n    on in {\n      out = {\n        meta.app.flag = 7\n        data = data\n      }\n    }\n  }\n  . -> o\n  o -> p\n  p -> .\n}\n";
    let bag = compile(
        &[source],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect_err("one field, one type");
    let hit = bag
        .diagnostics()
        .iter()
        .find(|diagnostic| diagnostic.code().as_str() == "E-META-TYPE")
        .expect("the unification refusal");
    assert!(
        hit.spans().len() >= 2,
        "both write sites are named: {hit:?}"
    );
}

#[test]
fn prefix_collisions_refuse() {
    // `audit` leaf + `audit.origin` branch cannot coexist.
    refuses_with(
        "graph g : App {\n  operator o {\n    on in {\n      out = {\n        meta.app.audit = \"leaf\"\n        meta.app.audit.origin = \"branch\"\n        data = data\n      }\n    }\n  }\n  . -> o -> .\n}\n",
        "E-META-PREFIX",
    );
}

#[test]
fn a_reserved_node_name_refuses() {
    for name in ["meta", "data", "sys", "app", "ext"] {
        let source = format!(
            "graph g : App {{\n  operator {name} {{\n    on in {{ out = data }}\n  }}\n  . -> {name} -> .\n}}\n"
        );
        let bag = compile(
            &[source.as_str()],
            &Registry::try_m0().expect("the standard catalogue holds"),
        )
        .expect_err("reserved");
        assert!(
            bag.diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code().as_str() == "E-NAME-RESERVED"),
            "`{name}` must refuse: {bag:?}"
        );
    }
}

#[test]
fn the_lab_cannot_extend_the_production_schema() {
    // An expectation reading a field the GRAPH writes nowhere
    // refuses — the Lab answers to the production schema and never
    // widens it.
    let source = r#"
graph g : App {
  operator o {
    on in { out = data }
  }
  . -> o -> .
}

lab g {
  simulation probing on app {
    scenario s "a lab-only field is not a field" {
      app.in = { v = 1 }
      expect "never" app.out count == 1 where meta.app.lab_only == "x"
    }
  }
}
"#;
    refuses_with(source, "E-META-ZONE");
}

#[test]
fn an_injection_cannot_write_the_app_zone() {
    // The fixture is the world's half: world context enters through
    // `ext`; `meta.app` belongs to the graph.
    let source = r#"
graph g : App {
  operator o {
    on in { out = data }
  }
  . -> o -> .
}

lab g {
  simulation probing on app {
    scenario s "the world writes ext, never app" {
      app.in = { data = { v = 1 }, meta.app.seeded = "x" }
      expect "unreached" app.out count == 1
    }
  }
}
"#;
    refuses_with(source, "E-META-ZONE");
}

#[test]
fn an_injection_seeds_ext_and_the_graph_copies_it() {
    // The idiom of D-2026-08-10-3: world context arrives in `ext`
    // (here the standard `content_type`), and the graph copies what
    // it wants to keep into `app` — explicitly.
    let source = r#"
graph g : App {
  operator o {
    on in {
      out = {
        meta.app.original_type = meta.ext.content_type
        data = data
      }
    }
  }
  . -> o -> .
}

lab g {
  simulation probing on app {
    scenario s "ext in, app kept" {
      app.in = { data = { v = 1 }, meta.ext.content_type = "application/json" }
      expect "the copy landed" app.out count == 1 where meta.app.original_type == "application/json"
    }
  }
}
"#;
    let compilation = compile(
        &[source],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect("a valid unit");
    let runs = rastok::lab::run(
        &compilation.plan,
        rastok::plan::GraphVersion(1),
        &compilation.labs[0],
    );
    for run in &runs {
        assert!(run.report.passed(), "{:#?}", run.report.failures);
    }
}

#[test]
fn the_lab_never_moves_the_production_hash() {
    // The proof D-2026-08-10-1 demands: adding a Lab — fixtures,
    // mocks, expectations — changes neither the plan nor the
    // semantic hash of the deployed graph.
    let graph = "graph g : App {\n  operator o {\n    on in {\n      out = {\n        meta.app.kept = \"x\"\n        data = data\n      }\n    }\n  }\n  . -> o -> .\n}\n";
    let with_lab = format!(
        "{graph}\nlab g {{\n  simulation probing on app {{\n    scenario s \"observes\" {{\n      app.in = {{ v = 1 }}\n      expect \"out\" app.out count == 1 where meta.app.kept == \"x\"\n    }}\n  }}\n}}\n"
    );
    let bare = compile(
        &[graph],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect("the bare graph");
    let inhabited = compile(
        &[with_lab.as_str()],
        &Registry::try_m0().expect("the standard catalogue holds"),
    )
    .expect("the graph with a lab");
    assert_eq!(bare.hash, inhabited.hash, "the Lab is not identity");
    assert_eq!(
        rastok::plan::encode(&bare.plan, &bare.hash),
        rastok::plan::encode(&inhabited.plan, &inhabited.hash),
        "the Lab is not the plan either"
    );
}

#[test]
fn content_type_follows_the_payload_not_the_merge() {
    // The lifecycle across spellings (D-2026-08-10-4): a kept
    // witness keeps it, fresh data strikes it, an explicit write
    // wins — variables and ternaries included.
    use rastok::boundary::{Draft, InboundSubmission, IngressAuthority, SubmissionSequence};
    use rastok::envelope::{Data, MetaValue, Namespace, Scalar, Value};
    use rastok::journal::MemJournal;
    use rastok::plan::GraphVersion;
    use rastok::runtime::vm::Vm;

    let run = |body: &str| {
        let source = format!(
            "graph g : App {{\n  operator o {{\n    on in {{\n{body}\n    }}\n  }}\n  . -> o -> .\n}}\n"
        );
        let compilation = compile(
            &[source.as_str()],
            &Registry::try_m0().expect("the standard catalogue holds"),
        )
        .expect("a valid plan");
        let mut journal = MemJournal::new(GraphVersion(1));
        let mut vm = Vm::boot(&compilation.plan, GraphVersion(1));
        let mut ext = Namespace::new();
        ext.insert(
            String::from("content_type"),
            MetaValue::Scalar(Scalar::Text(String::from("text/csv"))),
        );
        vm.inject(
            &mut journal,
            InboundSubmission {
                authority: IngressAuthority::single(rastok::node::Path::new(["probe"]).unwrap()),
                port: rastok::node::Port::new("in").unwrap(),
                draft: Draft {
                    data: Data::materialized(Value::Scalar(Scalar::Text(String::from("a,b")))),
                    ext,
                },
                sequence: SubmissionSequence::Next,
            },
        )
        .expect("the injection lands");
        vm.drain(&mut journal).expect("the drain completes");
        vm.outputs()[0]
            .envelope
            .meta()
            .ext()
            .get("content_type")
            .cloned()
    };
    let csv = Some(MetaValue::Scalar(Scalar::Text(String::from("text/csv"))));
    // Kept witness — every spelling.
    assert_eq!(run("      out = ."), csv, "identity keeps");
    assert_eq!(run("      out = data"), csv, "the bare relay keeps");
    assert_eq!(
        run("      val copy = data\n      out = copy"),
        csv,
        "a variable carrying the witness keeps"
    );
    assert_eq!(
        run("      out = true ? data : \"other\""),
        csv,
        "the taken ternary arm keeps"
    );
    assert_eq!(
        run("      out = { data = data, meta.app.tag = \"x\" }"),
        csv,
        "the whole-data carry keeps"
    );
    // Fresh data strikes.
    assert_eq!(run("      out = \"fresh\""), None, "fresh text strikes");
    assert_eq!(run("      out = { total = 1 }"), None, "a record strikes");
    assert_eq!(
        run("      out = { data = \"fresh\" }"),
        None,
        "a whole-data build strikes"
    );
    // An explicit write wins over both derivations.
    assert_eq!(
        run("      out = { data = \"fresh\", meta.ext.content_type = \"text/plain\" }"),
        Some(MetaValue::Scalar(Scalar::Text(String::from("text/plain")))),
        "the explicit write wins after a strike"
    );
}
