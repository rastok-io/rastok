fn main() {
    // A categorical-package candidate can be measured before blessing the
    // tracked fixture. CI without the override still embeds the tracked plan.
    println!("cargo:rerun-if-env-changed=RASTOK_SPIKE_PACKAGE");
    let package = std::env::var_os("RASTOK_SPIKE_PACKAGE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("echo.rkp"));
    let package = package.canonicalize().expect("the spike plan exists");
    println!("cargo:rerun-if-changed={}", package.display());
    println!("cargo:rustc-env=RASTOK_SPIKE_PACKAGE={}", package.display());
    // Expose memory.x to the linker.
    println!(
        "cargo:rustc-link-search={}",
        std::env::var("CARGO_MANIFEST_DIR").unwrap()
    );
}
