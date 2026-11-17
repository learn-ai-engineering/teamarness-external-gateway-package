//! Every automated Backend activation has its own private physical home.
#![allow(dead_code)]
use std::sync::Arc;
use teamarness_generated_package_backend::{
    State,
    config::StorageFault,
    setup::{BackendEnvironment, Clock},
    tokens::Entropy,
};
use teamarness_package_sdk::Router;
struct PrivateHome {
    _directory: tempfile::TempDir,
}
impl Clock for PrivateHome {
    fn seconds(&self) -> u64 {
        0
    }
}
fn environment() -> BackendEnvironment {
    let directory = tempfile::tempdir().unwrap();
    let mut e = BackendEnvironment::injected(directory.path().canonicalize().unwrap());
    e.clock = Arc::new(PrivateHome {
        _directory: directory,
    });
    e
}
pub fn router() -> Router<State> {
    teamarness_generated_package_backend::router_with_environment(environment())
}
pub fn router_with_entropy(entropy: Arc<dyn Entropy>) -> Router<State> {
    teamarness_generated_package_backend::router_with_environment_and_entropy(
        environment(),
        entropy,
    )
}
pub fn router_with_storage_fault(
    entropy: Arc<dyn Entropy>,
    fault: Option<Arc<dyn StorageFault>>,
) -> Router<State> {
    let mut e = environment();
    e.storage_fault = fault;
    teamarness_generated_package_backend::router_with_environment_and_entropy(e, entropy)
}
