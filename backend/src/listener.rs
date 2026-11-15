//! One IPv4 socket and a bounded set of connection observers per activation.
use crate::{limits::*, policy::Policy};
use std::{net::Ipv4Addr, sync::Weak, time::Duration};
use tokio::{
    net::TcpListener,
    sync::watch,
    task::{JoinHandle, JoinSet},
};

pub async fn bind(port: u16) -> std::io::Result<TcpListener> {
    TcpListener::bind((Ipv4Addr::LOCALHOST, port)).await
}
pub struct Listener {
    stop: watch::Sender<bool>,
    task: JoinHandle<()>,
}
impl Listener {
    pub fn start(socket: TcpListener, policy: Weak<Policy>) -> Self {
        let (stop, mut cancelled) = watch::channel(false);
        let port = socket.local_addr().expect("bound socket").port();
        let retired = policy.upgrade().expect("listener owner").retired.clone();
        let task = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            loop {
                tokio::select! {
                    biased;
                    _=cancelled.changed()=>break,
                    _=retired.cancelled()=>{connections.abort_all();break;},
                    Some(_)=connections.join_next(),if !connections.is_empty()=>{},
                    accepted=socket.accept()=>{
                        let Ok((stream,_))=accepted else {break};
                        if connections.len()>=MAX_CONNECTIONS {drop(stream);continue;}
                        let Some(policy)=policy.upgrade() else {break};
                        connections.spawn(crate::http::serve(stream,port,policy));
                    }
                }
            }
            drop(socket);
            if tokio::time::timeout(Duration::from_secs(LISTENER_DRAIN_TIMEOUT as u64), async {
                while connections.join_next().await.is_some() {}
            })
            .await
            .is_err()
            {
                connections.abort_all();
                while connections.join_next().await.is_some() {}
            }
        });
        Self { stop, task }
    }
    pub async fn stop(mut self) {
        self.stop.send_replace(true);
        let _ = (&mut self.task).await;
    }
}
impl Drop for Listener {
    fn drop(&mut self) {
        self.stop.send_replace(true);
    }
}
