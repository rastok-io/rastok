//! `File` through the connector-neutral conformance battery.
//!
//! The fixtures are physical because this profile proves the real Driver.
//! Contract identity, typed completion, correlation and journal ordering are
//! still judged by the shared harness rather than by File-private tests.

#![allow(clippy::disallowed_types, clippy::disallowed_methods)]
#![allow(clippy::expect_used)]

use rastok::conformance::{
    DriverTestFixture, ExitCaseTest, InterleaveExpectation, InterleaveProbe, assert_conformant,
    verify,
};
use rastok::connectors::{DriverRegistry, FileFactory};
use rastok::contract::ContractCatalog;
use rastok::envelope::{Data, Encoding};
use rastok::node;
use rastok::stdlib::FILE;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
struct TestRoot(PathBuf);

impl TestRoot {
    fn new() -> Self {
        let nonce = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "rastok-file-conformance-{}-{nonce}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("the conformance root is writable");
        std::fs::write(root.join("happy.bin"), b"happy").expect("the happy fixture writes");
        std::fs::write(root.join("left.bin"), b"left-result").expect("the left fixture writes");
        std::fs::write(root.join("right.bin"), b"right-result").expect("the right fixture writes");
        std::fs::write(root.join("large.bin"), b"too-large").expect("the bounded fixture writes");
        std::fs::create_dir(root.join("directory")).expect("the shape fixture writes");
        Self(root)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn read_probe(name: &str, path: &str, max_read: &str) -> String {
    format!(
        r#"
graph {name} : App {{
  connector file : File {{
    mode = read
    path = "{path}"
    max_read = "{max_read}"
  }}

  in -> file
  file.out -> out
  file.exit -> nothing
}}
"#
    )
}

fn pair_probe() -> String {
    String::from(
        r#"
graph pair : App {
  connector left : File {
    mode = read
    path = "left.bin"
    max_read = "32B"
  }

  connector right : File {
    mode = read
    path = "right.bin"
    max_read = "32B"
  }

  in -> left
  in -> right
  left.out -> out
  right.out -> out
  left.exit -> nothing
  right.exit -> nothing
}
"#,
    )
}

fn file_registry(root: &Path) -> DriverRegistry {
    DriverRegistry::default()
        .try_with(Box::new(
            FileFactory::try_new(root).expect("the File id is canonical"),
        ))
        .expect("File is registered once")
}

#[test]
fn file_passes_the_connector_neutral_battery() {
    let root = TestRoot::new();
    let profile = DriverTestFixture {
        contract: &FILE,
        source: read_probe("happy", "happy.bin", "16B"),
        input: "ignored",
        happy_exit_case: "Completed",
        provocations: vec![
            ExitCaseTest {
                exit_case: "NotFound",
                source: read_probe("missing", "missing.bin", "16B"),
                input: "ignored",
            },
            ExitCaseTest {
                exit_case: "NotAFile",
                source: read_probe("directory", "directory", "16B"),
                input: "ignored",
            },
            ExitCaseTest {
                exit_case: "InvalidPath",
                source: read_probe("invalid", "../outside", "16B"),
                input: "ignored",
            },
            ExitCaseTest {
                exit_case: "ReadLimitExceeded",
                source: read_probe("bounded", "large.bin", "4B"),
                input: "ignored",
            },
        ],
        untestable: vec![
            (
                "PermissionDenied",
                "no portable fixture can force host permission denial across supported systems",
            ),
            (
                "AtomicReplaceUnavailable",
                "the conformance host may provide the required same-directory atomic rename",
            ),
            (
                "IoFailed",
                "portable filesystem APIs expose no deterministic generic I/O fault fixture",
            ),
            (
                "TimeoutExpired",
                "a bounded one-file operation cannot be made portably and deterministically mute",
            ),
            (
                "OutcomeUnknown",
                "bounded Closing, not the File Driver, produces this case after settlement expires",
            ),
        ],
        mute_source: None,
        interleave: Some(InterleaveProbe {
            source: pair_probe(),
            a: "request-a",
            b: "request-b",
            expected: vec![
                InterleaveExpectation {
                    connector: node::Path::new(["left"]).expect("the left path is valid"),
                    data: Data::encoded(Encoding::Raw, b"left-result".to_vec()),
                },
                InterleaveExpectation {
                    connector: node::Path::new(["right"]).expect("the right path is valid"),
                    data: Data::encoded(Encoding::Raw, b"right-result".to_vec()),
                },
            ],
        }),
    };
    let report = verify(
        &profile,
        &ContractCatalog::try_m0().expect("the standard catalogue holds"),
        &file_registry(root.path()),
    );

    assert_conformant(
        &report,
        &[
            "PermissionDenied",
            "AtomicReplaceUnavailable",
            "IoFailed",
            "TimeoutExpired",
            "OutcomeUnknown",
        ],
    );
    assert_eq!(
        report.proven_cases,
        [
            "Completed",
            "NotFound",
            "NotAFile",
            "InvalidPath",
            "ReadLimitExceeded",
        ]
    );
}
