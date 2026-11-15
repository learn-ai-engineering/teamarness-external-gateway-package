//! Protocol budgets. Timeout and interval values are seconds.
pub const MAX_CONNECTIONS: usize = 32;
pub const MAX_HEADER_BYTES: usize = 16384;
pub const MAX_HEADERS: usize = 64;
pub const MAX_TARGET_BYTES: usize = 4096;
pub const REQUEST_READ_TIMEOUT: usize = 5;
pub const RESPONSE_WRITE_TIMEOUT: usize = 5;
pub const REQUESTS_PER_SECOND: usize = 8;
pub const REQUEST_BURST: usize = 16;
/// Outstanding gateway SDK invocations, not a global Host execution limit.
pub const MAX_HOST_CALLS: usize = 12;
pub const TOOL_WAIT_TIMEOUT: usize = 25;
pub const PORT_VALUE_BYTES: usize = 262144;
pub const ENVELOPE_ALLOWANCE_BYTES: usize = 4096;
pub const METADATA_BYTES: usize = 1048576;
pub const METADATA_CHUNK_BYTES: usize = 65536;
pub const SUMMARY_ROWS: usize = 32;
pub const SUMMARY_BYTES: usize = 32768;
pub const METADATA_CACHE_BYTES: usize = 16777216;
pub const MAX_CATALOG_RESTARTS: usize = 1;
pub const CATALOG_POLL_INTERVAL: usize = 5;
pub const CATALOG_READ_TIMEOUT: usize = 5;
pub const LISTENER_DRAIN_TIMEOUT: usize = 5;

use std::{
    future::Future,
    sync::{Arc, Mutex},
    time::Duration,
};
use teamarness_package_sdk::BackendError;
use tokio::{
    sync::{Semaphore, oneshot},
    task::JoinSet,
    time::Instant,
};

/// Owns bounded local futures independently of their HTTP observers. Every SDK terminal releases.
pub struct Calls {
    slots: Arc<Semaphore>,
    tasks: Mutex<JoinSet<()>>,
}
impl Default for Calls {
    fn default() -> Self {
        Self {
            slots: Arc::new(Semaphore::new(MAX_HOST_CALLS)),
            tasks: Mutex::new(JoinSet::new()),
        }
    }
}
impl Calls {
    pub async fn run<T: Send + 'static>(
        &self,
        work: impl Future<Output = Result<T, BackendError>> + Send + 'static,
    ) -> Result<T, BackendError> {
        let permit = self
            .slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| crate::error("busy"))?;
        let (tx, rx) = oneshot::channel();
        {
            let mut tasks = self.tasks.lock().expect("call ownership");
            if self.slots.is_closed() {
                return Err(crate::error("unavailable"));
            }
            while tasks.try_join_next().is_some() {}
            tasks.spawn(async move {
                let result = work.await;
                drop(permit);
                let _ = tx.send(result);
            });
        }
        rx.await.map_err(|_| crate::error("unavailable"))?
    }
    pub fn retire(&self) {
        self.slots.close();
        self.tasks.lock().expect("call ownership").abort_all();
    }
    pub async fn dispose(&self) {
        self.retire();
        let mut tasks = std::mem::take(&mut *self.tasks.lock().expect("call ownership"));
        while tasks.join_next().await.is_some() {}
    }
}
/// One Workspace-wide bucket. Listener restarts do not reset it.
pub struct Rate {
    tokens: f64,
    at: Instant,
}
impl Default for Rate {
    fn default() -> Self {
        Self {
            tokens: REQUEST_BURST as f64,
            at: Instant::now(),
        }
    }
}
impl Rate {
    pub fn charge(&mut self) -> bool {
        let now = Instant::now();
        self.tokens = (self.tokens
            + now.duration_since(self.at).as_secs_f64() * REQUESTS_PER_SECOND as f64)
            .min(REQUEST_BURST as f64);
        self.at = now;
        if self.tokens < 1.0 {
            false
        } else {
            self.tokens -= 1.0;
            true
        }
    }
}
pub async fn observe<T>(
    work: impl Future<Output = Result<T, BackendError>>,
) -> Result<T, BackendError> {
    tokio::time::timeout(Duration::from_secs(TOOL_WAIT_TIMEOUT as u64), work)
        .await
        .map_err(|_| crate::error("timeout"))?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test(start_paused = true)]
    async fn workspace_bucket_has_burst_sixteen_and_exact_eight_per_second() {
        let mut rate = Rate::default();
        for _ in 0..16 {
            assert!(rate.charge());
        }
        assert!(!rate.charge());
        tokio::time::advance(Duration::from_millis(124)).await;
        assert!(!rate.charge());
        tokio::time::advance(Duration::from_millis(1)).await;
        assert!(rate.charge());
        assert!(!rate.charge());
        tokio::time::advance(Duration::from_secs(1)).await;
        for _ in 0..8 {
            assert!(rate.charge());
        }
        assert!(!rate.charge());
    }
    #[tokio::test(start_paused = true)]
    async fn outer_observer_ceiling_does_not_retire_pending_local_work() {
        let calls = Arc::new(Calls::default());
        let (send, recv) = oneshot::channel();
        let pending = tokio::spawn({
            let calls = calls.clone();
            async move {
                observe(
                    calls.run(async move { recv.await.map_err(|_| crate::error("unavailable")) }),
                )
                .await
            }
        });
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(24)).await;
        assert!(!pending.is_finished());
        tokio::time::advance(Duration::from_secs(1)).await;
        assert_eq!(
            pending.await.unwrap().unwrap_err().code(),
            "external_gateway.timeout"
        );
        assert_eq!(calls.slots.available_permits(), 11);
        send.send(42).unwrap();
        tokio::task::yield_now().await;
        assert_eq!(calls.slots.available_permits(), 12);
        calls.dispose().await;
    }
    #[tokio::test]
    async fn twelve_owned_futures_refuse_thirteenth_and_all_terminals_return_capacity() {
        let calls = Arc::new(Calls::default());
        let mut releases = vec![];
        let mut observers = vec![];
        for _ in 0..12 {
            let (send, recv) = oneshot::channel();
            releases.push(send);
            let (entered, ready) = oneshot::channel();
            let calls = calls.clone();
            observers.push(tokio::spawn(async move {
                calls
                    .run(async move {
                        let _ = entered.send(());
                        recv.await
                            .unwrap_or_else(|_| Err(crate::error("unavailable")))
                    })
                    .await
            }));
            ready.await.unwrap();
        }
        assert_eq!(
            calls.run(async { Ok(()) }).await.unwrap_err().code(),
            "external_gateway.busy"
        );
        observers.remove(0).abort();
        assert_eq!(calls.slots.available_permits(), 0);
        for (i, send) in releases.into_iter().enumerate() {
            let result = match i % 4 {
                0 => Ok(()),
                1 => Err(crate::error("timeout")),
                2 => Err(BackendError::new("overloaded", true, serde_json::json!({}))),
                _ => Err(BackendError::new(
                    "host_unavailable",
                    true,
                    serde_json::json!({}),
                )),
            };
            send.send(result).unwrap();
        }
        for observer in observers {
            let _ = observer.await.unwrap();
        }
        assert_eq!(calls.slots.available_permits(), 12);
        assert!(calls.run(async { Ok(()) }).await.is_ok());
        calls.dispose().await;
    }
}
