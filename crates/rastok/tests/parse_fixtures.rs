// A test harness drives the OS on purpose; the frontier lints
// guard the core modules, not the evidence.
#![allow(clippy::disallowed_types, clippy::disallowed_methods)]
//! Fixture-driven parser evidence: every `.rkg` under `tests/fixtures/`
//! must parse to a stable outcome. A sibling `.diag` file holds the
//! expected rendered diagnostics — absent or empty means the source is
//! accepted cleanly; a sibling `.ast` file, when present, pins the
//! parsed tree itself. Run with `RASTOK_BLESS=1` to regenerate the
//! goldens after an intentional change, then review the diff.

use rastok::compiler::parser::parse;
use rastok::contract::ContractCatalog;
use rastok::diagnostics::{Bag, SourceId};
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

#[test]
fn fixtures_match_their_goldens() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let bless = std::env::var_os("RASTOK_BLESS").is_some();
    let registry = ContractCatalog::try_m0().expect("the standard frontend catalogue is valid");
    let mut fixtures = 0;
    let mut entries: Vec<_> = fs::read_dir(&root)
        .expect("fixtures directory")
        .map(|entry| entry.expect("fixture entry").path())
        .collect();
    entries.sort();
    for path in entries {
        if path.extension().is_none_or(|extension| extension != "rkg") {
            continue;
        }
        fixtures += 1;
        let source = fs::read_to_string(&path).expect("fixture source");
        let mut bag = Bag::new();
        let tree = parse(SourceId::new(0), &source, &registry, &mut bag);
        let mut rendered = String::new();
        for diagnostic in bag.diagnostics() {
            writeln!(rendered, "{diagnostic}").expect("string write");
        }
        let diag_path = path.with_extension("diag");
        let ast_path = path.with_extension("ast");
        let ast = format!("{tree:#?}\n");
        if bless {
            if rendered.is_empty() {
                let _ = fs::remove_file(&diag_path);
            } else {
                fs::write(&diag_path, &rendered).expect("bless diagnostics");
            }
            if ast_path.exists() {
                fs::write(&ast_path, &ast).expect("bless tree");
            }
        } else {
            let expected = fs::read_to_string(&diag_path).unwrap_or_default();
            assert_eq!(
                rendered,
                expected,
                "diagnostics diverged for {} (RASTOK_BLESS=1 to re-bless)",
                path.display()
            );
            if ast_path.exists() {
                let expected_tree = fs::read_to_string(&ast_path).expect("tree golden");
                assert_eq!(
                    ast,
                    expected_tree,
                    "tree diverged for {} (RASTOK_BLESS=1 to re-bless)",
                    path.display()
                );
            }
        }
    }
    assert!(fixtures >= 4, "expected fixtures on disk, found {fixtures}");
}
