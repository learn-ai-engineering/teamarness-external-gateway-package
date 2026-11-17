//! One numeric-loopback HTTP/1 exchange; no proxy, DNS, redirect, decompression or retry layer.
use super::output::{Failure, valid_json};
use crate::limits::*;
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper_util::rt::TokioIo;
use std::future::Future;
use std::{
    io,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::TcpStream,
    time::Sleep,
};
pub const CONNECT_SECONDS: u64 = 2;
pub const WRITE_SECONDS: u64 = 5;
pub const RESPONSE_SECONDS: u64 = 35;
fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "invalid HTTP response")
}
/// Inspect raw headers before Hyper can normalize duplicate lengths or conflicting framing.
pub fn validate_headers(bytes: &[u8], limit: usize) -> io::Result<()> {
    validate_response_headers(bytes, limit, false)
}
fn validate_response_headers(bytes: &[u8], limit: usize, stream: bool) -> io::Result<()> {
    if bytes.len() > MAX_HEADER_BYTES {
        return Err(invalid());
    }
    let text = std::str::from_utf8(bytes).map_err(|_| invalid())?;
    let mut lines = text.split("\r\n");
    let status = lines.next().ok_or_else(invalid)?;
    let mut words = status.split(' ');
    if words.next() != Some("HTTP/1.1") {
        return Err(invalid());
    }
    let code = words
        .next()
        .ok_or_else(invalid)?
        .parse::<u16>()
        .map_err(|_| invalid())?;
    if !(200..300).contains(&code) && !(400..600).contains(&code) {
        return Err(invalid());
    }
    let streaming = stream && code == 200;
    let (mut fields, mut length, mut transfer, mut content) = (0, None, false, false);
    for line in lines {
        if line.is_empty() {
            continue;
        }
        fields += 1;
        if fields > MAX_HEADERS || line.starts_with([' ', '\t']) {
            return Err(invalid());
        }
        let (name, value) = line.split_once(':').ok_or_else(invalid)?;
        let value = value.trim();
        match name.to_ascii_lowercase().as_str() {
            "content-length" => {
                if length.is_some()
                    || !value.bytes().all(|b| b.is_ascii_digit())
                    || value.is_empty()
                {
                    return Err(invalid());
                }
                let n = value.parse::<usize>().map_err(|_| invalid())?;
                if !streaming && n > limit {
                    return Err(invalid());
                }
                length = Some(n);
            }
            "transfer-encoding" => {
                if transfer || !value.eq_ignore_ascii_case("chunked") {
                    return Err(invalid());
                }
                transfer = true;
            }
            "content-type" => {
                let expected = if streaming {
                    "application/x-ndjson"
                } else {
                    "application/json"
                };
                if content || !value.eq_ignore_ascii_case(expected) {
                    return Err(invalid());
                }
                content = true;
            }
            "content-encoding" | "trailer" | "upgrade" => return Err(invalid()),
            _ => {}
        }
    }
    if !content || (transfer && length.is_some()) || (!transfer && length.is_none()) {
        return Err(invalid());
    }
    Ok(())
}
struct Wire<S> {
    socket: S,
    sent: Arc<AtomicBool>,
    write_clock: Option<Pin<Box<Sleep>>>,
    header: Vec<u8>,
    offset: usize,
    checked: bool,
    limit: usize,
    stream: bool,
}
impl<S: AsyncRead + Unpin> AsyncRead for Wire<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        out: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        while !self.checked {
            let mut chunk = [0; 1024];
            let mut buf = ReadBuf::new(&mut chunk);
            match Pin::new(&mut self.socket).poll_read(cx, &mut buf) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                Poll::Ready(Ok(())) => {}
            }
            if buf.filled().is_empty() {
                return Poll::Ready(Err(invalid()));
            }
            self.header.extend_from_slice(buf.filled());
            if let Some(end) = self.header.windows(4).position(|b| b == b"\r\n\r\n") {
                validate_response_headers(&self.header[..end + 4], self.limit, self.stream)?;
                self.checked = true;
            } else if self.header.len() > MAX_HEADER_BYTES {
                return Poll::Ready(Err(invalid()));
            }
        }
        if self.offset < self.header.len() {
            let n = out.remaining().min(self.header.len() - self.offset);
            out.put_slice(&self.header[self.offset..self.offset + n]);
            self.offset += n;
            return Poll::Ready(Ok(()));
        }
        // Header bytes have been handed off; retain no second long-lived read buffer.
        if !self.header.is_empty() {
            self.header = vec![];
            self.offset = 0;
        }
        Pin::new(&mut self.socket).poll_read(cx, out)
    }
}
impl<S: AsyncWrite + Unpin> AsyncWrite for Wire<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let clock = self.write_clock.get_or_insert_with(|| {
            Box::pin(tokio::time::sleep(Duration::from_secs(WRITE_SECONDS)))
        });
        if clock.as_mut().poll(cx).is_ready() {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "request write deadline",
            )));
        }
        let result = Pin::new(&mut self.socket).poll_write(cx, buf);
        if matches!(result,Poll::Ready(Ok(n)) if n>0) {
            self.sent.store(true, Ordering::SeqCst);
        }
        result
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.socket).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.socket).poll_shutdown(cx)
    }
}
struct Driver(tokio::task::JoinHandle<()>);
impl Drop for Driver {
    fn drop(&mut self) {
        self.0.abort();
    }
}
#[derive(Debug)]
pub struct Response {
    pub bytes: Vec<u8>,
    pub status: u16,
}
pub async fn request(
    port: u16,
    bearer: &str,
    target: &str,
    body: Vec<u8>,
    limit: usize,
    call_progress: Option<Arc<AtomicBool>>,
) -> Result<Response, Failure> {
    let sent = call_progress.clone().unwrap_or_default();
    let fail = || Failure::transport(call_progress.is_some() && sent.load(Ordering::SeqCst));
    let socket = connect(TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port)))
        .await
        .map_err(|_| fail())?
        .map_err(|_| fail())?;
    let wire = Wire {
        socket,
        sent: sent.clone(),
        write_clock: None,
        header: vec![],
        offset: 0,
        checked: false,
        limit,
        stream: false,
    };
    let work = async {
        let (mut sender, connection) = hyper::client::conn::http1::Builder::new()
            .max_headers(MAX_HEADERS)
            .max_buf_size(MAX_HEADER_BYTES)
            .handshake(TokioIo::new(wire))
            .await
            .map_err(|_| fail())?;
        let driver = Driver(tokio::spawn(async move {
            let _ = connection.await;
        }));
        let mut builder = http::Request::builder()
            .method(if body.is_empty() { "GET" } else { "POST" })
            .uri(target)
            .header("Host", format!("127.0.0.1:{port}"))
            .header("Authorization", format!("Bearer {bearer}"))
            .header("Connection", "close");
        if !body.is_empty() {
            builder = builder.header("Content-Type", "application/json");
        }
        let request = builder
            .body(Full::new(Bytes::from(body)))
            .map_err(|_| fail())?;
        let response = sender.send_request(request).await.map_err(|_| fail())?;
        let status = response.status().as_u16();
        let mut incoming = response.into_body();
        let mut bytes = vec![];
        while let Some(frame) = incoming.frame().await {
            let frame = frame.map_err(|_| fail())?;
            let data = frame.into_data().map_err(|_| fail())?;
            if data.len() > limit.saturating_sub(bytes.len()) {
                return Err(fail());
            }
            bytes.extend_from_slice(&data);
        }
        drop(sender);
        drop(driver);
        if !valid_json(&bytes) {
            return Err(fail());
        }
        Ok(Response { bytes, status })
    };
    tokio::time::timeout(Duration::from_secs(RESPONSE_SECONDS), work)
        .await
        .map_err(|_| fail())?
}

/// The connection future is polled by its body owner, never detached or reconnected.
pub struct StreamResponse {
    connection: hyper::client::conn::http1::Connection<TokioIo<Wire<TcpStream>>, Full<Bytes>>,
    incoming: hyper::body::Incoming,
    complete: bool,
    chunk: Bytes,
    line: Vec<u8>,
    pub status: u16,
}
impl StreamResponse {
    async fn chunk(&mut self) -> Result<Option<Bytes>, Failure> {
        use hyper::body::Body;
        std::future::poll_fn(|cx| {
            if !self.complete && Pin::new(&mut self.connection).poll(cx).is_ready() {
                self.complete = true;
            }
            Pin::new(&mut self.incoming).poll_frame(cx)
        })
        .await
        .transpose()
        .map_err(|_| Failure::transport(false))?
        .map(|f| f.into_data().map_err(|_| Failure::transport(false)))
        .transpose()
    }
    pub async fn refusal(&mut self) -> Result<Vec<u8>, Failure> {
        let mut bytes = vec![];
        while let Some(chunk) = self.chunk().await? {
            if chunk.len()
                > (PORT_VALUE_BYTES + ENVELOPE_ALLOWANCE_BYTES).saturating_sub(bytes.len())
            {
                return Err(Failure::transport(false));
            }
            bytes.extend_from_slice(&chunk);
        }
        if !valid_json(&bytes) || !super::output::upstream_error(&bytes) {
            return Err(Failure::transport(false));
        }
        Ok(bytes)
    }
    /// Cancellation-safe: a partial line stays in this single bounded read buffer.
    pub async fn line(&mut self) -> Result<Vec<u8>, Failure> {
        use bytes::Buf;
        loop {
            if self.chunk.is_empty() {
                self.chunk = self
                    .chunk()
                    .await?
                    .ok_or_else(|| Failure::transport(false))?;
                continue;
            }
            let end = self.chunk.iter().position(|b| *b == b'\n');
            let count = end.map_or(self.chunk.len(), |n| n + 1);
            if count > (PORT_VALUE_BYTES + ENVELOPE_ALLOWANCE_BYTES).saturating_sub(self.line.len())
            {
                return Err(Failure::transport(false));
            }
            self.line.extend_from_slice(&self.chunk[..count]);
            self.chunk.advance(count);
            if end.is_some() {
                return Ok(std::mem::take(&mut self.line));
            }
        }
    }
}
pub async fn open_stream(
    port: u16,
    bearer: &str,
    body: Vec<u8>,
) -> Result<StreamResponse, Failure> {
    let fail = || Failure::transport(false);
    let socket = connect(TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port)))
        .await
        .map_err(|_| fail())?
        .map_err(|_| fail())?;
    let wire = Wire {
        socket,
        sent: Arc::default(),
        write_clock: None,
        header: vec![],
        offset: 0,
        checked: false,
        limit: PORT_VALUE_BYTES + ENVELOPE_ALLOWANCE_BYTES,
        stream: true,
    };
    let (mut sender, mut connection) = hyper::client::conn::http1::Builder::new()
        .max_headers(MAX_HEADERS)
        .max_buf_size(MAX_HEADER_BYTES)
        .handshake(TokioIo::new(wire))
        .await
        .map_err(|_| fail())?;
    let request = http::Request::builder()
        .method("POST")
        .uri("/v1/subscribe")
        .header("Host", format!("127.0.0.1:{port}"))
        .header("Authorization", format!("Bearer {bearer}"))
        .header("Connection", "close")
        .header("Content-Type", "application/json")
        .body(Full::new(Bytes::from(body)))
        .map_err(|_| fail())?;
    let response = sender.send_request(request);
    tokio::pin!(response);
    let mut complete = false;
    let response = tokio::select! {biased;
        response = &mut response => response.map_err(|_| fail())?,
        _ = &mut connection => { complete = true; response.await.map_err(|_| fail())? },
    };
    let status = response.status().as_u16();
    drop(sender);
    Ok(StreamResponse {
        connection,
        incoming: response.into_body(),
        complete,
        chunk: Bytes::new(),
        line: vec![],
        status,
    })
}

async fn connect<T>(
    work: impl Future<Output = io::Result<T>>,
) -> Result<io::Result<T>, tokio::time::error::Elapsed> {
    tokio::time::timeout(Duration::from_secs(CONNECT_SECONDS), work).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    #[tokio::test(start_paused = true)]
    async fn connect_deadline_is_two_seconds_at_os_adapter() {
        let task = tokio::spawn(connect(std::future::pending::<io::Result<()>>()));
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_millis(1999)).await;
        assert!(!task.is_finished());
        tokio::time::advance(Duration::from_millis(1)).await;
        assert!(task.await.unwrap().is_err());
    }
    #[tokio::test(start_paused = true)]
    async fn stalled_nonblocking_request_write_expires_at_five_seconds() {
        let (socket, _peer) = tokio::io::duplex(1);
        let sent = Arc::new(AtomicBool::new(false));
        let mut wire = Wire {
            socket,
            sent: sent.clone(),
            write_clock: None,
            header: vec![],
            offset: 0,
            checked: false,
            limit: 32,
            stream: false,
        };
        let task = tokio::spawn(async move { wire.write_all(b"request bytes").await });
        tokio::task::yield_now().await;
        assert!(sent.load(Ordering::SeqCst));
        tokio::time::advance(Duration::from_millis(4999)).await;
        assert!(!task.is_finished());
        tokio::time::advance(Duration::from_millis(1)).await;
        assert_eq!(
            task.await.unwrap().unwrap_err().kind(),
            io::ErrorKind::TimedOut
        );
    }
    #[tokio::test]
    async fn response_deadline_is_thirty_five_seconds_and_does_not_retry() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (ready, accepted) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut byte = [0];
            socket.read_exact(&mut byte).await.unwrap();
            ready.send(()).unwrap();
            let mut bytes = vec![];
            let _ = socket.read_to_end(&mut bytes).await;
            // No second connection should have been attempted.
            assert!(
                tokio::time::timeout(Duration::from_secs(1), listener.accept())
                    .await
                    .is_err()
            );
        });
        let progress = Arc::new(AtomicBool::new(false));
        let task = tokio::spawn(request(
            port,
            "fixture",
            "/v1/call",
            b"{}".to_vec(),
            32,
            Some(progress),
        ));
        accepted.await.unwrap();
        tokio::time::pause();
        tokio::time::advance(Duration::from_secs(34)).await;
        assert!(!task.is_finished());
        tokio::time::advance(Duration::from_secs(2)).await;
        assert!(task.await.unwrap().unwrap_err().uncertain);
        tokio::time::resume();
        server.await.unwrap();
    }
}
