fn main() {
    println!("cargo:rerun-if-changed=../teamarness.package.json");
    let manifest: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string("../teamarness.package.json").expect("Package manifest"),
    )
    .expect("valid Package manifest");
    let version = manifest["version"].as_str().expect("Package version");
    semver::Version::parse(version).expect("valid Package release version");
    println!("cargo:rustc-env=TEAMARNESS_PACKAGE_VERSION={version}");
}
