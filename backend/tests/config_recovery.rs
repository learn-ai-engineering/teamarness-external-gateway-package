// AC13/AC15: real process interruption, never promotion of uncommitted settings.
use http_support::backend_environment;
mod http_support;
use http_support::*;
use serde_json::json;
use std::sync::{Arc, Mutex};
use teamarness_generated_package_backend::config::{FileStore, StorageFault, WriteStage};

struct Interrupt(WriteStage);
impl StorageFault for Interrupt {
    fn before(&self, stage: WriteStage) -> std::io::Result<()> {
        if stage == self.0 {
            // No Rust destructors or FileStore error cleanup run in this child.
            std::process::exit(73);
        }
        Ok(())
    }
}
#[test]
fn interrupted_settings_writer_child() {
    let Some(path) = std::env::var_os("GATEWAY_INTERRUPTED_WRITE_DIRECTORY") else {
        return;
    };
    let stage = match std::env::var("GATEWAY_INTERRUPTED_WRITE_STAGE")
        .unwrap()
        .as_str()
    {
        "write" => WriteStage::Write,
        "sync" => WriteStage::FileSync,
        "rename" => WriteStage::Rename,
        _ => panic!("unknown interruption boundary"),
    };
    let store = FileStore::open(std::path::Path::new(&path))
        .unwrap()
        .with_fault(Arc::new(Interrupt(stage)));
    let mut next = store.read().unwrap();
    next.desired_enabled = false;
    next.tokens.clear();
    next.contracts.clear();
    store.write(&next).unwrap();
    panic!("child must exit before rename");
}

#[tokio::test]
async fn interrupted_settings_save_recovers_committed_listener_and_durable_mutations() {
    for stage in ["write", "sync", "rename"] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().canonicalize().unwrap();
        let provider = Arc::new(Mutex::new(Provider::new()));
        let mut h = start(provider.clone(), &path).await;
        select(&mut h).await;
        let (port, bearer) = enable(&mut h).await;
        h.shutdown().await.unwrap();
        let committed = std::fs::read(path.join("config/state.json")).unwrap();
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "interrupted_settings_writer_child",
                "--nocapture",
            ])
            .env("GATEWAY_INTERRUPTED_WRITE_DIRECTORY", &path)
            .env("GATEWAY_INTERRUPTED_WRITE_STAGE", stage)
            .status()
            .unwrap();
        assert_eq!(child.code(), Some(73));
        assert!(path.join("config/state.tmp").is_file());
        assert_eq!(
            std::fs::read(path.join("config/state.json")).unwrap(),
            committed
        );
        // Opening/reading is not recovery and must not touch a possibly live writer's temp.
        let store = FileStore::open(&path).unwrap();
        assert!(store.read().unwrap().desired_enabled);
        assert!(path.join("config/state.tmp").is_file());
        drop(store);

        let mut h = start(provider.clone(), &path).await;
        let status = h
            .request("external-gateway.status.get", json!({}))
            .await
            .unwrap();
        assert_eq!(
            status["running"], true,
            "saved listener must recover after {stage}"
        );
        assert_eq!(status["storageFailed"], false);
        assert_eq!(status["port"], port);
        assert_eq!(
            std::fs::read(path.join("config/state.json")).unwrap(),
            committed
        );
        assert!(!path.join("config/state.tmp").exists());
        assert_eq!(post(port, &bearer, call(json!({}))).await.0, 200);
        let id = status["tokens"][0]["id"].clone();
        let regenerated = h
            .request("external-gateway.token.regenerate", json!({"id":id}))
            .await
            .unwrap();
        assert_eq!(get(port, &bearer, "/v1").await.0, 401);
        let replacement = regenerated["bearer"].as_str().unwrap();
        assert_eq!(get(port, replacement, "/v1").await.0, 200);
        h.request("external-gateway.token.revoke", json!({"id":id}))
            .await
            .unwrap();
        h.request("external-gateway.disable-and-revoke-all", json!({}))
            .await
            .unwrap();
        h.shutdown().await.unwrap();
        let saved = FileStore::open(&path).unwrap().read().unwrap();
        assert!(!saved.desired_enabled);
        assert!(saved.tokens.is_empty());
        assert!(saved.contracts.is_empty());
        assert_eq!(saved.port, Some(port));
        let mut h = start(provider, &path).await;
        assert_eq!(
            h.request("external-gateway.status.get", json!({}))
                .await
                .unwrap()["running"],
            false
        );
        h.shutdown().await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancelled_observer_cannot_reclaim_a_live_writers_temp() {
    use teamarness_generated_package_backend::{State, policy::Change, tokens::OsEntropy};
    use teamarness_package_sdk::{Context, Empty};
    struct PausedWrite(Arc<Mutex<Option<Arc<Pause>>>>);
    impl StorageFault for PausedWrite {
        fn before(&self, stage: WriteStage) -> std::io::Result<()> {
            if stage == WriteStage::Rename {
                let paused = self.0.lock().unwrap().take();
                if let Some(paused) = paused {
                    paused.entered.notify_one();
                    paused.wait();
                }
            }
            Ok(())
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().canonicalize().unwrap();
    let armed = Arc::new(Mutex::new(None));
    let (send, recv) = tokio::sync::oneshot::channel();
    let send = Arc::new(Mutex::new(Some(send)));
    let router = backend_environment::router_with_storage_fault(
        Arc::new(OsEntropy),
        Some(Arc::new(PausedWrite(armed.clone()))),
    )
    .request("test.capture", move |ctx: Context<State>, _: Empty| {
        let send = send.clone();
        async move {
            send.lock()
                .unwrap()
                .take()
                .unwrap()
                .send(ctx.state().policy.clone())
                .ok();
            Ok(json!({}))
        }
    });
    let mut h = start_router(Arc::new(Mutex::new(Provider::new())), &path, router).await;
    h.request("test.capture", json!({})).await.unwrap();
    let policy = recv.await.unwrap();
    let (barrier, release) = pause();
    *armed.lock().unwrap() = Some(barrier.clone());
    let observer = tokio::spawn({
        let policy = policy.clone();
        async move {
            policy
                .change(Change::Create("retained writer".into()))
                .await
        }
    });
    barrier.entered.notified().await;
    observer.abort();
    let _ = observer.await;
    let temp = path.join("config/state.tmp");
    use std::os::unix::fs::MetadataExt;
    let inode = std::fs::metadata(&temp).unwrap().ino();
    // Neither a second opener/read nor a second policy mutation can discard live state.tmp.
    assert!(
        FileStore::open(&path)
            .unwrap()
            .read()
            .unwrap()
            .tokens
            .is_empty()
    );
    assert_eq!(
        policy
            .change(Change::Create("second writer".into()))
            .await
            .unwrap_err()
            .code(),
        "external_gateway.busy"
    );
    assert_eq!(std::fs::metadata(&temp).unwrap().ino(), inode);
    let mut status = policy.watch();
    drop(release);
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if status.borrow_and_update()["tokens"]
                .as_array()
                .unwrap()
                .len()
                == 1
            {
                break;
            }
            status.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    assert!(!temp.exists());
    assert_eq!(
        FileStore::open(&path).unwrap().read().unwrap().tokens.len(),
        1
    );
    h.shutdown().await.unwrap();
}
