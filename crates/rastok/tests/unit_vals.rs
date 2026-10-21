//! File-level constants share one lexical unit with the App and its Lab.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use rastok::{compiler::compile, connector::contract::ContractCatalog, plan};

const GRAPH: &str = r#"
graph constants : App {
  input in: text
  output out: text
  graph nested {
    input in: text
    output out: text
    val prefix = "reply: "
    operator prepare {
      input in: text
      output out: text
      var received: int = 0
      on in {
        val content = data
        received = received + 1
        out = prefix + content
      }
    }
    connector echo : Process {
      command = "cat"
      codec = lines
      idempotent = true
      behavior = deterministic
    }
    in -> prepare -> echo -> out
    echo.err -> nothing
    echo.exit -> nothing
  }
  in -> nested -> out
}
"#;

const LAB: &str = r#"
lab constants {
  simulation scripted on app {
    mock nested.echo {
      on in where data == "reply: hello" {
        val reply = "accepted"
        out = reply
        exit = Exited { exit_code = 0, signal = null }
      }
    }
    scenario roundtrip {
      app.in = { meta.ext.session_id = "22222222222222222222222222222222"; data = "hello" }
      expect app.out count == 1 where data == "accepted" && meta.ext.session_id == "22222222222222222222222222222222"
      advance duration("1s")
      expect nothing app.out
    }
  }
}
"#;

#[test]
fn unit_vals_reach_graphs_attributes_mocks_and_scenarios_without_identity_changes() {
    let catalogue = ContractCatalog::try_m0().unwrap();
    let globals = r#"
val greeting: text = "hello"
val prefix: text = "reply: "
val request = prefix + greeting
val response = "accepted"
val command = "cat"
val bob_session = "22222222222222222222222222222222"
val pause = duration("1s")
"#;
    // Keep the Graph-local name distinct: globals cannot be shadowed.
    let graph = GRAPH
        .replace("val prefix = \"reply: \"", "val local_prefix = prefix")
        .replace("out = prefix + content", "out = local_prefix + content")
        .replace("command = \"cat\"", "command = command");
    let local_literal = GRAPH
        .replace("val prefix = \"reply: \"", "val local_prefix = \"reply: \"")
        .replace("out = prefix + content", "out = local_prefix + content");
    let expected = compile(&[&local_literal, LAB], &catalogue).unwrap();
    let lab = LAB
        .replace("\"reply: hello\"", "request")
        .replace("\"accepted\"", "response")
        .replace("\"hello\"", "greeting")
        .replace("\"22222222222222222222222222222222\"", "bob_session")
        .replace("duration(\"1s\")", "pause");
    // A globals-only file is visible even when passed after the App and Lab.
    let actual = compile(&[&graph, &lab, globals], &catalogue).unwrap();
    assert_eq!(actual.hash, expected.hash);
    assert_eq!(
        plan::encode(&actual.plan, &actual.hash),
        plan::encode(&expected.plan, &expected.hash)
    );
    let results = rastok::lab::run(&actual.plan, plan::GraphVersion(1), &actual.labs[0]);
    assert!(results.iter().all(|run| run.report.passed()), "{results:?}");
    assert_eq!(
        results,
        rastok::lab::run(&expected.plan, plan::GraphVersion(1), &expected.labs[0])
    );
}

#[test]
fn lab_only_constants_do_not_enter_the_production_package() {
    let catalogue = ContractCatalog::try_m0().unwrap();
    let expected = compile(&[GRAPH, LAB], &catalogue).unwrap();
    let lab = LAB.replace("\"22222222222222222222222222222222\"", "bob_session");
    let actual = compile(
        &[
            "val bob_session = \"22222222222222222222222222222222\"\nval unused = 42\n",
            GRAPH,
            &lab,
        ],
        &catalogue,
    )
    .unwrap();
    assert_eq!(actual.hash, expected.hash);
    assert_eq!(actual.plan, expected.plan);
    assert_eq!(actual.labs, expected.labs);
}

#[test]
fn global_constants_keep_structured_values_and_typed_paths() {
    let source = r#"
type Message = struct { text: text, n: int }
val message: Message = { text = "hello"; n = 2 }
val names = ["alice", "bob"]
val labels = { "greeting": "hello" }
graph constants : App {
  input in: text
  output out: text
  operator echo {
    input in: text
    output out: text
    on in { out = message.text + names[1] + labels.greeting }
  }
  in -> echo -> out
}
lab constants {
  simulation s on app {
    scenario s {
      app.in = message.text
      expect app.out where data == "hellobobhello"
    }
  }
}
"#;
    let compilation = compile(&[source], &ContractCatalog::try_m0().unwrap()).unwrap();
    let runs = rastok::lab::run(
        &compilation.plan,
        plan::GraphVersion(1),
        &compilation.labs[0],
    );
    assert!(runs.iter().all(|run| run.report.passed()), "{runs:?}");
}

#[test]
fn global_constants_reject_duplicates_shadowing_and_nonconstant_values() {
    let catalogue = ContractCatalog::try_m0().unwrap();
    for (source, code) in [
        ("val x = 1\nval x = 2\ngraph g : App {}", "E-DUP-BINDING"),
        ("val x = 1\ngraph g : App { val x = 2 }", "E-DUP-BINDING"),
        ("val data = 1\ngraph g : App {}", "E-DUP-BINDING"),
        ("val instance = 1\ngraph g : App {}", "E-DUP-BINDING"),
        ("val Loop = 1\ngraph g : App {}", "E-DUP-BINDING"),
        ("val now = 1\ngraph g : App {}", "E-NAME-RESERVED"),
        ("val x: int = \"wrong\"\ngraph g : App {}", "E-PORT-TYPE"),
        ("val x = data\ngraph g : App {}", "E-EXPR"),
        ("val x = now()\ngraph g : App {}", "E-EXPR"),
        ("val x = env(\"TOKEN\")\ngraph g : App {}", "E-EXPR"),
        ("val x = secret(\"token\")\ngraph g : App {}", "E-EXPR"),
        ("val x = instance.key\ngraph g : App {}", "E-EXPR"),
        ("val x = x\ngraph g : App {}", "E-EXPR"),
        ("val x = later\nval later = 1\ngraph g : App {}", "E-EXPR"),
        ("val unused = 1 / 0\ngraph g : App {}", "E-EXPR"),
    ] {
        let bag = compile(&[source], &catalogue).expect_err(source);
        assert!(
            bag.diagnostics().iter().any(|d| d.code().as_str() == code),
            "{source}: {bag:?}"
        );
        assert!(
            bag.diagnostics().iter().any(|d| !d.spans().is_empty()),
            "{bag:?}"
        );
    }
}

#[test]
fn global_constants_cannot_erase_data_policy() {
    let catalogue = ContractCatalog::try_m0().unwrap();
    for declaration in [
        "val value: text @sensitive(level = restricted) = \"private\"",
        "type Private = text @personal\nval value: Private = \"private\"",
    ] {
        let source = format!("{declaration}\ngraph g : App {{}}\n");
        let bag = compile(&[&source], &catalogue).expect_err("policy cannot be inlined away");
        assert!(
            bag.diagnostics().iter().any(|d| {
                d.code().as_str() == "E-CHECK-DEFERRED" && d.message().contains("annotated unit")
            }),
            "{bag:?}"
        );
    }
}

#[test]
fn graph_and_handler_vals_remain_local() {
    let catalogue = ContractCatalog::try_m0().unwrap();
    let source = format!("{GRAPH}\n{}", LAB.replace("\"reply: hello\"", "prefix"));
    let bag =
        compile(&[&source], &catalogue).expect_err("Graph constants do not escape into the Lab");
    assert!(
        bag.diagnostics()
            .iter()
            .any(|d| d.code().as_str() == "E-EXPR")
    );
    for body in ["val greeting = \"other\"", "var greeting: text = \"other\""] {
        let source = format!(
            "val greeting = \"hello\"\ngraph g : App {{ operator o {{ input in: text; {body}\non in {{ nothing }} }}\nin -> o }}"
        );
        let bag = compile(&[&source], &catalogue).expect_err("no implicit shadowing");
        assert!(
            bag.diagnostics()
                .iter()
                .any(|d| d.code().as_str() == "E-DUP-BINDING")
        );
    }
}

#[test]
fn constant_map_reads_remain_lazy_and_refuse_a_missing_key_when_evaluated() {
    let catalogue = ContractCatalog::try_m0().unwrap();
    let source = r#"
val labels = { "present": "yes" }
graph constants : App {
  input in: bool
  output out: text
  operator echo {
    input in: bool
    output out: text
    on in { out = data ? labels.missing : labels.present }
  }
  in -> echo -> out
}
lab constants {
  simulation s on app {
    scenario s {
      app.in = false
      expect app.out where data == "yes"
    }
  }
}
"#;
    let compilation = compile(&[source], &catalogue).unwrap();
    let runs = rastok::lab::run(
        &compilation.plan,
        plan::GraphVersion(1),
        &compilation.labs[0],
    );
    assert!(runs.iter().all(|run| run.report.passed()), "{runs:?}");
    let bad =
        "val labels = { \"present\": \"yes\" }\nval missing = labels.absent\ngraph g : App {}";
    let bag = compile(&[bad], &catalogue).expect_err("the initializer actually evaluates the read");
    assert!(
        bag.diagnostics()
            .iter()
            .any(|d| d.code().as_str() == "E-EXPR" && !d.message().contains("unbound")),
        "{bag:?}"
    );
}

#[test]
fn attribute_choice_words_are_not_captured_by_global_bindings() {
    let catalogue = ContractCatalog::try_m0().unwrap();
    let expected = compile(&[GRAPH], &catalogue).unwrap();
    let actual = compile(
        &[
            "val lines = \"not a codec\"\nval deterministic = 123\n",
            GRAPH,
        ],
        &catalogue,
    )
    .unwrap();
    assert_eq!(actual.hash, expected.hash);
    assert_eq!(actual.plan, expected.plan);
}

#[test]
fn join_aliases_cannot_capture_a_global_binding() {
    let source = r#"
val left = "constant"
graph joined : App {
  input in: text
  output out: text
  operator pair {
    input left: text
    input right: text
    output out: text
    on (left, right) { out = left.data + right.data }
  }
  in -> { pair.left; pair.right }
  pair -> out
}
"#;
    let bag = compile(&[source], &ContractCatalog::try_m0().unwrap())
        .expect_err("an envelope alias cannot capture the constant");
    assert!(
        bag.diagnostics()
            .iter()
            .any(|d| d.code().as_str() == "E-DUP-BINDING"),
        "{bag:?}"
    );
}

#[test]
fn typed_constants_preserve_tagged_and_decimal_values() {
    let source = r#"
type Reply = union { Ok { message: text } Failed { reason: text } }
type Price = decimal(10,2)
val answer: Reply = Ok { message = "done" }
val price: Price = decimal("1.20") as Price
graph typed : App {
  input in: text
  output out: text
  operator reply {
    input in: text
    output out: text
    on in match {
      answer is Ok => out = answer.message
      answer is Failed => out = answer.reason
      _ => nothing
    }
  }
  in -> reply -> out
}
lab typed {
  simulation s on app {
    scenario s {
      app.in = "go"
      expect app.out where data == "done" && price == decimal("1.20") && answer is Ok && answer.message == data
    }
  }
}
"#;
    let compilation = compile(&[source], &ContractCatalog::try_m0().unwrap()).unwrap();
    let runs = rastok::lab::run(
        &compilation.plan,
        plan::GraphVersion(1),
        &compilation.labs[0],
    );
    assert!(runs.iter().all(|run| run.report.passed()), "{runs:?}");
}
