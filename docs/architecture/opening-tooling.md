# Opening tooling qualification

The runtime, content validation and editor package introduce no Node dependency.
Node is a **test-host tool** for the existing debugger/layout harness. The site
build remains in another repository; this repository tests Markdown contracts
with its existing Rust `serde_yaml_ng` dependency.

## Browser test runner

Capability: run the existing `browser.test.cjs` against an isolated real browser,
including keyboard controls, recordings and console errors. Rust HTTP probes
cannot check DOM interaction or geometry. Playwright is maintained by Microsoft;
the pinned test version is **1.58.2**, Apache-2.0, depending on matching
`playwright-core` (and optional platform-specific packaging). Registry metadata
was inspected before selection. No package is linked into Rastok, shipped with
the debugger, or added to a repository `package.json`.

CI installs the runner into its temporary directory. Browser download is an
explicit test setup step. It exercises local synthetic fixtures only, without
reusing a personal browser profile. Native Chrome can be selected with
`RASTOK_CHROME`; otherwise Playwright uses its pinned Chromium. The exit path is
to remove this test-host job or select another external runner; neither changes
the static page, v17, any Contract or the Rust dependency tree.

`RASTOK_BIN` can select a copied qualification binary. Use it when running
different Cargo feature matrices nearby: a featureless build must not replace
the debugger executable between browser cases. The default remains
`target/debug/rastok` for the CI build-then-test sequence.

The editor grammar is original Apache-2.0 JSON under
`editors/vscode/rastok-dsl`. No seed implementation or third-party grammar was
copied. Its first qualification checks the manifest, includes and representative
v17 lexical rules. This is syntax highlighting, not a second checker.

## Release artifacts

The manual `alpha-artifacts` workflow builds macOS arm64, macOS x86-64 and
Linux x86-64 with `hosted-ui,hosted-websocket` on Rust 1.97.1. It neither includes
`hosted-http3` nor creates a tag, GitHub release or registry publication.
An uploaded CI artifact is qualification evidence, not an announced release.
Cross-host builds and installation are unqualified until that workflow runs
and the actual outputs are checked. Public publication waits for 15 October
and Yoann's explicit order.
