//! A handler filter ends before its Dispatch without changing admission,
//! narrowing, plan identity or the journal.

#![allow(clippy::disallowed_types, clippy::disallowed_methods)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use rastok::compiler::compile;
use rastok::connector::contract::ContractCatalog;
use rastok::journal::Record;
use rastok::plan::{self, GraphVersion};

#[test]
fn a_where_match_is_identical_to_its_explicit_nested_dispatch() {
    let source = r#"
type Event = union {
  Accepted { score: int }
  Ignored {}
}
graph filter : App {
  input in: Event
  output out: int
  operator choose {
    input in: Event
    output out: int
    on in where data is Accepted match {
      data.score > 0 => out = data.score
      _ => out = 0
    }
  }
  in -> choose -> out
}
lab filter {
  simulation decisions {
    scenario filter_then_decide {
      app.in = Ignored {}
      expect app.out == nothing
      app.in = Accepted { score = 7 }
      expect app.out where data == 7
      app.in = Accepted { score = -3 }
      expect app.out where data == 0
    }
  }
}
"#;
    let explicit = source.replace(
        "on in where data is Accepted match {\n      data.score > 0 => out = data.score\n      _ => out = 0\n    }",
        "on in match {\n      data is Accepted => match {\n        data.score > 0 => out = data.score\n        _ => out = 0\n      }\n      _ => nothing\n    }",
    );
    assert_ne!(source, explicit);
    let registry = ContractCatalog::try_m0().unwrap();
    let filtered = compile(&[source], &registry).unwrap();
    let nested = compile(&[&explicit], &registry).unwrap();
    assert_eq!(filtered.hash, nested.hash);
    assert_eq!(filtered.plan, nested.plan);
    let bytes = plan::encode(&filtered.plan, &filtered.hash);
    assert_eq!(bytes, plan::encode(&nested.plan, &nested.hash));
    let (decoded, hash) = plan::decode(&bytes).unwrap();
    assert_eq!(decoded, filtered.plan);
    assert_eq!(hash, filtered.hash);

    let runs = rastok::lab::run(&decoded, GraphVersion(1), &filtered.labs[0]);
    let twins = rastok::lab::run(&nested.plan, GraphVersion(1), &nested.labs[0]);
    assert_eq!(runs.len(), 1);
    assert!(runs[0].report.passed(), "{:?}", runs[0].report);
    assert_eq!(runs[0].records, twins[0].records);
    let decisions: Vec<_> = runs[0]
        .records
        .iter()
        .filter_map(|record| match record {
            Record::Transaction(transaction)
                if transaction.owner_path.to_string() == "choose"
                    && !transaction.consumed.is_empty() =>
            {
                Some(transaction)
            }
            _ => None,
        })
        .collect();
    assert_eq!(decisions.len(), 3, "the false filter consumes too");
    assert!(decisions[0].emissions.is_empty());
}

#[test]
fn a_join_where_match_retains_unmatched_candidates_before_dispatch() {
    for bounds in [
        "",
        "last 3",
        "within duration(\"1s\")",
        "within duration(\"1s\") last 3",
    ] {
        let source = format!(
            r#"
type Expiry = union {{ JoinExpired {{}} }}
graph pairing : App {{
  input left: int
  input right: int
  output out: int
  operator pair {{
    input left: int
    input right: int
    output out: int
    output expired: Expiry
    on (left, right) where left.data == right.data {bounds} match left.data > 1 {{
      true => out = left.data + right.data
      false => out = 0
    }}
  }}
  left -> pair.left
  right -> pair.right
  pair.out -> out
  pair.expired -> nothing
}}
lab pairing {{
  simulation candidates {{
    scenario keep_then_match {{
      app.left = 1
      app.right = 2
      expect app.out == nothing
      app.right = 1
      expect app.out where data == 0
      app.left = 2
      expect app.out where data == 4
    }}
  }}
}}
"#
        );
        let result = compile(&[&source], &ContractCatalog::try_m0().unwrap());
        if bounds.is_empty() {
            let bag = result.expect_err("a content join still requires a retention bound");
            assert_eq!(bag.diagnostics().len(), 1, "{bag:?}");
            assert_eq!(bag.diagnostics()[0].code().as_str(), "E-INSTANCE");
            assert!(
                bag.diagnostics()[0]
                    .message()
                    .contains("`within` or `last`")
            );
            continue;
        }
        let compilation = result.unwrap();
        let bytes = plan::encode(&compilation.plan, &compilation.hash);
        let (decoded, _) = plan::decode(&bytes).unwrap();
        let runs = rastok::lab::run(&decoded, GraphVersion(1), &compilation.labs[0]);
        assert_eq!(runs.len(), 1);
        assert!(runs[0].report.passed(), "{bounds}: {:?}", runs[0].report);
        assert_eq!(
            runs[0]
                .records
                .iter()
                .filter(|record| matches!(record, Record::Transaction(transaction)
                    if transaction.join_inputs.len() == 2))
                .count(),
            2,
            "a false admission must not consume either candidate"
        );
    }
}

#[test]
fn a_malformed_where_before_match_is_an_expression_error_not_a_statement_error() {
    let source = "graph g : App { input in: int; operator o { input in: int; on in where data > match { true => nothing; _ => nothing } } in -> o }";
    let bag = compile(&[source], &ContractCatalog::try_m0().unwrap()).unwrap_err();
    assert_eq!(bag.diagnostics().len(), 1, "{bag:?}");
    assert_eq!(bag.diagnostics()[0].code().as_str(), "E-EXPR");
    let span = bag.diagnostics()[0].spans()[0].range();
    assert!(span.start >= source.find("data >").unwrap());
    assert!(span.end <= source.find("match {").unwrap());
}
