//! The browser front-end.
//!
//! `aircode --web` starts a small HTTP server on loopback (or a LAN
//! address with `--host`) and serves a single-page app plus a JSON API. The
//! page is embedded in the binary — there is no build step, no bundler and no
//! CDN, so the web front-end works on a machine with no internet access at all.
//!
//! Concurrency: each connection is read on its own thread, but every request is
//! answered by the one thread that owns the `StudioApp`. Application state
//! therefore has a single writer and needs no locking. Live updates travel over
//! Server-Sent Events, which the page falls back to polling on if the stream
//! drops.

mod api;
mod http;
mod page;

use std::io::Write;
use std::net::TcpStream;
use std::sync::mpsc::RecvTimeoutError;

use crate::tui::term::{BUSY_TICK, IDLE_TICK};
use crate::ui::snapshot::AppSnapshot;
use crate::ui::studio::StudioApp;

/// Starts the server and blocks until the process is interrupted.
pub fn run(mut app: StudioApp, host: &str, port: u16) {
    let (requests, subscribers, addr) = match http::bind(host, port) {
        Ok(v) => v,
        Err(e) => {
            eprintln!(
                "[AirCode] could not bind {}:{} — {}",
                host, port, e
            );
            eprintln!(
                "[AirCode] try another port with --port N, or a different address with --host ADDR"
            );
            std::process::exit(1);
        }
    };

    let snap = app.snapshot();
    println!("AirCode {} — web UI", snap.version);
    println!("  serving on  http://{}", addr);
    println!("  workspace   {}", snap.workspace);
    println!("  model       {}", if snap.model.is_empty() { "unset" } else { &snap.model });
    println!("  press ctrl+c to stop");
    println!();
    println!("  The page talks to the same engine as the desktop app and the TUI,");
    println!("  so sessions, the agent stream and provider auth are shared.");
    let _ = std::io::stdout().flush();

    let mut live: Vec<TcpStream> = Vec::new();
    let mut last_json = String::new();

    loop {
        // 1. Pump the agent stream and the file watcher.
        app.tick();

        // 2. Answer whatever arrived.
        while let Ok((req, reply)) = requests.try_recv() {
            let resp = api::route(&mut app, &req).unwrap_or_else(|| {
                http::Response::not_found(&req.path)
            });
            let _ = reply.send(resp);
        }

        // 3. Adopt new SSE subscribers and write their preamble.
        while let Ok(mut stream) = subscribers.try_recv() {
            if http::begin_sse(&mut stream) {
                // Send the current frame immediately so the page is never blank.
                let s = app.snapshot();
                let _ = http::push_sse(&mut stream, "state", &encode(&s));
                live.push(stream);
            }
        }

        // 4. Push a frame when something actually changed. `swap_remove` keeps
        //    this O(1) per dead peer and avoids cloning the socket (which would
        //    leak a duplicate descriptor per event).
        let snap = app.snapshot();
        let json = encode(&snap);
        if json != last_json {
            last_json = json.clone();
            let mut i = 0;
            while i < live.len() {
                if http::push_sse(&mut live[i], "state", &json) {
                    i += 1;
                } else {
                    live.swap_remove(i);
                }
            }
        }

        // 5. Idle politely.
        let timeout = if snap.agent_running {
            BUSY_TICK
        } else {
            IDLE_TICK
        };
        match requests.recv_timeout(timeout) {
            Ok((req, reply)) => {
                let resp = api::route(&mut app, &req)
                    .unwrap_or_else(|| http::Response::not_found(&req.path));
                let _ = reply.send(resp);
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                // Every listener thread is gone; the server is finished.
                eprintln!("[AirCode] web server stopped.");
                return;
            }
        }
    }
}

/// Serialises a snapshot the same way `/api/state` does, for the SSE frames.
fn encode(snap: &AppSnapshot) -> String {
    crate::web::api::encode_state_json(snap)
}


// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use crate::json::JsonValue;
    use crate::sqlite::SqliteStore;
    use crate::ui::studio::StudioApp;
    use crate::web::http::Request;

    fn headless(tag: &str) -> (StudioApp, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "aircode_web_{}_{}_{}",
            tag,
            std::process::id(),
            line!()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let store = std::sync::Arc::new(SqliteStore::open(&dir).unwrap());
        let app = StudioApp::new_headless_for_test(store, dir.clone());
        (app, dir)
    }

    fn req(method: &str, path: &str, body: &str) -> Request {
        Request {
            method: method.to_string(),
            path: path.to_string(),
            body: body.to_string(),
        }
    }

    #[test]
    fn test_static_assets_are_served() {
        let (mut app, dir) = headless("static");
        for (path, ctype) in [
            ("/", "text/html; charset=utf-8"),
            ("/app.css", "text/css; charset=utf-8"),
            ("/app.js", "application/javascript; charset=utf-8"),
            ("/favicon.svg", "image/svg+xml"),
        ] {
            let r = api::route(&mut app, &req("GET", path, "")).expect(path);
            assert_eq!(r.status, 200, "{} status", path);
            assert_eq!(r.content_type, ctype, "{} content-type", path);
            assert!(!r.body.is_empty(), "{} empty body", path);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_index_references_the_real_assets() {
        let (mut app, dir) = headless("index");
        let r = api::route(&mut app, &req("GET", "/", "")).unwrap();
        let html = String::from_utf8_lossy(&r.body);
        for needle in ["/app.css", "/app.js", "id=\"prompt\"", "id=\"transcript\""] {
            assert!(html.contains(needle), "index.html missing {}", needle);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_unknown_path_is_404() {
        let (mut app, dir) = headless("404");
        assert!(api::route(&mut app, &req("GET", "/nope", "")).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_state_json_has_the_fields_the_page_reads() {
        let (mut app, dir) = headless("state");
        let r = api::route(&mut app, &req("GET", "/api/state", "")).unwrap();
        let v = JsonValue::parse(&String::from_utf8_lossy(&r.body)).expect("valid JSON");
        for key in [
            "version", "workspace", "view", "messages", "streaming", "prompt",
            "history", "model", "tokens", "contextPct", "cost", "mcp", "todos",
            "modified", "approval", "connect", "theme", "sessions",
        ] {
            assert!(v.get(key).is_some(), "state JSON missing {}", key);
        }
        // The theme must carry real colours, not zeroes.
        let theme = v.get("theme").unwrap();
        let accent = theme.get("accent").and_then(|c| c.as_i64()).unwrap_or(0);
        assert!(accent > 0, "theme colours are all zero");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_char_and_key_reach_the_composer() {
        let (mut app, dir) = headless("input");
        let r = api::route(&mut app, &req("POST", "/api/char", "{\"ch\":\"hi\"}")).unwrap();
        assert_eq!(r.status, 200);
        assert_eq!(app.prompt_text(), "hi");

        api::route(&mut app, &req("POST", "/api/key", "{\"key\":\"backspace\"}")).unwrap();
        assert_eq!(app.prompt_text(), "h");

        // An unknown key is a 400, not a panic.
        let bad = api::route(&mut app, &req("POST", "/api/key", "{\"key\":\"nonsense\"}")).unwrap();
        assert_eq!(bad.status, 400);

        // ctrl+L clears the line. The route used to compare the lowercase "l"
        // straight against the app's uppercase Letter('L'), so the clear silently
        // did nothing.
        api::route(&mut app, &req("POST", "/api/char", "{\"ch\":\"x\"}")).unwrap();
        assert_eq!(app.prompt_text(), "hx");
        let clr = api::route(&mut app, &req("POST", "/api/key", "{\"key\":\"l\",\"ctrl\":true}")).unwrap();
        assert_eq!(clr.status, 200);
        assert_eq!(app.prompt_text(), "");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_connect_opens_from_the_api() {
        let (mut app, dir) = headless("connect");
        let r = api::route(&mut app, &req("POST", "/api/action", "{\"action\":\"connect\"}")).unwrap();
        assert_eq!(r.status, 200);
        let snap = app.snapshot();
        assert_eq!(snap.shell_view, "connect");
        assert!(snap.connect.open);
        assert!(!snap.connect.zen_models.is_empty(), "Zen catalog should be listed");

        // Closing returns to the conversation.
        api::route(&mut app, &req("POST", "/api/connect/close", "{}")).unwrap();
        assert_eq!(app.shell_view(), crate::ui::shell::ShellView::Chat);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_palette_command_ids_are_routable() {
        let (mut app, dir) = headless("palette");
        // "cmd:*" ids go through the same table the GUI palette uses.
        let r = api::route(&mut app, &req("POST", "/api/action", "{\"action\":\"cmd:shell_sidebar\"}")).unwrap();
        assert_eq!(r.status, 200);
        // A completely unknown id must be a clean 400.
        let bad = api::route(&mut app, &req("POST", "/api/action", "{\"action\":\"made-up\"}")).unwrap();
        assert_eq!(bad.status, 400);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_send_requires_a_prompt() {
        let (mut app, dir) = headless("send");
        let empty = api::route(&mut app, &req("POST", "/api/send", "{\"prompt\":\"   \"}")).unwrap();
        assert_eq!(empty.status, 400);
        let malformed = api::route(&mut app, &req("POST", "/api/send", "not json")).unwrap();
        assert_eq!(malformed.status, 400);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_sse_frame_format() {
        // A real socket, so the HTTP layer is exercised rather than assumed.
        let (requests, _subscribers, addr) = http::bind("127.0.0.1", 0).expect("bind");
        let port: u16 = addr.rsplit(':').next().unwrap().parse().unwrap();

        let handle = std::thread::spawn(move || {
            use std::io::{Read, Write};
            let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
            stream
                .write_all(b"GET /api/health HTTP/1.1\r\nHost: x\r\n\r\n")
                .unwrap();
            let mut buf = Vec::new();
            let mut chunk = [0u8; 1024];
            while let Ok(n) = stream.read(&mut chunk) {
                buf.extend_from_slice(&chunk[..n]);
                if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            String::from_utf8_lossy(&buf).to_string()
        });

        if let Ok((job, reply)) = requests.recv_timeout(std::time::Duration::from_secs(5)) {
            // The isolated headless app, not `StudioApp::new(None)`: the latter
            // opens the real ~/.local/share/aircode database, which makes this
            // test race against every other test running in parallel.
            let (mut app, dir) = headless("sse");
            let resp =
                api::route(&mut app, &job).unwrap_or_else(|| http::Response::not_found(&job.path));
            let _ = reply.send(resp);
            let _ = std::fs::remove_dir_all(&dir);
        }
        let raw = handle.join().expect("client thread");
        assert!(raw.starts_with("HTTP/1.1 200 OK"), "bad status line: {}", raw);
        assert!(raw.contains("Content-Type: application/json"));
        assert!(raw.contains("Content-Length:"), "missing Content-Length");
        assert!(raw.contains("X-Content-Type-Options: nosniff"));
        assert!(raw.contains("\"ok\":true"), "missing body: {}", raw);
    }

    #[test]
    fn test_response_to_bytes_is_well_formed() {
        let r = http::Response::text(404, "not here");
        let bytes = r.to_bytes();
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.starts_with("HTTP/1.1 404 Not Found\r\n"));
        assert!(text.ends_with("not here"));
        assert!(text.contains("Content-Length: 8\r\n"));
    }

    #[test]
    fn test_sse_helpers_round_trip() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            use std::io::Read;
            let (mut s, _) = listener.accept().unwrap();
            // Read to EOF rather than one fixed-size chunk: a single 256-byte
            // read can split the headers from the event frame and make this
            // test fail at random, which only ever hid real regressions.
            let mut buf = Vec::new();
            let _ = s.read_to_end(&mut buf);
            String::from_utf8_lossy(&buf).to_string()
        });
        let mut client = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        assert!(http::begin_sse(&mut client));
        assert!(http::push_sse(&mut client, "state", "{\"a\":1}"));
        drop(client);
        let got = handle.join().unwrap();
        assert!(got.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(got.contains("Content-Type: text/event-stream\r\n"));
        assert!(got.contains("event: state\ndata: {\"a\":1}\n\n"));
    }

    /// A prompt sent with no reachable provider must surface an error, not hang
    /// or crash — this is the failure the sandbox actually produces.
    #[test]
    fn test_send_surfaces_provider_errors_without_hanging() {
        let (mut app, dir) = headless("err");
        let r = api::route(&mut app, &req("POST", "/api/send", "{\"prompt\":\"hello\"}")).unwrap();
        assert_eq!(r.status, 200);
        // Drain the stream the same way the server loop does.
        for _ in 0..40 {
            app.tick();
            if !app.snapshot().agent_running {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let snap = app.snapshot();
        assert!(!snap.agent_running, "run never finished");
        assert_eq!(snap.messages.len(), 2, "user + assistant turns expected");
        assert_eq!(snap.messages[0].role, "user");
        assert_eq!(snap.messages[0].content, "hello");
        assert_eq!(snap.prompt_history.last().map(|s| s.as_str()), Some("hello"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The SSE encoder and `/api/state` must agree, or the page would flicker.
    #[test]
    fn test_sse_encoder_matches_state_endpoint() {
        let (app, dir) = headless("sse_match");
        let via_api = api::encode_state_json(&app.snapshot());
        assert!(via_api.starts_with('{'));
        assert!(via_api.contains("\"version\""));
        // A change must produce a different frame.
        let mut app2 = app;
        app2.handle_char_input('x');
        let after = api::encode_state_json(&app2.snapshot());
        assert_ne!(via_api, after, "snapshot did not change after typing");
        let _ = std::fs::remove_dir_all(&dir);
    }

}
