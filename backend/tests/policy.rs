//! Verifies: TEAM.SYS.PKG-R03, R05, R13, R14, R19. Only the Host capability is scripted.
mod membership_support;
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::sync::{Arc, Mutex};
use teamarness_generated_package_backend::{State, router};
use teamarness_package_sdk::testing::{Harness, HarnessBuilder, ScriptedCapability};
use teamarness_package_sdk::{BackendError, Context};

#[derive(Clone)]
struct Provider {
    document: Value,
    provider: String,
    major: u16,
    enabled: bool,
    invocations: usize,
    cleanup_fails: bool,
}
impl Provider {
    fn new() -> Self {
        let request = json!({"type":"object","properties":{},"additionalProperties":false});
        let ports = json!([
            {"name":"runtime.guide","kind":"query","description":"Guide","request":request,"response":{"type":"object","properties":{"markdown":{"type":"string","maxLength":1000}},"required":["markdown"],"additionalProperties":false}},
            {"name":"read.item","kind":"query","description":"Read","request":request,"response":{"$ref":"#/$defs/Result"}},
            {"name":"write.item","kind":"command","description":"Write","request":request,"response":request},
            {"name":"watch.item","kind":"query","description":"Stream","request":request,"event":request},
            {"name":"items.changed","kind":"event","description":"Event","event":request}
        ]);
        Self {
            document: json!({"schemaVersion":1,"$defs":{"Result":{"type":"object","properties":{"value":{"type":"integer"}},"required":["value"],"additionalProperties":false}},"contracts":[{"identity":"dev.test.reads","major":1,"minor":0,"description":"Unknown provider","ports":ports}]}),
            provider: "dev.test.provider".into(),
            major: 1,
            enabled: false,
            invocations: 0,
            cleanup_fails: false,
        }
    }
    fn call(&mut self, method: &str, p: Value) -> Result<Value, BackendError> {
        assert_eq!(p["access"], "person");
        let bytes = serde_json::to_vec(&self.document).unwrap();
        let digest: String = Sha256::digest(&bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        match method {
            "available" => Ok(
                json!({"contracts":[{"contract":"dev.test.reads","major":self.major,"minor":0,"description":"Reads","available":true,"enabled":self.enabled,"enabledMajor":self.enabled.then_some(self.major),"metadataSha256":digest,"provider":{"identity":self.provider,"version":"1.0.0","displayName":"Unknown","description":null},"guideEligibility":"eligible","routeRevision":"a".repeat(64)}]}),
            ),
            "describe" => {
                assert_eq!(p["routeRevision"], "a".repeat(64));
                assert_eq!(p["metadataSha256"], digest);
                Ok(
                    json!({"metadataSha256":digest,"totalBytes":bytes.len(),"offset":0,"data":STANDARD.encode(bytes)}),
                )
            }
            "enable" => {
                assert_eq!(p["routeRevision"], "a".repeat(64));
                self.enabled = true;
                Ok(
                    json!({"access":"person","contract":"dev.test.reads","major":self.major,"enabled":true}),
                )
            }
            "disable" => {
                if self.cleanup_fails {
                    return Err(BackendError::new("port_unavailable", false, json!({})));
                }
                self.enabled = false;
                Ok(json!({"access":"person","contract":p["contract"],"enabled":false}))
            }
            "invoke" => {
                assert_eq!(p["routeRevision"], "a".repeat(64));
                assert_eq!(p["kind"], "query");
                self.invocations += 1;
                if p["port"] == "runtime.guide" {
                    Ok(json!({"markdown":"Unknown provider guide"}))
                } else {
                    Ok(json!({"value":42}))
                }
            }
            _ => panic!("unexpected method"),
        }
    }
}
#[derive(Deserialize)]
struct Read {
    bearer: String,
    port: String,
}
async fn start(provider: Arc<Mutex<Provider>>, directory: &std::path::Path) -> Harness {
    start_router(provider, directory, router()).await
}
async fn start_router(
    provider: Arc<Mutex<Provider>>,
    directory: &std::path::Path,
    r: teamarness_package_sdk::Router<State>,
) -> Harness {
    let script = ScriptedCapability::new().calls(move |m, p| provider.lock().unwrap().call(m, p));
    let r = r.request("test.read", |c: Context<State>, p: Read| async move {
        c.state()
            .policy
            .query(p.bearer, "dev.test.reads".into(), 1, p.port, json!({}))
            .await
    });
    let mut h = HarnessBuilder::new(r.into_backend())
        .data_directory(directory.canonicalize().unwrap())
        .installation(&"b".repeat(64), "Local")
        .capability("teamarness.membership", membership_support::present())
        .capability_version("teamarness.package-ports", script, 3, 0)
        .start()
        .await;
    h.activate().await.unwrap();
    h
}
async fn select(h: &mut Harness) -> Value {
    let tools = h
        .request(
            "external-gateway.catalog.page",
            json!({"contract":"dev.test.reads"}),
        )
        .await
        .unwrap();
    let fingerprint = tools["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["port"] == "read.item")
        .unwrap()["fingerprint"]
        .clone();
    h.request("external-gateway.tool.set",json!({"contract":"dev.test.reads","major":1,"provider":"dev.test.provider","port":"read.item","fingerprint":fingerprint,"enabled":true})).await.unwrap()
}
async fn token(h: &mut Harness, label: &str) -> Value {
    h.request("external-gateway.token.create", json!({"label":label}))
        .await
        .unwrap()
}
#[tokio::test]
async fn shared_scope_exact_reads_guide_and_regeneration_refuse_unselected_operations() {
    let dir = tempfile::tempdir().unwrap();
    let p = Arc::new(Mutex::new(Provider::new()));
    let mut h = start(p.clone(), dir.path()).await;
    let a = token(&mut h, "a").await;
    let b = token(&mut h, "b").await;
    h.request("external-gateway.listener.set", json!({"enabled":true}))
        .await
        .unwrap();
    assert!(
        h.request(
            "test.read",
            json!({"bearer":a["bearer"],"port":"read.item"})
        )
        .await
        .is_err()
    );
    select(&mut h).await;
    for t in [&a, &b] {
        assert_eq!(
            h.request(
                "test.read",
                json!({"bearer":t["bearer"],"port":"read.item"})
            )
            .await
            .unwrap(),
            json!({"value":42})
        );
    }
    h.request(
        "test.read",
        json!({"bearer":a["bearer"],"port":"runtime.guide"}),
    )
    .await
    .unwrap();
    let before = p.lock().unwrap().invocations;
    for port in ["write.item", "watch.item", "items.changed", "read.unknown"] {
        assert!(
            h.request("test.read", json!({"bearer":a["bearer"],"port":port}))
                .await
                .is_err()
        );
    }
    assert_eq!(p.lock().unwrap().invocations, before);
    let new = h
        .request("external-gateway.token.regenerate", json!({"id":a["id"]}))
        .await
        .unwrap();
    assert!(
        h.request(
            "test.read",
            json!({"bearer":a["bearer"],"port":"read.item"})
        )
        .await
        .is_err()
    );
    h.request(
        "test.read",
        json!({"bearer":new["bearer"],"port":"read.item"}),
    )
    .await
    .unwrap();
    h.request("external-gateway.token.revoke", json!({"id":b["id"]}))
        .await
        .unwrap();
    assert!(
        h.request(
            "test.read",
            json!({"bearer":b["bearer"],"port":"read.item"})
        )
        .await
        .is_err()
    );
    p.lock().unwrap().cleanup_fails = true;
    let result = h
        .request(
            "external-gateway.package.set",
            json!({"provider":"dev.test.provider","enabled":false,"contracts":[]}),
        )
        .await
        .unwrap();
    assert_eq!(result["warning"], "host_cleanup_failed");
    assert!(
        h.request(
            "test.read",
            json!({"bearer":new["bearer"],"port":"read.item"})
        )
        .await
        .is_err()
    );
    h.shutdown().await.unwrap();
    let mut h = start(p, dir.path()).await;
    assert!(
        h.request(
            "test.read",
            json!({"bearer":new["bearer"],"port":"read.item"})
        )
        .await
        .is_err()
    );
    h.shutdown().await.unwrap();
}
#[tokio::test]
async fn changed_schema_provider_major_and_missing_host_selection_need_review() {
    let dir = tempfile::tempdir().unwrap();
    let p = Arc::new(Mutex::new(Provider::new()));
    let mut h = start(p.clone(), dir.path()).await;
    let a = token(&mut h, "a").await;
    h.request("external-gateway.listener.set", json!({"enabled":true}))
        .await
        .unwrap();
    select(&mut h).await;
    let read = json!({"bearer":a["bearer"],"port":"read.item"});
    p.lock().unwrap().document["contracts"][0]["description"] = "Changed description".into();
    p.lock().unwrap().document["$defs"]["Result"]["description"] = "Description only".into();
    h.request("test.read", read.clone()).await.unwrap();
    let newport = p.lock().unwrap().document["contracts"][0]["ports"][1].clone();
    {
        let mut p = p.lock().unwrap();
        let mut port = newport;
        port["name"] = "read.added".into();
        p.document["contracts"][0]["ports"]
            .as_array_mut()
            .unwrap()
            .push(port);
    }
    h.request("test.read", read.clone()).await.unwrap();
    assert!(
        h.request(
            "test.read",
            json!({"bearer":a["bearer"],"port":"read.added"})
        )
        .await
        .is_err()
    );
    p.lock().unwrap().document["$defs"]["Result"]["properties"]["value"]["maximum"] = 100.into();
    assert!(h.request("test.read", read.clone()).await.is_err());
    select(&mut h).await;
    p.lock().unwrap().enabled = false;
    assert!(h.request("test.read", read.clone()).await.is_err());
    assert!(!p.lock().unwrap().enabled);
    p.lock().unwrap().enabled = true;
    p.lock().unwrap().provider = "dev.test.replacement".into();
    assert!(h.request("test.read", read.clone()).await.is_err());
    p.lock().unwrap().provider = "dev.teamarness.conversations".into();
    assert!(h.request("test.read", read.clone()).await.is_err());
    p.lock().unwrap().provider = "dev.test.provider".into();
    p.lock().unwrap().major = 2;
    p.lock().unwrap().document["contracts"][0]["major"] = 2.into();
    assert!(h.request("test.read", read).await.is_err());
    h.shutdown().await.unwrap();
}
#[tokio::test]
async fn property_named_description_is_schema_not_an_annotation() {
    let dir = tempfile::tempdir().unwrap();
    let p = Arc::new(Mutex::new(Provider::new()));
    p.lock().unwrap().document["$defs"]["Result"]["properties"]["description"] =
        json!({"type":"string","maxLength":10});
    let mut h = start(p.clone(), dir.path()).await;
    let a = token(&mut h, "a").await;
    h.request("external-gateway.listener.set", json!({"enabled":true}))
        .await
        .unwrap();
    select(&mut h).await;
    p.lock().unwrap().document["$defs"]["Result"]["properties"]["description"]["maxLength"] =
        20.into();
    assert!(
        h.request(
            "test.read",
            json!({"bearer":a["bearer"],"port":"read.item"})
        )
        .await
        .is_err()
    );
    assert_eq!(p.lock().unwrap().invocations, 0);
    h.shutdown().await.unwrap();
}

#[tokio::test]
async fn failed_enable_never_publishes_tool_policy_even_when_host_selection_committed() {
    use teamarness_generated_package_backend::{
        config::{StorageFault, WriteStage},
        router_with_storage_fault,
        tokens::OsEntropy,
    };
    struct Once(Mutex<Option<WriteStage>>);
    impl StorageFault for Once {
        fn before(&self, stage: WriteStage) -> std::io::Result<()> {
            let mut armed = self.0.lock().unwrap();
            if *armed == Some(stage) {
                *armed = None;
                Err(std::io::Error::other("injected storage failure"))
            } else {
                Ok(())
            }
        }
    }
    for stage in [
        WriteStage::Write,
        WriteStage::FileSync,
        WriteStage::Rename,
        WriteStage::DirectorySync,
    ] {
        let fault = Arc::new(Once(Mutex::new(None)));
        let dir = tempfile::tempdir().unwrap();
        let p = Arc::new(Mutex::new(Provider::new()));
        let mut h = start_router(
            p.clone(),
            dir.path(),
            router_with_storage_fault(Arc::new(OsEntropy), Some(fault.clone())),
        )
        .await;
        let a = token(&mut h, "a").await;
        h.request("external-gateway.listener.set", json!({"enabled":true}))
            .await
            .unwrap();
        let tools = h
            .request(
                "external-gateway.catalog.page",
                json!({"contract":"dev.test.reads"}),
            )
            .await
            .unwrap();
        let fingerprint = tools["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["port"] == "read.item")
            .unwrap()["fingerprint"]
            .clone();
        *fault.0.lock().unwrap() = Some(stage);
        assert_eq!(h.request("external-gateway.tool.set",json!({"contract":"dev.test.reads","major":1,"provider":"dev.test.provider","port":"read.item","fingerprint":fingerprint,"enabled":true})).await.unwrap_err().code(),"external_gateway.storage_failed");
        assert!(p.lock().unwrap().enabled);
        assert!(
            h.request(
                "test.read",
                json!({"bearer":a["bearer"],"port":"read.item"})
            )
            .await
            .is_err()
        );
        h.request("external-gateway.port.set", json!({"port":43210}))
            .await
            .unwrap();
        assert!(
            h.request(
                "test.read",
                json!({"bearer":a["bearer"],"port":"read.item"})
            )
            .await
            .is_err()
        );
        assert_eq!(p.lock().unwrap().invocations, 0);
        h.shutdown().await.unwrap();
    }
}
#[tokio::test]
async fn package_enable_selects_no_tools_and_removed_tool_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let p = Arc::new(Mutex::new(Provider::new()));
    let mut h = start(p.clone(), dir.path()).await;
    let a = token(&mut h, "a").await;
    h.request("external-gateway.listener.set", json!({"enabled":true}))
        .await
        .unwrap();
    h.request("external-gateway.package.set",json!({"provider":"dev.test.provider","enabled":true,"contracts":[{"contract":"dev.test.reads","major":1}]})).await.unwrap();
    assert!(
        h.request(
            "test.read",
            json!({"bearer":a["bearer"],"port":"read.item"})
        )
        .await
        .is_err()
    );
    select(&mut h).await;
    p.lock().unwrap().document["contracts"][0]["ports"]
        .as_array_mut()
        .unwrap()
        .retain(|p| p["name"] != "read.item");
    assert!(
        h.request(
            "test.read",
            json!({"bearer":a["bearer"],"port":"read.item"})
        )
        .await
        .is_err()
    );
    assert_eq!(p.lock().unwrap().invocations, 0);
    h.shutdown().await.unwrap();
}
