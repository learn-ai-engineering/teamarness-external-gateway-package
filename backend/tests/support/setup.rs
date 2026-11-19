#![allow(dead_code)]
use crate::membership_support;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{path::PathBuf, sync::Arc};
use teamarness_generated_package_backend::{
    State, managed_install::Admitted, setup::BackendEnvironment,
};
use teamarness_package_sdk::testing::{Harness, HarnessBuilder, ScriptedCapability};
use teamarness_package_sdk::{Context, Empty};
pub struct Fixture {
    pub data: tempfile::TempDir,
    pub home: tempfile::TempDir,
    pub source: PathBuf,
}
/// An owner-only temporary directory, independent of the process umask (Ubuntu's
/// default 0002 would otherwise create a group-writable home the stores refuse).
pub fn private_dir() -> tempfile::TempDir {
    use std::os::unix::fs::PermissionsExt;
    tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap()
}
/// Creates one owner-only directory, independent of the process umask.
pub fn create_private_dir(path: impl AsRef<std::path::Path>) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new().mode(0o700).create(path)
}
/// Writes a file with an explicit owner-only mode, as the Host admits Backend bytes.
pub fn write_private(path: &std::path::Path, bytes: &[u8]) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, bytes).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
}
impl Fixture {
    pub fn new() -> Self {
        let data = private_dir();
        let home = private_dir();
        let source = data.path().canonicalize().unwrap().join("source");
        write_private(&source, b"admitted setup fixture v1");
        Self { data, home, source }
    }
    pub fn environment(&self) -> BackendEnvironment {
        self.environment_version("1.0.0")
    }
    pub fn environment_version(&self, version: &str) -> BackendEnvironment {
        let digest = Sha256::digest(std::fs::read(&self.source).unwrap())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        BackendEnvironment::injected(self.home.path().canonicalize().unwrap()).with_admitted_source(
            self.source.clone(),
            Admitted::new(&"b".repeat(64), version, &digest).unwrap(),
        )
    }
    pub async fn start(&self) -> (Harness, Arc<State>) {
        self.start_with(self.environment()).await
    }
    pub async fn start_with(&self, e: BackendEnvironment) -> (Harness, Arc<State>) {
        self.start_as(e, &"b".repeat(64)).await
    }
    pub async fn start_as(
        &self,
        e: BackendEnvironment,
        installation: &str,
    ) -> (Harness, Arc<State>) {
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        let r = teamarness_generated_package_backend::router_with_environment(e).request(
            "test.capture",
            move |c: Context<State>, _: Empty| {
                let tx = tx.clone();
                async move {
                    tx.send(c.state().clone()).await.unwrap();
                    Ok(json!({}))
                }
            },
        );
        let mut h = HarnessBuilder::new(r.into_backend())
            .package_identity("dev.teamarness.external-gateway")
            .package_version(teamarness_generated_package_backend::PACKAGE_VERSION)
            .process_minor(1)
            .data_directory(self.data.path().canonicalize().unwrap())
            .installation(installation, "Local")
            .capability("teamarness.membership", membership_support::present())
            .capability_version("teamarness.package-ports", ScriptedCapability::new(), 3, 0)
            .start()
            .await;
        h.activate().await.unwrap();
        h.request("test.capture", json!({})).await.unwrap();
        let s = rx.recv().await.unwrap();
        (h, s)
    }
    pub fn connections(
        &self,
    ) -> Vec<teamarness_generated_package_backend::client_store::Connection> {
        teamarness_generated_package_backend::client_store::ClientStore::for_home(
            &self.home.path().canonicalize().unwrap(),
        )
        .unwrap()
        .connections()
        .unwrap()
    }
}
pub async fn finish(h: &mut Harness) -> Value {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let v = h
                .request("external-gateway.setup.status", json!({}))
                .await
                .unwrap();
            if !["accepted", "prepared", "outputs", "committed"]
                .contains(&v["phase"].as_str().unwrap())
            {
                return v;
            }
            tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        }
    })
    .await
    .unwrap()
}
pub async fn apply(h: &mut Harness, intent: &str) -> Value {
    let p = h
        .request("external-gateway.setup.preview", json!({"intent":intent}))
        .await
        .unwrap();
    h.request(
        "external-gateway.setup.apply",
        json!({"planId":p["planId"]}),
    )
    .await
    .unwrap();
    finish(h).await
}
pub async fn connect(h: &mut Harness) -> Value {
    h.request("external-gateway.listener.set", json!({"enabled":true}))
        .await
        .unwrap();
    let s = apply(h, "connect").await;
    assert_eq!(s["phase"], "complete", "{s}");
    s
}
