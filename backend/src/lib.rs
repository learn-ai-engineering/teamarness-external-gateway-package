#![forbid(unsafe_code)]
//! Standalone Person-authorized exact-tool policy and bounded loopback HTTP.
pub const PACKAGE_VERSION: &str = env!("TEAMARNESS_PACKAGE_VERSION");
pub mod catalog;
pub mod cli;
pub mod client_store;
pub mod config;
pub mod http;
pub mod limits;
pub mod listener;
pub mod managed_install;
mod membership;
pub mod picker;
pub mod picker_helper;
pub mod policy;
pub mod setup;
pub mod skill_install;
mod streams;
pub mod tokens;
use policy::{Change, Policy};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;
use teamarness_package_sdk::{BackendError, Context, Empty, Router, event_channel};

pub fn error(code: &str) -> BackendError {
    BackendError::new(
        format!("external_gateway.{code}"),
        matches!(
            code,
            "busy" | "rate_limited" | "timeout" | "unavailable" | "storage_failed"
        ),
        json!({}),
    )
}
pub struct State {
    pub policy: Arc<Policy>,
    membership: membership::Monitor,
    pub setup: Arc<setup::SetupCoordinator>,
    pub picker: Arc<picker::Picker>,
}
impl Drop for State {
    fn drop(&mut self) {
        self.picker.close();
        self.policy.retire();
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Enabled {
    enabled: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Port {
    port: u16,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Label {
    label: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Id {
    id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Page {
    cursor: Option<String>,
    contract: Option<String>,
    #[serde(default)]
    offset: usize,
}

pub fn router() -> Router<State> {
    router_with_entropy(Arc::new(tokens::OsEntropy))
}
pub fn router_with_entropy(entropy: Arc<dyn tokens::Entropy>) -> Router<State> {
    router_with_storage_fault(entropy, None)
}
pub fn router_with_storage_fault(
    entropy: Arc<dyn tokens::Entropy>,
    fault: Option<Arc<dyn config::StorageFault>>,
) -> Router<State> {
    let mut environment = setup::BackendEnvironment::default();
    environment.storage_fault = fault;
    router_with_environment_and_entropy(environment, entropy)
}
pub fn router_with_environment(environment: setup::BackendEnvironment) -> Router<State> {
    router_with_environment_and_entropy(environment, Arc::new(tokens::OsEntropy))
}
pub fn router_with_environment_and_entropy(
    environment: setup::BackendEnvironment,
    entropy: Arc<dyn tokens::Entropy>,
) -> Router<State> {
    Router::new(move |activation| {
        let workspace=json!({"id":activation.initialization().workspace().id,"name":activation.initialization().workspace().name});
        let path=activation.data_directory().to_owned();let entropy=entropy.clone();let fault=environment.storage_fault.clone();let environment=environment.clone();let init=activation.initialization().clone();
        let membership=membership::Monitor::new(activation.initialization(),activation.capabilities().clone());
        let client=teamarness_package_sdk::ports_v3::PortClientV3::from_initialization(activation.initialization(),activation.capabilities().clone());
        async move {
            let client=client?;
            let membership=membership?;
            let store=tokio::task::spawn_blocking(move||config::FileStore::open(&path).map(|store|match fault {Some(f)=>store.with_fault(f),None=>store})).await.map_err(|_|error("storage_failed"))?.map_err(|_|error("storage_failed"))?;
            let policy=Policy::open(store,catalog::Catalog::new(client),entropy,workspace).await?;
            membership.start(&policy).await;
            policy.resume().await;
            let (source,admitted)=environment.executable(&init)?;
            let picker=picker::Picker::new(source,admitted);
            let setup=setup::SetupCoordinator::open(policy.clone(),&init,environment).await?;
            Ok(State{policy,membership,setup,picker})
        }
    })
    .on_suspend(|s:Arc<State>|async move{s.picker.close();s.setup.close();s.policy.suspend().await;s.picker.stop().await;s.setup.stop().await;Ok(())})
    .on_dispose(|s:Arc<State>|async move{s.picker.close();s.setup.close();s.policy.suspend().await;s.picker.stop().await;s.setup.stop().await;s.policy.dispose().await;s.membership.join().await;Ok(())})
    .on_resume(|s:Arc<State>|async move{s.picker.resume();s.setup.resume();s.membership.start(&s.policy).await;s.policy.resume().await;Ok(())})
    .request("external-gateway.status.get",|c:Context<State>,_:Empty|async move{Ok(c.state().policy.status())})
    .request("external-gateway.catalog.page",|c:Context<State>,p:Page|async move{c.state().policy.catalog_page(p.cursor.as_deref(),p.contract.as_deref(),p.offset).await})
    .request("external-gateway.listener.set",|c:Context<State>,p:Enabled|async move{c.state().policy.change(Change::Listener(p.enabled)).await})
    .request("external-gateway.port.set",|c:Context<State>,p:Port|async move{c.state().policy.change(Change::Port(p.port)).await})
    .request("external-gateway.package.set",|c:Context<State>,p:policy::PackageChange|async move{c.state().policy.change(Change::Package(p)).await})
    .request("external-gateway.tool.set",|c:Context<State>,p:policy::ToolChange|async move{c.state().policy.change(Change::Tool(p)).await})
    .request("external-gateway.token.create",|c:Context<State>,p:Label|async move{c.state().policy.change(Change::Create(p.label)).await})
    .request("external-gateway.token.regenerate",|c:Context<State>,p:Id|async move{c.state().policy.change(Change::Regenerate(p.id)).await})
    .request("external-gateway.token.revoke",|c:Context<State>,p:Id|async move{c.state().policy.change(Change::Revoke(p.id)).await})
    .request("external-gateway.disable-and-revoke-all",|c:Context<State>,_:Empty|async move{c.state().policy.change(Change::DisableAll).await})
    .request("external-gateway.setup.preview",|c:Context<State>,p:setup::Preview|async move{c.state().setup.preview(p).await})
    .request("external-gateway.setup.apply",|c:Context<State>,p:setup::Apply|async move{c.state().setup.apply(p).await})
    .request("external-gateway.setup.status",|c:Context<State>,_:Empty|async move{Ok(c.state().setup.status().await)})
    .subscription("external-gateway.setup.choose-skill-folder",|c:Context<State>,_:Empty|async move{c.state().picker.choose(c.state().setup.clone(),c.cancellation().clone())})
    .subscription("external-gateway.setup.watch",|c:Context<State>,_:Empty|async move{
        let (sender,stream)=event_channel(1);let mut status=c.state().setup.receipt_watch();let mut policy=c.state().policy.watch();let cancel=c.cancellation().clone();let setup=c.state().setup.clone();
        tokio::spawn(async move{loop {
            status.borrow_and_update();policy.borrow_and_update();let value=setup.status().await;
            tokio::select!{_=cancel.cancelled()=>break,result=sender.send(value)=>{if result.is_err(){break;}}}
            tokio::select!{_=cancel.cancelled()=>break,_=sender.closed()=>break,result=status.changed()=>{if result.is_err(){break;}},result=policy.changed()=>{if result.is_err(){break;}}}
        }});Ok(stream)
    })
    .subscription("external-gateway.status.watch",|c:Context<State>,_:Empty|async move{
        let (sender,stream)=event_channel(1);let mut status=c.state().policy.watch();let cancel=c.cancellation().clone();
        tokio::spawn(async move{
            loop {
                let value:Value=status.borrow_and_update().clone();
                tokio::select! { _=cancel.cancelled()=>break,result=sender.send(value)=>{if result.is_err(){break;}} }
                tokio::select! { _=cancel.cancelled()=>break,_=sender.closed()=>break,result=status.changed()=>{if result.is_err(){break;}} }
            }
        });Ok(stream)
    })
}
