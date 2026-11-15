//! HTTP/1 application surface. Hyper owns parsing/framing; policy owns authority.
use crate::{error, limits::*, policy::Policy};
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{Request, Response, body::Incoming, server::conn::http1, service::service_fn};
use hyper_util::rt::{TokioIo, TokioTimer};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{convert::Infallible, sync::Arc, time::Duration};
use teamarness_package_sdk::BackendError;
use tokio::net::TcpStream;

type Reply = Response<Full<Bytes>>;
pub async fn serve(socket: TcpStream, port: u16, policy: Arc<Policy>) {
    serve_application(socket, move |request, accepted| {
        handle(request, port, policy.clone(), accepted)
    })
    .await;
}
async fn serve_application<F, Fut>(socket: TcpStream, handler: F)
where
    F: Fn(Request<Incoming>, tokio::time::Instant) -> Fut,
    Fut: std::future::Future<Output = Result<Value, BackendError>>,
{
    let accepted = tokio::time::Instant::now();
    let Ok(Ok(socket)) = tokio::time::timeout_at(
        accepted + Duration::from_secs(REQUEST_READ_TIMEOUT as u64),
        ConnectionIo::admit(socket),
    )
    .await
    else {
        return;
    };
    let service = service_fn(move |request| {
        let response = handler(request, accepted);
        async move {
            Ok::<_, Infallible>(match response.await {
                Ok(v) => reply(200, v),
                Err(e) => failure(e),
            })
        }
    });
    let _ = http1::Builder::new()
        .keep_alive(false)
        .max_headers(MAX_HEADERS)
        .max_buf_size(MAX_HEADER_BYTES)
        .timer(TokioTimer::new())
        .header_read_timeout(Duration::from_secs(REQUEST_READ_TIMEOUT as u64))
        .serve_connection(TokioIo::new(socket), service)
        .await;
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Call {
    contract: String,
    major: u16,
    port: String,
    request: Value,
}
async fn handle(
    request: Request<Incoming>,
    port: u16,
    policy: Arc<Policy>,
    accepted: tokio::time::Instant,
) -> Result<Value, BackendError> {
    policy.charge()?;
    let headers = request.headers();
    if request.version() != http::Version::HTTP_11
        || headers.get_all("host").iter().count() != 1
        || headers.get("host").and_then(|h| h.to_str().ok())
            != Some(format!("127.0.0.1:{port}").as_str())
        || headers.contains_key("origin")
        || request.uri().scheme().is_some()
        || request.uri().authority().is_some()
    {
        return Err(error("bad_request"));
    }
    if headers.get_all("authorization").iter().count() != 1 {
        return Err(error("unauthorized"));
    }
    let bearer = headers
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .ok_or_else(|| error("unauthorized"))?
        .to_owned();
    policy.authenticate(&bearer).await?;
    if request.uri().to_string().len() > MAX_TARGET_BYTES {
        return Err(error("too_large"));
    }
    if [
        "forwarded",
        "x-forwarded-host",
        "x-forwarded-for",
        "x-forwarded-proto",
    ]
    .iter()
    .any(|h| headers.contains_key(*h))
        || headers.contains_key("cookie")
        || headers.contains_key("upgrade")
        || headers.contains_key("expect")
        || headers.contains_key("trailer")
    {
        return Err(error("bad_request"));
    }
    if headers.contains_key("content-encoding") {
        return Err(error("unsupported_media_type"));
    }
    if headers
        .get("transfer-encoding")
        .is_some_and(|v| v != "chunked")
    {
        return Err(error("bad_request"));
    }
    let path = request.uri().path().to_owned();
    let query = request.uri().query().map(str::to_owned);
    if request.method() == http::Method::POST && path == "/v1/call" {
        if query.is_some() {
            return Err(error("bad_request"));
        }
        if headers.get_all("content-type").iter().count() != 1
            || headers
                .get("content-type")
                .is_none_or(|h| h != "application/json")
        {
            return Err(error("unsupported_media_type"));
        }
        if headers
            .get("content-length")
            .and_then(|h| h.to_str().ok())
            .and_then(|s| s.parse::<usize>().ok())
            .is_some_and(|n| n > PORT_VALUE_BYTES + ENVELOPE_ALLOWANCE_BYTES)
        {
            return Err(error("too_large"));
        }
        let mut body = request.into_body();
        let mut bytes = Vec::new();
        tokio::time::timeout_at(
            accepted + Duration::from_secs(REQUEST_READ_TIMEOUT as u64),
            async {
                while let Some(frame) = body.frame().await {
                    let data = frame
                        .map_err(|_| error("bad_request"))?
                        .into_data()
                        .map_err(|_| error("bad_request"))?;
                    if bytes.len() + data.len() > PORT_VALUE_BYTES + ENVELOPE_ALLOWANCE_BYTES {
                        return Err(error("too_large"));
                    }
                    bytes.extend_from_slice(&data);
                }
                Ok(())
            },
        )
        .await
        .map_err(|_| error("timeout"))??;
        let call: Call = serde_json::from_slice(&bytes).map_err(|_| error("bad_request"))?;
        if !teamarness_package_contract::identity::is_valid_package_identity(&call.contract)
            || call.major == 0
            || call.port.is_empty()
            || call.port.len() > 128
        {
            return Err(error("bad_request"));
        }
        let result =
            observe(policy.query(bearer, call.contract, call.major, call.port, call.request))
                .await?;
        return Ok(json!({"result":result}));
    }
    if request.method() != http::Method::GET {
        return Err(error("method_not_allowed"));
    }
    if headers.contains_key("transfer-encoding")
        || headers.get("content-length").is_some_and(|v| v != "0")
    {
        return Err(error("bad_request"));
    }
    if query.is_some() && path != "/v1/contracts" {
        return Err(error("bad_request"));
    }
    match path.as_str() {
        "/v1" => Ok(
            json!({"version":1,"readOnly":true,"workspace":policy.workspace,"routes":{"contracts":"/v1/contracts","onboarding":"/v1/onboarding","call":"/v1/call"},"limits":{"outstandingSdkInvocations":MAX_HOST_CALLS,"portValueBytes":PORT_VALUE_BYTES,"metadataBytes":METADATA_BYTES,"toolWaitSeconds":TOOL_WAIT_TIMEOUT}}),
        ),
        "/v1/onboarding" => Ok(json!({"markdown":ONBOARDING})),
        "/v1/contracts" => {
            let cursor = match query.as_deref() {
                None => None,
                Some(q) => {
                    let c = q
                        .strip_prefix("cursor=")
                        .filter(|c| {
                            !c.is_empty()
                                && c.len() <= 2048
                                && c.bytes()
                                    .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
                        })
                        .ok_or_else(|| error("bad_request"))?;
                    Some(c)
                }
            };
            tokio::time::timeout(
                Duration::from_secs(CATALOG_READ_TIMEOUT as u64),
                policy.external_page(&bearer, cursor),
            )
            .await
            .map_err(|_| error("timeout"))?
        }
        _ => {
            let parts: Vec<_> = path.split('/').collect();
            if !(parts.len() == 5 || parts.len() == 6 && parts[5] == "guide")
                || parts.get(1) != Some(&"v1")
                || parts.get(2) != Some(&"contracts")
            {
                return Err(error("not_found"));
            }
            let contract = parts[3];
            let major = parts[4]
                .parse::<u16>()
                .ok()
                .filter(|m| *m > 0)
                .ok_or_else(|| error("bad_request"))?;
            if !teamarness_package_contract::identity::is_valid_package_identity(contract) {
                return Err(error("bad_request"));
            }
            if parts.len() == 6 {
                observe(policy.guide(bearer, contract.into(), major)).await
            } else {
                tokio::time::timeout(
                    Duration::from_secs(CATALOG_READ_TIMEOUT as u64),
                    policy.schema(&bearer, contract, major),
                )
                .await
                .map_err(|_| error("timeout"))?
            }
        }
    }
}
const ONBOARDING: &str = include_str!("../../docs/onboarding.md");
fn reply(status: u16, value: Value) -> Reply {
    Response::builder()
        .status(status)
        .header("Content-Type", "application/json")
        .header("Cache-Control", "no-store")
        .header("X-Content-Type-Options", "nosniff")
        .header("Connection", "close")
        .body(Full::new(Bytes::from(
            serde_json::to_vec(&value).expect("JSON value"),
        )))
        .expect("fixed response")
}
fn failure(e: BackendError) -> Reply {
    let mut source = if e.code().starts_with("external_gateway.") {
        "gateway"
    } else {
        "host"
    };
    let mut code = e.code();
    let mut retryable = e.retryable();
    let mut uncertain = e.outcome_uncertain();
    let mut details = json!({});
    if code == "provider_error"
        && let Some(provider) = e.details().get("provider")
        && let Some(c) = provider.get("code").and_then(Value::as_str).filter(|c| {
            !c.is_empty()
                && c.len() <= 128
                && c.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_.-".contains(&b))
        })
    {
        source = "provider";
        code = c;
        retryable = provider
            .get("retryable")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        uncertain |= provider
            .get("outcomeUncertain")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        // Only machine-readable numeric bounds and identifier fields are safe to project.
        if let Some(fields) = provider.get("details").and_then(Value::as_object) {
            for key in [
                "limit",
                "maximum",
                "minimum",
                "actual",
                "expected",
                "retryAfterMs",
                "field",
                "reason",
            ] {
                if let Some(v) = fields.get(key)
                    && (v.is_number()
                        || v.is_boolean()
                        || v.as_str().is_some_and(|s| {
                            s.len() <= 128
                                && !s.contains("eg1.")
                                && s.bytes()
                                    .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
                        }))
                {
                    details[key] = v.clone();
                }
            }
        }
    }
    if uncertain {
        retryable = false;
    }
    let status = match code {
        "external_gateway.unauthorized" => 401,
        "external_gateway.bad_request" => 400,
        "external_gateway.forbidden" => 403,
        "external_gateway.not_found" => 404,
        "external_gateway.method_not_allowed" => 405,
        "external_gateway.unsupported_media_type" => 415,
        "external_gateway.too_large" => 413,
        "external_gateway.rate_limited" => 429,
        "external_gateway.timeout" | "deadline_exceeded" => 504,
        "external_gateway.catalog_changed" | "port_catalog_changed" => 409,
        _ => 503,
    };
    reply(
        status,
        json!({"error":{"source":source,"code":code,"retryable":retryable,"outcomeUncertain":uncertain,"details":details}}),
    )
}

use std::{
    future::Future,
    io,
    pin::Pin,
    task::{Context, Poll},
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, ReadBuf};
/// Bounded raw header preflight prevents Hyper's deliberate CL/TE normalization from hiding
/// ambiguous framing. It never decodes a body or routes a request; Hyper remains the parser.
struct ConnectionIo<S> {
    socket: S,
    prefix: std::io::Cursor<Vec<u8>>,
    write_deadline: Option<Pin<Box<tokio::time::Sleep>>>,
}
impl ConnectionIo<TcpStream> {
    async fn admit(mut socket: TcpStream) -> io::Result<Self> {
        let mut prefix = Vec::new();
        loop {
            let mut chunk = [0; 4096];
            let remaining = (MAX_HEADER_BYTES + 1).saturating_sub(prefix.len());
            if remaining == 0 {
                return Err(io::ErrorKind::InvalidData.into());
            }
            let n = socket.read(&mut chunk[..remaining.min(4096)]).await?;
            if n == 0 {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            prefix.extend_from_slice(&chunk[..n]);
            if let Some(end) = prefix
                .windows(4)
                .position(|w| w == b"\r\n\r\n")
                .map(|p| p + 4)
            {
                if end > MAX_HEADER_BYTES {
                    return Err(io::ErrorKind::InvalidData.into());
                }
                let mut length = 0;
                let mut transfer = 0;
                for line in prefix[..end].split(|b| *b == b'\n').skip(1) {
                    let Some(colon) = line.iter().position(|b| *b == b':') else {
                        continue;
                    };
                    let name = &line[..colon];
                    if name.eq_ignore_ascii_case(b"content-length") {
                        length += 1;
                    }
                    if name.eq_ignore_ascii_case(b"transfer-encoding") {
                        transfer += 1;
                    }
                }
                if length > 1 || transfer > 1 || length > 0 && transfer > 0 {
                    return Err(io::ErrorKind::InvalidData.into());
                }
                return Ok(Self {
                    socket,
                    prefix: std::io::Cursor::new(prefix),
                    write_deadline: None,
                });
            }
        }
    }
}
impl<S: AsyncRead + Unpin> AsyncRead for ConnectionIo<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        out: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if self.prefix.position() < (self.prefix.get_ref().len() as u64) {
            Pin::new(&mut self.prefix).poll_read(cx, out)
        } else {
            Pin::new(&mut self.socket).poll_read(cx, out)
        }
    }
}
impl<S: AsyncWrite + Unpin> ConnectionIo<S> {
    fn write_expired(&mut self, cx: &mut Context<'_>) -> bool {
        self.write_deadline
            .get_or_insert_with(|| {
                Box::pin(tokio::time::sleep(Duration::from_secs(
                    RESPONSE_WRITE_TIMEOUT as u64,
                )))
            })
            .as_mut()
            .poll(cx)
            .is_ready()
    }
}
impl<S: AsyncWrite + Unpin> AsyncWrite for ConnectionIo<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        if self.write_expired(cx) {
            return Poll::Ready(Err(io::ErrorKind::TimedOut.into()));
        }
        Pin::new(&mut self.socket).poll_write(cx, bytes)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        // Hyper flushes while waiting for the service. The response deadline starts only
        // when actual response bytes first reach poll_write, never during Provider wait.
        if self
            .write_deadline
            .as_mut()
            .is_some_and(|d| d.as_mut().poll(cx).is_ready())
        {
            return Poll::Ready(Err(io::ErrorKind::TimedOut.into()));
        }
        Pin::new(&mut self.socket).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.socket).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod transport_tests {
    use super::*;
    use tokio::io::AsyncWriteExt;
    #[tokio::test(start_paused = true)]
    async fn write_clock_starts_with_response_bytes_and_expires_at_five_seconds() {
        // Real asynchronous duplex backpressure at the connection I/O seam, not an SDK timer.
        let (writer, _reader) = tokio::io::duplex(1);
        let mut io = ConnectionIo {
            socket: writer,
            prefix: std::io::Cursor::new(vec![]),
            write_deadline: None,
        };
        io.flush().await.unwrap();
        tokio::time::advance(Duration::from_secs(25)).await;
        assert!(io.write_deadline.is_none());
        io.write_all(b"x").await.unwrap();
        let blocked = tokio::spawn(async move { io.write_all(b"y").await });
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(4)).await;
        assert!(!blocked.is_finished());
        tokio::time::advance(Duration::from_secs(1)).await;
        assert_eq!(
            blocked.await.unwrap().unwrap_err().kind(),
            io::ErrorKind::TimedOut
        );
    }
}

#[cfg(test)]
mod observer_socket_tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    #[tokio::test(start_paused = true)]
    async fn incomplete_socket_headers_close_at_exactly_five_seconds() {
        let clock_guard = tokio::spawn(async {
            loop {
                tokio::task::yield_now().await;
            }
        });
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        let (entered, ready) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            entered.send(()).unwrap();
            serve_application(socket, |_, _| std::future::ready(Ok(json!({})))).await;
        });
        let mut client = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        client.write_all(b"G").await.unwrap();
        ready.await.unwrap();
        tokio::time::advance(Duration::from_millis(4999)).await;
        assert!(!server.is_finished());
        tokio::time::advance(Duration::from_millis(1)).await;
        server.await.unwrap();
        let mut out = Vec::new();
        client.read_to_end(&mut out).await.unwrap();
        assert!(out.is_empty());
        clock_guard.abort();
    }
    #[tokio::test(start_paused = true)]
    async fn real_socket_observer_returns_504_at_25_seconds_without_extending_sdk_guards() {
        let clock_guard = tokio::spawn(async {
            loop {
                tokio::task::yield_now().await;
            }
        });
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        let entered = Arc::new(tokio::sync::Notify::new());
        let ready = entered.clone();
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            // This is controlled local application work at the real HTTP observer seam,
            // not a claim that the SDK permits 25 seconds of Provider execution.
            serve_application(socket, move |_, _| {
                let ready = ready.clone();
                async move {
                    ready.notify_one();
                    observe(std::future::pending::<Result<Value, BackendError>>()).await
                }
            })
            .await;
        });
        let mut client = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        client
            .write_all(b"GET /v1 HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
            .await
            .unwrap();
        entered.notified().await;
        let read = tokio::spawn(async move {
            let mut out = vec![];
            client.read_to_end(&mut out).await.unwrap();
            out
        });
        tokio::time::advance(Duration::from_secs(24)).await;
        assert!(!read.is_finished());
        tokio::time::advance(Duration::from_secs(1)).await;
        let bytes = read.await.unwrap();
        assert!(bytes.starts_with(b"HTTP/1.1 504"));
        server.await.unwrap();
        clock_guard.abort();
    }
}
