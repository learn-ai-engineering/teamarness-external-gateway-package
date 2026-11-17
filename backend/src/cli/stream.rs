//! One subscription owner: no producer queue, reconnect or detached network task.
use super::{
    args::Observation,
    output::{self, Failure},
    transport,
};
use crate::{client_store::Connection, limits::*};
use serde_json::{json, value::RawValue};
use std::{collections::BTreeMap, future::Future, time::Duration};
use tokio::{
    io::{AsyncWrite, AsyncWriteExt},
    time::{Instant, sleep_until},
};
const LIVENESS: Duration = Duration::from_secs(45);
const OUTPUT_WAIT: Duration = Duration::from_secs(5);

trait Input {
    fn line(&mut self) -> impl Future<Output = Result<Vec<u8>, Failure>>;
}
impl Input for transport::StreamResponse {
    async fn line(&mut self) -> Result<Vec<u8>, Failure> {
        self.line().await
    }
}
#[derive(Debug, PartialEq)]
enum Frame {
    Ready,
    Value,
    Heartbeat,
    End(u8),
}
fn frame(bytes: &[u8], selected: &Observation, ready: bool) -> Option<Frame> {
    if bytes.len() > PORT_VALUE_BYTES + ENVELOPE_ALLOWANCE_BYTES
        || !bytes.ends_with(b"\n")
        || !output::valid_json(bytes)
    {
        return None;
    }
    let fields: BTreeMap<String, &RawValue> = serde_json::from_slice(bytes).ok()?;
    let string = |key: &str| {
        fields
            .get(key)
            .and_then(|r| serde_json::from_str::<String>(r.get()).ok())
    };
    match string("type").as_deref()? {
        "ready"
            if !ready
                && fields.len() == 6
                && string("contract").as_deref() == Some(selected.contract.as_str())
                && fields
                    .get("major")
                    .and_then(|r| serde_json::from_str::<u16>(r.get()).ok())
                    == Some(selected.major)
                && string("port").as_deref() == Some(selected.port.as_str())
                && matches!(string("kind").as_deref(), Some("event" | "query"))
                && string("delivery").as_deref() == Some("stream") =>
        {
            Some(Frame::Ready)
        }
        "value"
            if ready
                && fields.len() == 2
                && fields
                    .get("value")
                    .is_some_and(|v| v.get().len() <= PORT_VALUE_BYTES) =>
        {
            Some(Frame::Value)
        }
        "heartbeat" if ready && fields.len() == 1 => Some(Frame::Heartbeat),
        "end" if ready && string("reason").as_deref() == Some("completed") && fields.len() == 2 => {
            Some(Frame::End(0))
        }
        "end" if ready && string("reason").as_deref() == Some("error") && fields.len() == 3 => {
            let error = fields.get("error")?;
            output::upstream_error(format!("{{\"error\":{}}}", error.get()).as_bytes())
                .then_some(Frame::End(1))
        }
        _ => None,
    }
}
fn local_end(reason: &str) -> Vec<u8> {
    format!("{{\"type\":\"client_end\",\"reason\":\"{reason}\"}}\n").into_bytes()
}
fn interruption(ready: bool) -> Vec<u8> {
    let mut failure = Failure::transport(false);
    if ready {
        failure.code = "cli.stream_interrupted";
    }
    let mut bytes = if ready {
        serde_json::to_vec(&json!({"type":"client_error","error":serde_json::from_slice::<serde_json::Value>(&failure.bytes()).unwrap()["error"]})).unwrap()
    } else {
        failure.bytes()
    };
    bytes.push(b'\n');
    bytes
}
async fn drain<W: AsyncWrite + Unpin, C: Future<Output = ()> + Unpin>(
    out: &mut W,
    line: &[u8],
    cancel: &mut C,
    cancelled: &mut bool,
) -> bool {
    let write = tokio::time::timeout(OUTPUT_WAIT, out.write_all(line));
    tokio::pin!(write);
    if !*cancelled {
        tokio::select! {biased;
            _ = &mut *cancel => *cancelled = true,
            result = &mut write => return matches!(result, Ok(Ok(()))),
        }
    }
    // Finish an already-started line, never append a terminal to partial JSON.
    matches!(write.await, Ok(Ok(())))
}
async fn terminal<W: AsyncWrite + Unpin>(out: &mut W, bytes: &[u8], exit: u8) -> u8 {
    if matches!(
        tokio::time::timeout(OUTPUT_WAIT, out.write_all(bytes)).await,
        Ok(Ok(()))
    ) {
        exit
    } else {
        1
    }
}
/// The same cancellation future owns opening, ready, output and all terminal decisions.
pub async fn run<W: AsyncWrite + Unpin, C: Future<Output = ()> + Unpin>(
    connection: &Connection,
    body: Vec<u8>,
    selected: &Observation,
    out: &mut W,
    cancel: &mut C,
) -> u8 {
    let opening_deadline = Instant::now() + Duration::from_secs(transport::RESPONSE_SECONDS);
    let opening = async {
        let mut input = transport::open_stream(connection.port, &connection.bearer, body).await?;
        if input.status != 200 {
            return Ok::<_, Failure>(Err(input.refusal().await?));
        }
        Ok(Ok(input))
    };
    let mut input = tokio::select! {biased;
        _ = &mut *cancel => return cancelled_before_ready(out).await,
        _ = sleep_until(opening_deadline) => return terminal(out, &interruption(false), 1).await,
        result = opening => match result {
            Ok(Ok(input)) => input,
            Ok(Err(mut bytes)) => { bytes.push(b'\n'); return terminal(out, &bytes, 1).await; },
            Err(_) => return terminal(out, &interruption(false), 1).await,
        },
    };
    observe(&mut input, selected, out, cancel, opening_deadline).await
}
async fn cancelled_before_ready<W: AsyncWrite + Unpin>(out: &mut W) -> u8 {
    let mut failure = Failure::transport(false);
    failure.code = "cli.cancelled";
    let mut bytes = failure.bytes();
    bytes.push(b'\n');
    terminal(out, &bytes, 130).await
}
async fn observe<I: Input, W: AsyncWrite + Unpin, C: Future<Output = ()> + Unpin>(
    input: &mut I,
    selected: &Observation,
    out: &mut W,
    cancel: &mut C,
    opening_deadline: Instant,
) -> u8 {
    let (mut ready, mut cancelled, mut count) = (false, false, 0u32);
    let mut duration_deadline = None;
    let mut liveness = opening_deadline;
    loop {
        // An accepted terminal is delivered below before returning here. Otherwise
        // cancellation precedes duration, which precedes accepting another frame.
        if cancelled {
            return terminal(out, &local_end("cancelled"), 130).await;
        }
        let duration = async {
            match duration_deadline {
                Some(deadline) => sleep_until(deadline).await,
                None => std::future::pending().await,
            }
        };
        let bytes = tokio::select! {biased;
            _ = &mut *cancel => {
                return if ready { terminal(out, &local_end("cancelled"), 130).await } else { cancelled_before_ready(out).await };
            },
            _ = duration => return terminal(out, &local_end("duration_limit"), 0).await,
            _ = sleep_until(liveness) => return terminal(out, &interruption(ready), 1).await,
            result = input.line() => match result {
                Ok(bytes) => bytes,
                Err(_) => return terminal(out, &interruption(ready), 1).await,
            },
        };
        let Some(frame) = frame(&bytes, selected, ready) else {
            return terminal(out, &interruption(ready), 1).await;
        };
        match frame {
            Frame::Ready => {
                ready = true;
                duration_deadline = match selected.duration {
                    Some(d) => match Instant::now().checked_add(d) {
                        Some(t) => Some(t),
                        None => return terminal(out, &interruption(true), 1).await,
                    },
                    None => None,
                };
            }
            // No overflow even in unlimited mode; count matters only with a limit.
            Frame::Value if selected.max_events.is_some() => count += 1,
            _ => {}
        }
        if !drain(out, &bytes, cancel, &mut cancelled).await {
            return 1;
        }
        if let Frame::End(exit) = frame {
            return exit;
        }
        if matches!(frame, Frame::Value) && selected.max_events == Some(count) {
            return terminal(out, &local_end("event_limit"), 0).await;
        }
        // Local output backpressure suspends no-valid-frame liveness, not duration.
        liveness = Instant::now() + LIVENESS;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, DuplexStream};
    const READY: &[u8] = b"{\"type\":\"ready\",\"contract\":\"dev.new.provider\",\"major\":7,\"port\":\"changes\",\"kind\":\"event\",\"delivery\":\"stream\"}\n";
    const VALUE: &[u8] = b"{\"type\":\"value\",\"value\":null}\n";
    const HEARTBEAT: &[u8] = b"{\"type\":\"heartbeat\"}\n";
    const END: &[u8] = b"{\"type\":\"end\",\"reason\":\"completed\"}\n";
    struct Reader {
        socket: DuplexStream,
        line: Vec<u8>,
    }
    impl Input for Reader {
        async fn line(&mut self) -> Result<Vec<u8>, Failure> {
            loop {
                let mut byte = [0];
                self.socket
                    .read_exact(&mut byte)
                    .await
                    .map_err(|_| Failure::transport(false))?;
                self.line.push(byte[0]);
                if byte[0] == b'\n' {
                    return Ok(std::mem::take(&mut self.line));
                }
            }
        }
    }
    fn selected(duration: Option<Duration>, max_events: Option<u32>) -> Observation {
        Observation {
            contract: "dev.new.provider".into(),
            major: 7,
            port: "changes".into(),
            duration,
            max_events,
        }
    }
    fn reader(socket: DuplexStream) -> Reader {
        Reader {
            socket,
            line: vec![],
        }
    }
    async fn settle() {
        for _ in 0..20 {
            tokio::task::yield_now().await;
        }
    }
    #[tokio::test(start_paused = true)]
    async fn duration_starts_at_valid_ready_and_precedes_next_frame() {
        let (mut server, socket) = tokio::io::duplex(4096);
        let task = tokio::spawn(async move {
            let mut bytes = vec![];
            let exit = observe(
                &mut reader(socket),
                &selected(Some(Duration::from_secs(2)), None),
                &mut bytes,
                &mut std::future::pending(),
                Instant::now() + Duration::from_secs(35),
            )
            .await;
            (exit, bytes)
        });
        settle().await;
        tokio::time::advance(Duration::from_secs(30)).await;
        assert!(!task.is_finished());
        server.write_all(READY).await.unwrap();
        settle().await;
        tokio::time::advance(Duration::from_secs(1)).await;
        assert!(!task.is_finished());
        tokio::time::advance(Duration::from_secs(1)).await;
        server.write_all(VALUE).await.unwrap();
        let (exit, bytes) = task.await.unwrap();
        assert_eq!(exit, 0);
        assert_eq!(bytes, [READY, &local_end("duration_limit")].concat());
    }
    #[tokio::test(start_paused = true)]
    async fn continuous_sparse_heartbeats_have_no_thirty_second_or_lifetime_cap() {
        let (mut server, socket) = tokio::io::duplex(4096);
        let task = tokio::spawn(async move {
            let mut bytes = vec![];
            let exit = observe(
                &mut reader(socket),
                &selected(None, Some(1)),
                &mut bytes,
                &mut std::future::pending(),
                Instant::now() + Duration::from_secs(35),
            )
            .await;
            (exit, bytes)
        });
        server.write_all(READY).await.unwrap();
        settle().await;
        for _ in 0..8 {
            tokio::time::advance(Duration::from_secs(15)).await;
            server.write_all(HEARTBEAT).await.unwrap();
            settle().await;
            assert!(!task.is_finished());
        }
        server.write_all(VALUE).await.unwrap();
        let (exit, bytes) = task.await.unwrap();
        assert_eq!(exit, 0);
        assert_eq!(
            bytes,
            [
                READY,
                &HEARTBEAT.repeat(8),
                VALUE,
                &local_end("event_limit")
            ]
            .concat()
        );
    }
    #[tokio::test(start_paused = true)]
    async fn cancellation_precedes_expired_duration_and_unaccepted_end() {
        let (mut server, socket) = tokio::io::duplex(4096);
        let (cancel, cancelled) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let cancel = async {
                let _ = cancelled.await;
            };
            tokio::pin!(cancel);
            let mut bytes = vec![];
            let exit = observe(
                &mut reader(socket),
                &selected(Some(Duration::from_secs(1)), None),
                &mut bytes,
                &mut cancel,
                Instant::now() + Duration::from_secs(35),
            )
            .await;
            (exit, bytes)
        });
        server.write_all(READY).await.unwrap();
        settle().await;
        cancel.send(()).unwrap();
        server.write_all(END).await.unwrap();
        tokio::time::advance(Duration::from_secs(1)).await;
        let (exit, bytes) = task.await.unwrap();
        assert_eq!(exit, 130);
        assert_eq!(bytes, [READY, &local_end("cancelled")].concat());
    }
    #[tokio::test(start_paused = true)]
    async fn accepted_end_wins_cancel_and_duration_while_output_drains() {
        let (mut server, socket) = tokio::io::duplex(4096);
        let (mut output, mut consumer) = tokio::io::duplex(READY.len());
        let (cancel, cancelled) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let cancel = async {
                let _ = cancelled.await;
            };
            tokio::pin!(cancel);
            observe(
                &mut reader(socket),
                &selected(Some(Duration::from_secs(1)), None),
                &mut output,
                &mut cancel,
                Instant::now() + Duration::from_secs(35),
            )
            .await
        });
        server.write_all(&[READY, END].concat()).await.unwrap();
        settle().await;
        // Ready fills the pipe, then the accepted gateway end owns a pending write.
        cancel.send(()).unwrap();
        tokio::time::advance(Duration::from_secs(1)).await;
        let mut bytes = vec![];
        consumer.read_to_end(&mut bytes).await.unwrap();
        assert_eq!(task.await.unwrap(), 0);
        assert_eq!(bytes, [READY, END].concat());
    }
    #[tokio::test(start_paused = true)]
    async fn count_reaching_value_drains_then_wins_cancel_and_does_not_accept_end() {
        let (mut server, socket) = tokio::io::duplex(4096);
        let (mut output, mut consumer) = tokio::io::duplex(READY.len());
        let (cancel, cancelled) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let cancel = async {
                let _ = cancelled.await;
            };
            tokio::pin!(cancel);
            observe(
                &mut reader(socket),
                &selected(None, Some(1)),
                &mut output,
                &mut cancel,
                Instant::now() + Duration::from_secs(35),
            )
            .await
        });
        server
            .write_all(&[READY, VALUE, END].concat())
            .await
            .unwrap();
        settle().await;
        cancel.send(()).unwrap();
        settle().await;
        let mut bytes = vec![];
        consumer.read_to_end(&mut bytes).await.unwrap();
        assert_eq!(task.await.unwrap(), 0);
        assert_eq!(bytes, [READY, VALUE, &local_end("event_limit")].concat());
    }
    #[tokio::test(start_paused = true)]
    async fn liveness_is_45_seconds_and_partial_bytes_do_not_refresh_it() {
        let (mut server, socket) = tokio::io::duplex(4096);
        let task = tokio::spawn(async move {
            let mut bytes = vec![];
            let exit = observe(
                &mut reader(socket),
                &selected(None, None),
                &mut bytes,
                &mut std::future::pending(),
                Instant::now() + Duration::from_secs(35),
            )
            .await;
            (exit, bytes)
        });
        server.write_all(READY).await.unwrap();
        settle().await;
        tokio::time::advance(Duration::from_secs(44)).await;
        server.write_all(b"{").await.unwrap();
        settle().await;
        assert!(!task.is_finished());
        tokio::time::advance(Duration::from_secs(1)).await;
        let (exit, bytes) = task.await.unwrap();
        assert_eq!(exit, 1);
        assert_eq!(bytes, [READY, &interruption(true)].concat());
    }
    #[tokio::test(start_paused = true)]
    async fn pending_write_has_exact_five_second_bound_and_failure_beats_chosen_terminal() {
        for ending in [END, VALUE] {
            let (mut server, socket) = tokio::io::duplex(4096);
            let (mut output, _consumer) = tokio::io::duplex(READY.len());
            let task = tokio::spawn(async move {
                observe(
                    &mut reader(socket),
                    &selected(None, Some(1)),
                    &mut output,
                    &mut std::future::pending(),
                    Instant::now() + Duration::from_secs(35),
                )
                .await
            });
            server.write_all(&[READY, ending].concat()).await.unwrap();
            settle().await;
            tokio::time::advance(Duration::from_millis(4999)).await;
            assert!(!task.is_finished());
            tokio::time::advance(Duration::from_millis(1)).await;
            assert_eq!(task.await.unwrap(), 1);
        }
    }
    #[tokio::test(start_paused = true)]
    async fn liveness_is_suspended_while_local_output_backpressure_owns_progress() {
        let (mut server, socket) = tokio::io::duplex(4096);
        let (mut output, mut consumer) = tokio::io::duplex(READY.len());
        let task = tokio::spawn(async move {
            observe(
                &mut reader(socket),
                &selected(None, None),
                &mut output,
                &mut std::future::pending(),
                Instant::now() + Duration::from_secs(35),
            )
            .await
        });
        server.write_all(READY).await.unwrap();
        settle().await;
        tokio::time::advance(Duration::from_secs(44)).await;
        server.write_all(VALUE).await.unwrap();
        settle().await;
        tokio::time::advance(Duration::from_secs(4)).await;
        let mut ready = vec![0; READY.len()];
        consumer.read_exact(&mut ready).await.unwrap();
        settle().await;
        assert!(!task.is_finished());
        let mut value = vec![0; VALUE.len()];
        consumer.read_exact(&mut value).await.unwrap();
        tokio::time::advance(Duration::from_secs(44)).await;
        assert!(!task.is_finished());
        server.write_all(END).await.unwrap();
        settle().await;
        let mut end = vec![0; END.len()];
        consumer.read_exact(&mut end).await.unwrap();
        assert_eq!(task.await.unwrap(), 0);
        assert_eq!(end, END);
    }
    #[tokio::test(start_paused = true)]
    async fn opening_deadline_and_pre_ready_cancel_use_ordinary_json() {
        for cancel_now in [false, true] {
            let (_server, socket) = tokio::io::duplex(4096);
            let mut out = vec![];
            let cancel = async move {
                if !cancel_now {
                    std::future::pending::<()>().await;
                }
            };
            tokio::pin!(cancel);
            let started = Instant::now();
            let exit = observe(
                &mut reader(socket),
                &selected(None, None),
                &mut out,
                &mut cancel,
                Instant::now() + Duration::from_secs(35),
            )
            .await;
            assert_eq!(exit, if cancel_now { 130 } else { 1 });
            assert_eq!(
                Instant::now() - started,
                Duration::from_secs(if cancel_now { 0 } else { 35 })
            );
            let value: serde_json::Value = serde_json::from_slice(&out).unwrap();
            assert!(value.get("type").is_none());
            assert_eq!(value["error"]["source"], "cli");
        }
    }
    #[test]
    fn ready_shape_selection_order_and_raw_value_budget_are_validated() {
        let selection = selected(None, None);
        assert_eq!(frame(READY, &selection, false), Some(Frame::Ready));
        assert_eq!(frame(READY, &selection, true), None);
        for (from, to) in [
            ("dev.new.provider", "dev.other.provider"),
            ("7", "8"),
            ("changes", "wrong"),
            ("event", "command"),
            ("stream", "response"),
        ] {
            let bad = String::from_utf8_lossy(READY).replace(from, to);
            assert_eq!(frame(bad.as_bytes(), &selection, false), None);
        }
        for bytes in [VALUE, HEARTBEAT, END] {
            assert_eq!(frame(bytes, &selection, false), None);
        }
        for json in [
            "null",
            "true",
            "false",
            "123456789012345678901234567890",
            "1.2300e+04",
            "\"text\"",
            "[]",
            "{}",
        ] {
            let line = format!("{{\"type\":\"value\",\"value\":{json}}}\n");
            assert_eq!(frame(line.as_bytes(), &selection, true), Some(Frame::Value));
        }
        let value = format!("\"{}\"", "x".repeat(PORT_VALUE_BYTES - 2));
        assert_eq!(
            frame(
                format!("{{\"type\":\"value\",\"value\":{value}}}\n").as_bytes(),
                &selection,
                true
            ),
            Some(Frame::Value)
        );
        assert_eq!(
            frame(
                format!(
                    "{{\"type\":\"value\",\"value\":\"{}\"}}\n",
                    "x".repeat(PORT_VALUE_BYTES - 1)
                )
                .as_bytes(),
                &selection,
                true
            ),
            None
        );
        for json in [
            "{\"type\":\"heartbeat\",\"extra\":0}\n",
            "{\"type\":\"end\",\"reason\":\"unknown\"}\n",
            "{\"type\":\"value\",\"value\":{\"a\":0,\"a\":1}}\n",
        ] {
            assert_eq!(frame(json.as_bytes(), &selection, true), None);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn duration_expires_during_write_but_waits_for_accepted_value_to_drain() {
        let (mut server, socket) = tokio::io::duplex(4096);
        let (mut output, mut consumer) = tokio::io::duplex(READY.len());
        let task = tokio::spawn(async move {
            observe(
                &mut reader(socket),
                &selected(Some(Duration::from_secs(1)), None),
                &mut output,
                &mut std::future::pending(),
                Instant::now() + Duration::from_secs(35),
            )
            .await
        });
        server
            .write_all(&[READY, VALUE, END].concat())
            .await
            .unwrap();
        settle().await;
        tokio::time::advance(Duration::from_secs(2)).await;
        let mut bytes = vec![];
        consumer.read_to_end(&mut bytes).await.unwrap();
        assert_eq!(task.await.unwrap(), 0);
        assert_eq!(bytes, [READY, VALUE, &local_end("duration_limit")].concat());
    }
    #[tokio::test(start_paused = true)]
    async fn undeliverable_local_terminal_is_failure_not_success() {
        let (mut server, socket) = tokio::io::duplex(4096);
        let (mut output, _consumer) = tokio::io::duplex(READY.len() + VALUE.len());
        let task = tokio::spawn(async move {
            observe(
                &mut reader(socket),
                &selected(None, Some(1)),
                &mut output,
                &mut std::future::pending(),
                Instant::now() + Duration::from_secs(35),
            )
            .await
        });
        server
            .write_all(&[READY, VALUE, END].concat())
            .await
            .unwrap();
        settle().await;
        tokio::time::advance(Duration::from_secs(5)).await;
        assert_eq!(task.await.unwrap(), 1);
    }
    struct Broken {
        writes: usize,
    }
    impl AsyncWrite for Broken {
        fn poll_write(
            mut self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
            _: &[u8],
        ) -> std::task::Poll<std::io::Result<usize>> {
            self.writes += 1;
            std::task::Poll::Ready(Err(std::io::ErrorKind::BrokenPipe.into()))
        }
        fn poll_flush(
            self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            unreachable!()
        }
        fn poll_shutdown(
            self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            unreachable!()
        }
    }
    #[tokio::test(start_paused = true)]
    async fn broken_output_gets_no_second_write_even_for_chosen_end() {
        let (mut server, socket) = tokio::io::duplex(4096);
        server.write_all(READY).await.unwrap();
        let mut out = Broken { writes: 0 };
        let exit = observe(
            &mut reader(socket),
            &selected(None, None),
            &mut out,
            &mut std::future::pending(),
            Instant::now() + Duration::from_secs(35),
        )
        .await;
        assert_eq!(exit, 1);
        assert_eq!(out.writes, 1);
    }
}
