//! A minimal HTTP/1.1 server on `std::net`.
//!
//! The app has no third-party dependencies by design, and crates.io is not
//! reachable from the build sandbox, so the web front-end speaks HTTP directly.
//! That is enough here: the front-end is a single page plus a handful of JSON
//! endpoints, all served over loopback or a LAN.
//!
//! Concurrency model — every request is handed to the one thread that owns the
//! `StudioApp` and answered over a reply channel. State therefore has a single
//! writer and no locks are needed on the application side.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::Duration;

/// A parsed request. Only what the front-end actually uses is kept.
pub struct Request {
    pub method: String,
    pub path: String,
    pub body: String,
}

/// A response ready to be written to the socket.
pub struct Response {
    pub status: u16,
    pub content_type: String,
    pub body: Vec<u8>,
}

impl Response {
    pub fn json(value: &crate::json::JsonValue) -> Self {
        Self {
            status: 200,
            content_type: "application/json".to_string(),
            body: value.to_json_string().into_bytes(),
        }
    }

    pub fn text(status: u16, body: impl Into<String>) -> Self {
        Self {
            status,
            content_type: "text/plain; charset=utf-8".to_string(),
            body: body.into().into_bytes(),
        }
    }

    pub fn html(body: impl Into<String>) -> Self {
        Self {
            status: 200,
            content_type: "text/html; charset=utf-8".to_string(),
            body: body.into().into_bytes(),
        }
    }

    pub fn asset(content_type: &str, body: &'static str) -> Self {
        Self {
            status: 200,
            content_type: content_type.to_string(),
            body: body.as_bytes().to_vec(),
        }
    }

    pub fn not_found(what: &str) -> Self {
        Self::text(404, format!("not found: {}", what))
    }

    /// `400` with a human-readable reason.
    pub fn bad_request(why: &str) -> Self {
        Self::text(400, why.to_string())
    }

    fn status_line(&self) -> &'static str {
        match self.status {
            200 => "200 OK",
            204 => "204 No Content",
            400 => "400 Bad Request",
            404 => "404 Not Found",
            405 => "405 Method Not Allowed",
            413 => "413 Payload Too Large",
            500 => "500 Internal Server Error",
            _ => "200 OK",
        }
    }

    /// Serialises the response with the headers the front-end needs.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.body.len() + 200);
        out.extend_from_slice(b"HTTP/1.1 ");
        out.extend_from_slice(self.status_line().as_bytes());
        out.extend_from_slice(b"\r\n");
        out.extend_from_slice(format!("Content-Type: {}\r\n", self.content_type).as_bytes());
        out.extend_from_slice(b"Content-Length: ");
        out.extend_from_slice(self.body.len().to_string().as_bytes());
        out.extend_from_slice(b"\r\n");
        // The SPA is same-origin and self-contained; no CORS needed. `nosniff`
        // keeps the JSON endpoints from being sniffed as a document.
        out.extend_from_slice(b"X-Content-Type-Options: nosniff\r\n");
        out.extend_from_slice(b"Cache-Control: no-store\r\n");
        out.extend_from_slice(b"Connection: close\r\n\r\n");
        out.extend_from_slice(&self.body);
        out
    }
}

/// Maximum request body we will buffer (1 MiB). Prompts and pasted diffs stay
/// far below this; anything larger is refused rather than exhausting memory.
const MAX_BODY: usize = 1024 * 1024;

/// Reads one request off the socket. Returns None on EOF or a malformed line.
fn read_request(stream: &mut TcpStream) -> Option<Request> {
    stream.set_read_timeout(Some(Duration::from_secs(30))).ok();
    let mut buf: Vec<u8> = Vec::with_capacity(2048);
    let mut chunk = [0u8; 2048];

    // Read until the end of the headers.
    let header_end = loop {
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(pos) = find_subslice(&buf, b"\r\n\r\n") {
            break pos;
        }
        if buf.len() > 64 * 1024 {
            return None;
        }
    };

    let head = String::from_utf8_lossy(&buf[..header_end]).to_string();
    let mut lines = head.split("\r\n");
    let request_line = lines.next()?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next()?.to_string();
    let target = parts.next()?.to_string();

    // Query strings are not used by this API; the path alone routes.
    let path = target.split('?').next().unwrap_or(&target).to_string();

    // Content-Length drives the body read.
    let mut content_length = 0usize;
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
                content_length = value.trim().parse().unwrap_or(0);
            }
        }
    }
    if content_length > MAX_BODY {
        return None;
    }

    let mut body = buf[header_end + 4..].to_vec();
    while body.len() < content_length {
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..n]);
    }
    body.truncate(content_length);

    Some(Request {
        method,
        path,
        body: String::from_utf8_lossy(&body).to_string(),
    })
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|w| w == needle)
}

/// A channel back to the application thread.
pub type Reply = Sender<Response>;

/// A request plus the channel to answer it on.
pub type Job = (Request, Reply);

/// A live Server-Sent-Events subscriber. The application thread owns these and
/// writes to them whenever the snapshot changes.
pub type Subscriber = TcpStream;

/// Binds the listener and starts accepting.
///
/// Returns three channels for the application thread to poll:
///   * `requests` — ordinary request/response jobs
///   * `subscribers` — newly opened SSE sockets, which the app thread now owns
///   * the bound local address, for the startup banner
///
/// Each connection gets its own thread, but none of them touch application
/// state: they only read bytes and forward. All state lives on the one thread
/// that polls `requests`, so there is a single writer and nothing to lock.
pub fn bind(
    host: &str,
    port: u16,
) -> std::io::Result<(Receiver<Job>, Receiver<Subscriber>, String)> {
    let listener = TcpListener::bind((host, port))?;
    let addr = listener
        .local_addr()
        .map(|a| a.to_string())
        .unwrap_or_else(|_| format!("{}:{}", host, port));

    let (req_tx, req_rx) = channel::<Job>();
    let (sse_tx, sse_rx) = channel::<Subscriber>();
    let req_tx = std::sync::Arc::new(req_tx);

    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = match stream {
                Ok(s) => s,
                Err(_) => continue,
            };
            let req_tx = req_tx.clone();
            let sse_tx = sse_tx.clone();
            std::thread::spawn(move || {
                let Some(req) = read_request(&mut stream) else {
                    return;
                };
                if req.path == "/api/events" {
                    // Hand the socket to the application thread; it will write
                    // the SSE headers and every event. If the client goes away
                    // the next write fails and the subscriber is dropped.
                    let _ = sse_tx.send(stream);
                    return;
                }
                let (reply, pending) = channel::<Response>();
                if req_tx.send((req, reply)).is_ok() {
                    if let Ok(resp) = pending.recv_timeout(Duration::from_secs(60)) {
                        let _ = stream.write_all(&resp.to_bytes());
                        let _ = stream.flush();
                    }
                }
            });
        }
    });

    Ok((req_rx, sse_rx, addr))
}

/// Writes the SSE preamble on a freshly registered subscriber.
pub fn begin_sse(stream: &mut TcpStream) -> bool {
    let head = b"HTTP/1.1 200 OK\r\n\
                 Content-Type: text/event-stream\r\n\
                 Cache-Control: no-store\r\n\
                 Connection: close\r\n\r\n";
    stream.write_all(head).is_ok() && stream.flush().is_ok()
}

/// Pushes one event. Returns false when the peer has gone away.
pub fn push_sse(stream: &mut TcpStream, event: &str, data: &str) -> bool {
    // SSE frames are line-based; embedded newlines would break the protocol.
    let payload = data.replace('\n', "\\n");
    let frame = format!("event: {}\ndata: {}\n\n", event, payload);
    stream.write_all(frame.as_bytes()).is_ok() && stream.flush().is_ok()
}
