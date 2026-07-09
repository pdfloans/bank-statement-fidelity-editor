//! Headless HTTP server - additive entry point for container/cloud
//! deployments (e.g. Railway, Fly, Cloud Run).
//!
//! This module does NOT change the GUI, the CLI job model, or the
//! [`crate::app::runtime::Runtime`]. It only wraps the *existing*
//! `Job`/`JobResult` channel in a minimal HTTP/1.1 health surface so that:
//!
//!   1. the platform healthcheck has a port to talk to, and
//!   2. the process stays alive instead of exiting like the one-shot CLI
//!      subcommands do.
//!
//! It is implemented with `std::net` only, so it pulls **no new crates**
//! into the dependency graph.
//!
//! Endpoints (all `GET`):
//!   * `/health`, `/healthz`, `/livez` - liveness. Returns 200 as soon as
//!     the listener is up. This is the cheap probe a platform should hit.
//!   * `/readyz`, `/ready`            - readiness. Pings the worker actor
//!     (`Job::Ping` -> `JobResult::Pong`); 200 when it answers, 503 if not.
//!   * `/`                            - plain-text banner.
//!
//! The port is taken from the `PORT` environment variable (Railway sets
//! this automatically), defaulting to `8080`. The bind address is always
//! `0.0.0.0` so the container is reachable from the platform proxy.

use crate::app::runtime::{Job, JobResult};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// The existing runtime channel, guarded so concurrent connections take
/// turns when they need to exercise the worker (readiness probe). Liveness
/// probes never touch this.
type RuntimeChannel = Arc<Mutex<(Sender<Job>, Receiver<JobResult>)>>;

/// Default listen port when `$PORT` is unset.
const DEFAULT_PORT: u16 = 8080;

/// How long a readiness probe waits for the worker to answer `Ping`.
const READY_TIMEOUT: Duration = Duration::from_secs(5);

/// Run the blocking accept loop. Returns only on a fatal listener error;
/// in normal operation it runs for the lifetime of the process.
pub fn run_server(
    job_tx: Sender<Job>,
    job_rx: Receiver<JobResult>,
    _config: Arc<crate::app::config::AppConfig>,
) -> std::io::Result<()> {
    let port = std::env::var("PORT")
        .ok()
        .and_then(|p| p.trim().parse::<u16>().ok())
        .unwrap_or(DEFAULT_PORT);
    let addr = format!("0.0.0.0:{port}");

    let listener = TcpListener::bind(&addr)?;
    tracing::info!("[serve] listening on http://{addr} (GET /health for liveness)");
    println!("[serve] listening on http://{addr}  •  liveness: /health  •  readiness: /readyz");

    let channel: RuntimeChannel = Arc::new(Mutex::new((job_tx, job_rx)));

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let channel = channel.clone();
                // One thread per connection keeps the loop simple and stops
                // a slow client from blocking healthchecks.
                std::thread::spawn(move || {
                    if let Err(e) = handle_connection(stream, &channel) {
                        tracing::debug!("[serve] connection error: {e}");
                    }
                });
            }
            Err(e) => tracing::warn!("[serve] accept failed: {e}"),
        }
    }

    Ok(())
}

/// Read the request line, route it, and write the response.
fn handle_connection(mut stream: TcpStream, channel: &RuntimeChannel) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;

    // The request line ("GET /path HTTP/1.1") is always first, so a single
    // read of the head of the request is enough to route.
    let mut buf = [0u8; 1024];
    let n = stream.read(&mut buf).unwrap_or(0);
    let req = String::from_utf8_lossy(&buf[..n]);
    let (method, path) = parse_request_line(&req);

    let (status, content_type, body) = route(method, path, channel);
    write_response(&mut stream, status, content_type, &body)
}

/// Extract the method and path from the HTTP request line. Falls back to
/// sensible defaults for a malformed/empty request.
fn parse_request_line(req: &str) -> (&str, &str) {
    let first = req.lines().next().unwrap_or("");
    let mut parts = first.split_whitespace();
    let method = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("/");
    (method, path)
}

/// Map a method + path to an HTTP status, content type, and body.
fn route(
    method: &str,
    path: &str,
    channel: &RuntimeChannel,
) -> (&'static str, &'static str, String) {
    if method != "GET" {
        return (
            "405 Method Not Allowed",
            "text/plain; charset=utf-8",
            "method not allowed\n".to_string(),
        );
    }

    // Ignore any query string when matching.
    let path = path.split('?').next().unwrap_or(path);

    match path {
        "/health" | "/healthz" | "/livez" => (
            "200 OK",
            "application/json",
            r#"{"status":"ok"}"#.to_string(),
        ),
        "/readyz" | "/ready" => {
            if ping_worker(channel) {
                (
                    "200 OK",
                    "application/json",
                    r#"{"status":"ready"}"#.to_string(),
                )
            } else {
                (
                    "503 Service Unavailable",
                    "application/json",
                    r#"{"status":"not-ready"}"#.to_string(),
                )
            }
        }
        "/" => (
            "200 OK",
            "text/html; charset=utf-8",
            "<!DOCTYPE html>
<html>
<head>
    <title>Bank Statement Fidelity Editor</title>
    <style>
        body { font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Helvetica, Arial, sans-serif; background-color: #121212; color: #ffffff; display: flex; flex-direction: column; align-items: center; justify-content: center; height: 100vh; margin: 0; text-align: center; }
        .container { max-width: 600px; padding: 40px; background-color: #1e1e1e; border-radius: 12px; box-shadow: 0 4px 20px rgba(0,0,0,0.5); }
        h1 { color: #4facfe; margin-top: 0; }
        p { line-height: 1.6; color: #cccccc; }
        .code { background: #2d2d2d; padding: 10px; border-radius: 6px; font-family: monospace; color: #a3be8c; }
        .endpoints { margin-top: 30px; display: flex; gap: 20px; justify-content: center; }
        .endpoint { padding: 10px 20px; background: #252525; border: 1px solid #333; border-radius: 6px; }
    </style>
</head>
<body>
    <div class='container'>
        <h1>Bank Statement Fidelity Editor</h1>
        <h2>Headless Backend Mode</h2>
        <p>This deployment is successfully running the backend worker and API.</p>
        <p><strong>Note:</strong> This application is a native desktop application. The current deployment serves as a headless health-check and worker backend, not a Web GUI.</p>
        <p>To use the graphical interface, please download and run the application locally on your desktop environment.</p>
        <div class='endpoints'>
            <div class='endpoint'>Liveness: <span class='code'>GET /health</span></div>
            <div class='endpoint'>Readiness: <span class='code'>GET /readyz</span></div>
        </div>
    </div>
</body>
</html>"
                .to_string(),
        ),
        _ => (
            "404 Not Found",
            "text/plain; charset=utf-8",
            "not found\n".to_string(),
        ),
    }
}

/// Exercise the worker actor over the existing channel: send `Job::Ping`
/// and wait (bounded) for `JobResult::Pong`. Serialised behind the mutex so
/// concurrent readiness probes don't race on the single-consumer receiver.
fn ping_worker(channel: &RuntimeChannel) -> bool {
    let guard = match channel.lock() {
        Ok(g) => g,
        Err(_) => return false,
    };
    let (tx, rx) = &*guard;

    if tx.send(Job::Ping).is_err() {
        return false;
    }

    // Drain until we see the Pong or time out. In serve mode nothing else
    // produces results, so this normally returns on the first message.
    loop {
        match rx.recv_timeout(READY_TIMEOUT) {
            Ok(JobResult::Pong) => return true,
            Ok(_) => continue,
            Err(_) => return false,
        }
    }
}

/// Write a complete HTTP/1.1 response with `Connection: close`.
fn write_response(
    stream: &mut TcpStream,
    status_line: &str,
    content_type: &str,
    body: &str,
) -> std::io::Result<()> {
    let response = format!(
        "HTTP/1.1 {status_line}\r\n\
         Content-Type: {content_type}\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n\
         {body}",
        body.len()
    );
    stream.write_all(response.as_bytes())?;
    stream.flush()
}
