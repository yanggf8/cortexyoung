//! Synchronous Hrana HTTP client. A transport seam keeps the wire testable without a server.

use serde_json::{json, Value};
use std::fmt;
use std::net::ToSocketAddrs;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cell {
    Null,
    Int(i64),
    Text(String),
}

impl Cell {
    fn encode(&self) -> Value {
        match self {
            Self::Null => json!({"type":"null"}),
            Self::Int(n) => json!({"type":"integer","value":n.to_string()}),
            Self::Text(s) => json!({"type":"text","value":s}),
        }
    }

    fn decode(value: &Value) -> Result<Self, RemoteError> {
        match value.get("type").and_then(Value::as_str) {
            Some("null") => Ok(Self::Null),
            Some("integer") => value
                .get("value")
                .and_then(Value::as_str)
                .and_then(|s| s.parse().ok())
                .map(Self::Int)
                .ok_or_else(|| RemoteError::sql("invalid integer cell")),
            Some("text") => value
                .get("value")
                .and_then(Value::as_str)
                .map(|s| Self::Text(s.to_owned()))
                .ok_or_else(|| RemoteError::sql("invalid text cell")),
            _ => Err(RemoteError::sql("unsupported cell type")),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    Blocked,
    Unauthorized,
    Unreachable,
    Timeout,
    Busy,
    Constraint,
    Sql,
}
impl ErrorKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Blocked => "blocked",
            Self::Unauthorized => "unauthorized",
            Self::Unreachable => "unreachable",
            Self::Timeout => "timeout",
            Self::Busy => "busy",
            Self::Constraint => "constraint",
            Self::Sql => "sql",
        }
    }
}

/// `detail` is for trusted logs only; Display is safe for user-facing output.
#[derive(Clone, PartialEq, Eq)]
pub struct RemoteError {
    pub kind: ErrorKind,
    pub step: Option<usize>,
    detail: String,
}
impl RemoteError {
    /// Diagnostic detail for trusted logs only; never include it in user-facing output.
    pub fn detail(&self) -> &str {
        &self.detail
    }
    fn new(kind: ErrorKind, detail: impl Into<String>) -> Self {
        Self {
            kind,
            step: None,
            detail: detail.into(),
        }
    }
    fn sql(detail: impl Into<String>) -> Self {
        Self::new(ErrorKind::Sql, detail)
    }
    fn at(mut self, step: usize) -> Self {
        self.step = Some(step);
        self
    }
}
impl fmt::Display for RemoteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.step {
            Some(step) => write!(f, "{} at step {step}", self.kind.as_str()),
            None => f.write_str(self.kind.as_str()),
        }
    }
}
impl fmt::Debug for RemoteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RemoteError")
            .field("kind", &self.kind)
            .field("step", &self.step)
            .field("detail", &"<log only>")
            .finish()
    }
}
impl std::error::Error for RemoteError {}

// Hrana carries SQLite symbolic codes (including extended SQLITE_CONSTRAINT_* codes).
fn classify(
    code: Option<&str>,
    message: Option<&str>,
    status: Option<u16>,
    transport: Option<ErrorKind>,
) -> ErrorKind {
    if let Some(kind) = transport {
        return kind;
    }
    // `None` is a code this client does not know (say, an auth code on a 401): the HTTP status
    // then decides. A known code, including a generic `SQLITE_*` one, wins over the status.
    let coded = match code.unwrap_or("") {
        "BLOCKED" => Some(ErrorKind::Blocked),
        c if c == "SQLITE_BUSY"
            || c.starts_with("SQLITE_BUSY_")
            || c == "SQLITE_LOCKED"
            || c.starts_with("SQLITE_LOCKED_") =>
        {
            Some(ErrorKind::Busy)
        }
        c if c == "SQLITE_CONSTRAINT" || c.starts_with("SQLITE_CONSTRAINT_") => {
            Some(ErrorKind::Constraint)
        }
        c if c.starts_with("SQLITE_") => Some(ErrorKind::Sql),
        // Message text is a fallback for code-less errors only: an ordinary SQL error that names
        // a `blocked_users` table must not read as the quota block.
        "" if message.is_some_and(|m| m.to_ascii_lowercase().contains("blocked")) => {
            Some(ErrorKind::Blocked)
        }
        "" if message.is_some_and(|m| m.to_ascii_uppercase().contains("SQLITE_BUSY")) => {
            Some(ErrorKind::Busy)
        }
        _ => None,
    };
    if let Some(kind) = coded {
        return kind;
    }
    match status {
        Some(401 | 403) => ErrorKind::Unauthorized,
        Some(408 | 504) => ErrorKind::Timeout,
        Some(429 | 500..=503 | 505..=599) => ErrorKind::Unreachable,
        _ => ErrorKind::Sql,
    }
}
fn status_error(status: u16, body: &[u8]) -> RemoteError {
    let parsed: Option<Value> = serde_json::from_slice(body).ok();
    let code = parsed.as_ref().and_then(|value| {
        value
            .get("results")
            .and_then(Value::as_array)
            .and_then(|entries| {
                entries
                    .iter()
                    .find_map(|entry| entry.get("error")?.get("code")?.as_str())
            })
            .or_else(|| value.get("code").and_then(Value::as_str))
            .or_else(|| value.get("error")?.get("code")?.as_str())
    });
    RemoteError::new(
        classify(code, None, Some(status), None),
        code.map_or_else(
            || format!("HTTP {status}"),
            |code| format!("Hrana status error {code}"),
        ),
    )
}
fn hrana_error(value: &Value) -> RemoteError {
    let code = value.get("code").and_then(Value::as_str);
    let message = value.get("message").and_then(Value::as_str);
    // Server text can contain request data, so only the symbolic code reaches the log detail.
    RemoteError::new(
        classify(code, message, None, None),
        format!("Hrana statement error {}", code.unwrap_or("(no code)")),
    )
}

pub trait Transport {
    fn post(
        &self,
        url: &str,
        body: &[u8],
        headers: &[(&str, &str)],
    ) -> Result<Vec<u8>, RemoteError>;
}

/// When the recall hook's network waits must end, from `main()`. Every harness kills a hook at 5 s
/// (`settings::TIMEOUT_SECS` and its two TOML twins); the watchdog fires 900 ms after this, which
/// covers what the deadline cannot bound: a blocking socket read outlasted its 4.175 s timeout by
/// 30-210 ms on this machine (strace, 2026-10-29), a debug build spent ~35 ms between setting the
/// socket timeout and its first read, and rendering plus the usage row follow. With 500 ms of
/// headroom a slow CI runner still lost the race and printed nothing. It was 2500 ms until
/// 2026-10-29, when the first read after an idle spell timed out on three session starts in one
/// afternoon while warm reads took about 300 ms.
pub const RECALL_DEADLINE: Duration = Duration::from_millis(3800);

/// The hook makes sequential requests (including its ledger-only retry). Keep the latest
/// connection attempt visible to other threads without a mutex a stalled worker could hold.
/// Boundaries are published in order; zero means that boundary has not been reached.
struct NetworkRecorder {
    boundaries: [AtomicU64; 4],
    stopped: AtomicU64,
    /// Nonzero once a call gave up for lack of budget without sending a request; the boundaries
    /// then still describe the previous request, which is what spent the budget.
    budget: AtomicU64,
}

impl NetworkRecorder {
    const fn new() -> Self {
        Self {
            boundaries: [const { AtomicU64::new(0) }; 4],
            stopped: AtomicU64::new(0),
            budget: AtomicU64::new(0),
        }
    }

    fn reset(&self) {
        for boundary in &self.boundaries {
            boundary.store(0, Ordering::Release);
        }
        self.stopped.store(0, Ordering::Release);
        self.budget.store(0, Ordering::Release);
    }

    fn mark_budget(&self) {
        self.budget.store(1, Ordering::Release);
    }

    fn tick() -> u64 {
        static ORIGIN: OnceLock<Instant> = OnceLock::new();
        ORIGIN.get_or_init(Instant::now).elapsed().as_nanos() as u64 + 1
    }

    fn enter(&self, phase: usize) {
        self.boundaries[phase].store(Self::tick(), Ordering::Release);
    }

    fn stop(&self) {
        self.stopped.store(Self::tick(), Ordering::Release);
    }

    fn snapshot(&self) -> Option<Value> {
        self.snapshot_at(Self::tick())
    }

    fn snapshot_at(&self, now: u64) -> Option<Value> {
        // Read newest first: observing a boundary also observes all preceding writes.
        let stopped = self.stopped.load(Ordering::Acquire);
        let mut times = [0; 4];
        for i in (0..4).rev() {
            times[i] = self.boundaries[i].load(Ordering::Acquire);
        }
        let budget = self.budget.load(Ordering::Acquire) != 0;
        let Some(phase) = times.iter().rposition(|&t| t != 0) else {
            return budget.then(|| json!({"phase": "budget"}));
        };
        // A reset racing this read can leave a newer boundary beside cleared older ones; that
        // would print a plausible but invented duration, so report nothing instead.
        if times[..=phase].contains(&0) || times[..=phase].windows(2).any(|w| w[0] > w[1]) {
            return None;
        }
        let names = ["dns", "connect", "tls", "response"];
        let mut out = json!({"phase": if budget { "budget" } else { names[phase] }});
        for i in 0..=phase {
            let end = if i < phase {
                times[i + 1]
            } else if stopped != 0 {
                stopped
            } else {
                now
            };
            out[format!("{}_ms", names[i])] = json!(end.saturating_sub(times[i]) / 1_000_000);
        }
        Some(out)
    }
}

static NETWORK: NetworkRecorder = NetworkRecorder::new();

/// Last HTTP request's completed phases and elapsed time in its current (or final) phase.
/// Before DNS begins this is absent. Normal returns freeze it; a stuck request stays readable.
pub fn network_snapshot() -> Option<Value> {
    NETWORK.snapshot()
}

/// A recall that stops for lack of budget before sending its next request reports phase
/// `budget`, keeping the previous request's timings, instead of looking stalled in that request.
pub fn network_budget_exhausted() {
    NETWORK.mark_budget();
}

struct NetworkRequest;
impl Drop for NetworkRequest {
    fn drop(&mut self) {
        NETWORK.stop();
    }
}

/// Recompute the SessionStart allowance for each POST, including a ledger-only retry.
pub struct RecallTransport {
    pub started: Instant,
    pub max_timeout: Duration,
}

impl Transport for RecallTransport {
    fn post(
        &self,
        url: &str,
        body: &[u8],
        headers: &[(&str, &str)],
    ) -> Result<Vec<u8>, RemoteError> {
        let remaining = RECALL_DEADLINE.saturating_sub(self.started.elapsed());
        if remaining < Duration::from_millis(200) {
            network_budget_exhausted();
            return Err(RemoteError::new(
                ErrorKind::Timeout,
                "recall budget exhausted",
            ));
        }
        let timeout = self
            .max_timeout
            .min(remaining.saturating_sub(Duration::from_millis(100)));
        // ureq bounds connect, TLS and body reads; synchronous DNS resolution is outside it.
        UreqTransport { timeout }.post(url, body, headers)
    }
}

/// TLS for [`UreqTransport`], pinned to aws-lc-rs with `builder_with_provider`, so nothing here
/// depends on a process-wide provider and nothing compiles in a second one (see `Cargo.toml`).
struct AwsLcTls(Arc<rustls::ClientConfig>);

fn tls() -> Result<Arc<AwsLcTls>, RemoteError> {
    // Only a built config is cached, so a failure is retried on the next request.
    static TLS: OnceLock<Arc<AwsLcTls>> = OnceLock::new();
    if let Some(tls) = TLS.get() {
        return Ok(tls.clone());
    }
    let builder = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::aws_lc_rs::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(|_| RemoteError::new(ErrorKind::Unreachable, "TLS configuration failed"))?;
    let roots = rustls::RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    };
    let config = builder.with_root_certificates(roots).with_no_client_auth();
    Ok(TLS
        .get_or_init(|| Arc::new(AwsLcTls(Arc::new(config))))
        .clone())
}

struct TlsStream(rustls::StreamOwned<rustls::ClientConnection, Box<dyn ureq::ReadWrite>>);

impl ureq::ReadWrite for TlsStream {
    fn socket(&self) -> Option<&std::net::TcpStream> {
        self.0.get_ref().socket()
    }
}
impl std::io::Read for TlsStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }
}
impl std::io::Write for TlsStream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}
impl fmt::Debug for TlsStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("TlsStream")
    }
}

impl ureq::TlsConnector for AwsLcTls {
    fn connect(
        &self,
        dns_name: &str,
        mut io: Box<dyn ureq::ReadWrite>,
    ) -> Result<Box<dyn ureq::ReadWrite>, ureq::Error> {
        NETWORK.enter(2);
        // ureq's errors can only be built from an io::Error outside the crate; a handshake that
        // stalls keeps its TimedOut kind and classifies as Timeout.
        let host = match dns_name.strip_prefix('[').and_then(|h| h.strip_suffix(']')) {
            Some(bare) => bare,
            None => dns_name,
        };
        let name = rustls::pki_types::ServerName::try_from(host.to_owned()).map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid TLS server name")
        })?;
        let mut session = rustls::ClientConnection::new(self.0.clone(), name)
            .map_err(|_| std::io::Error::other("TLS session setup failed"))?;
        session.complete_io(&mut io)?;
        NETWORK.enter(3);
        Ok(Box::new(TlsStream(rustls::StreamOwned::new(session, io))))
    }
}

/// HTTP transport. ureq 2 resolves DNS synchronously, outside its overall timeout.
pub struct UreqTransport {
    pub timeout: Duration,
}
impl Transport for UreqTransport {
    fn post(
        &self,
        url: &str,
        body: &[u8],
        headers: &[(&str, &str)],
    ) -> Result<Vec<u8>, RemoteError> {
        use std::io::Read;
        NETWORK.reset();
        let _recording = NetworkRequest;
        // ureq 2.12's per-request timeout does not bound the TCP connect: that waits for the
        // agent's `timeout_connect` (30 s by default), which a blackholed address reaches in
        // full (strace on 2026-10-28: `poll(..., 29999)` under a 1 s request timeout).
        let agent = ureq::AgentBuilder::new()
            .tls_connector(tls()?)
            .resolver(|netloc: &str| {
                // A redirect may open another connection within this POST. Do not let the
                // preceding connection's TLS/response boundary hide a stall in its DNS.
                NETWORK.reset();
                NETWORK.enter(0);
                let result = netloc
                    .to_socket_addrs()
                    .map(|addrs| addrs.collect::<Vec<_>>());
                if result.as_ref().is_ok_and(|addrs| !addrs.is_empty()) {
                    NETWORK.enter(1);
                }
                result
            })
            .timeout_connect(self.timeout)
            .build();
        let mut request = agent.post(url).timeout(self.timeout);
        for (key, value) in headers {
            request = request.set(key, value);
        }
        match request.send_bytes(body) {
            Ok(response) => {
                let mut bytes = Vec::new();
                response
                    .into_reader()
                    .read_to_end(&mut bytes)
                    .map_err(|error| {
                        let cause = if matches!(
                            error.kind(),
                            std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                        ) {
                            ErrorKind::Timeout
                        } else {
                            ErrorKind::Unreachable
                        };
                        let kind = classify(None, None, None, Some(cause));
                        RemoteError::new(kind, "response read failed")
                    })?;
                Ok(bytes)
            }
            Err(ureq::Error::Status(status, response)) => {
                let mut bytes = Vec::new();
                let _ = response
                    .into_reader()
                    .take(64 * 1024)
                    .read_to_end(&mut bytes);
                Err(status_error(status, &bytes))
            }
            Err(ureq::Error::Transport(error)) => {
                let cause = match error.kind() {
                    ureq::ErrorKind::InvalidUrl
                    | ureq::ErrorKind::UnknownScheme
                    | ureq::ErrorKind::InvalidProxyUrl => ErrorKind::Sql,
                    ureq::ErrorKind::Dns
                    | ureq::ErrorKind::ConnectionFailed
                    | ureq::ErrorKind::Io => {
                        if std::error::Error::source(&error)
                            .and_then(|source| source.downcast_ref::<std::io::Error>())
                            .is_some_and(|io| {
                                matches!(
                                    io.kind(),
                                    std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                                )
                            })
                        {
                            ErrorKind::Timeout
                        } else {
                            ErrorKind::Unreachable
                        }
                    }
                    _ => ErrorKind::Unreachable,
                };
                Err(RemoteError::new(
                    classify(None, None, None, Some(cause)),
                    "HTTP transport failed",
                ))
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stmt {
    pub sql: String,
    pub args: Vec<Cell>,
}
impl Stmt {
    pub fn new(sql: impl Into<String>, args: Vec<Cell>) -> Self {
        Self {
            sql: sql.into(),
            args,
        }
    }
    fn encode(&self) -> Value {
        json!({"sql":self.sql,"args":self.args.iter().map(Cell::encode).collect::<Vec<_>>()})
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StmtResult {
    pub cols: Vec<String>,
    pub rows: Vec<Vec<Cell>>,
    pub affected_row_count: u64,
    /// Turso's billing counters, when the server reports them (scanned rows, not returned rows).
    pub rows_read: Option<u64>,
    pub rows_written: Option<u64>,
}
fn parse_stmt_result(value: &Value) -> Result<StmtResult, RemoteError> {
    let cols = value
        .get("cols")
        .and_then(Value::as_array)
        .ok_or_else(|| RemoteError::sql("missing cols"))?
        .iter()
        .map(|c| {
            c.get("name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned()
        })
        .collect();
    let rows = value
        .get("rows")
        .and_then(Value::as_array)
        .ok_or_else(|| RemoteError::sql("missing rows"))?
        .iter()
        .map(|r| {
            r.as_array()
                .ok_or_else(|| RemoteError::sql("invalid row"))?
                .iter()
                .map(Cell::decode)
                .collect::<Result<Vec<_>, _>>()
        })
        .collect::<Result<Vec<_>, _>>()?;
    let counter = |key: &str| {
        value.get(key).and_then(|v| {
            v.as_u64()
                .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
        })
    };
    let count = counter("affected_row_count")
        .ok_or_else(|| RemoteError::sql("missing affected row count"))?;
    Ok(StmtResult {
        cols,
        rows,
        affected_row_count: count,
        rows_read: counter("rows_read"),
        rows_written: counter("rows_written"),
    })
}

/// A batch outcome retains skipped steps so the live probe can inspect server semantics.
#[derive(Debug)]
pub enum StepOutcome {
    Ok(StmtResult),
    Error(RemoteError),
    Skipped,
}

pub struct RemoteDb {
    url: String,
    token: String,
    transport: Box<dyn Transport>,
    sleeper: Box<dyn Fn(Duration) + Send + Sync>,
}
impl fmt::Debug for RemoteDb {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RemoteDb")
            .field("url", &self.url)
            .field("token", &"<redacted>")
            .finish()
    }
}
impl RemoteDb {
    pub fn new(
        url: &str,
        token: impl Into<String>,
        transport: Box<dyn Transport>,
    ) -> Result<Self, RemoteError> {
        let base = ["turso://", "libsql://", "https://"]
            .iter()
            .find_map(|scheme| url.strip_prefix(scheme))
            .filter(|rest| !rest.is_empty() && !rest.starts_with('/'))
            .ok_or_else(|| RemoteError::sql("unsupported URL scheme"))?;
        Ok(Self {
            url: format!("https://{}/v2/pipeline", base.trim_end_matches('/')),
            token: token.into(),
            transport,
            sleeper: Box::new(std::thread::sleep),
        })
    }
    pub fn with_sleeper(mut self, sleeper: impl Fn(Duration) + Send + Sync + 'static) -> Self {
        self.sleeper = Box::new(sleeper);
        self
    }
    pub fn pipeline_url(&self) -> &str {
        &self.url
    }
    fn post(&self, requests: Vec<Value>) -> Result<Value, RemoteError> {
        let body = serde_json::to_vec(&json!({"requests":requests}))
            .map_err(|_| RemoteError::sql("request encode failed"))?;
        let auth = format!("Bearer {}", self.token);
        let raw = self.transport.post(
            &self.url,
            &body,
            &[
                ("Authorization", &auth),
                ("Content-Type", "application/json"),
            ],
        )?;
        serde_json::from_slice(&raw).map_err(|_| RemoteError::sql("invalid JSON response"))
    }
    pub fn execute(&self, sql: &str, args: Vec<Cell>) -> Result<StmtResult, RemoteError> {
        let mut results = self.pipeline(&[Stmt::new(sql, args)])?;
        results
            .pop()
            .ok_or_else(|| RemoteError::sql("missing execute result"))
    }
    pub fn pipeline(&self, stmts: &[Stmt]) -> Result<Vec<StmtResult>, RemoteError> {
        let mut requests: Vec<Value> = stmts
            .iter()
            .map(|s| json!({"type":"execute","stmt":s.encode()}))
            .collect();
        requests.push(json!({"type":"close"}));
        let payload = self.post(requests)?;
        let entries = payload
            .get("results")
            .and_then(Value::as_array)
            .ok_or_else(|| RemoteError::sql("missing pipeline results"))?;
        if entries.len() != stmts.len() + 1 {
            return Err(RemoteError::sql("pipeline result count mismatch"));
        }
        let mut out = Vec::with_capacity(stmts.len());
        for (index, entry) in entries.iter().take(stmts.len()).enumerate() {
            if entry.get("type").and_then(Value::as_str) == Some("error") {
                return Err(hrana_error(&entry["error"]).at(index));
            }
            if entry.get("type").and_then(Value::as_str) != Some("ok") {
                return Err(RemoteError::sql("invalid pipeline entry").at(index));
            }
            out.push(parse_stmt_result(&entry["response"]["result"]).map_err(|e| e.at(index))?);
        }
        let close = &entries[stmts.len()];
        if !matches!(
            close.get("type").and_then(Value::as_str),
            Some("ok" | "error")
        ) {
            return Err(RemoteError::sql("invalid close result").at(stmts.len()));
        }
        Ok(out)
    }
    fn batch_body(stmts: &[Stmt]) -> Value {
        let mut steps = vec![json!({"stmt":Stmt::new("BEGIN TRANSACTION", vec![]).encode()})];
        for (index, stmt) in stmts.iter().enumerate() {
            steps.push(json!({"stmt":stmt.encode(),"condition":{"type":"ok","step":index}}));
        }
        steps.push(json!({"stmt":Stmt::new("COMMIT", vec![]).encode(),"condition":{"type":"ok","step":stmts.len()}}));
        steps.push(json!({"stmt":Stmt::new("ROLLBACK", vec![]).encode(),"condition":{"type":"not","cond":{"type":"ok","step":stmts.len()+1}}}));
        json!({"steps":steps})
    }
    /// Only Busy is retried automatically by `transaction`. After Timeout or Unreachable the
    /// outcome is unknown and may have committed, so every statement set must be re-runnable.
    pub fn transaction_steps(&self, stmts: &[Stmt]) -> Result<Vec<StepOutcome>, RemoteError> {
        let payload = self.post(vec![
            json!({"type":"batch","batch":Self::batch_body(stmts)}),
            json!({"type":"close"}),
        ])?;
        let entries = payload
            .get("results")
            .and_then(Value::as_array)
            .ok_or_else(|| RemoteError::sql("missing batch results"))?;
        if entries.len() != 2 {
            return Err(RemoteError::sql("batch result count mismatch"));
        }
        let entry = entries
            .first()
            .ok_or_else(|| RemoteError::sql("missing batch entry"))?;
        if entry["type"] == "error" {
            return Err(hrana_error(&entry["error"]));
        }
        let batch = entry
            .get("response")
            .and_then(|r| r.get("result"))
            .ok_or_else(|| RemoteError::sql("missing batch result"))?;
        let results = batch
            .get("step_results")
            .and_then(Value::as_array)
            .ok_or_else(|| RemoteError::sql("missing step results"))?;
        let errors = batch
            .get("step_errors")
            .and_then(Value::as_array)
            .ok_or_else(|| RemoteError::sql("missing step errors"))?;
        if results.len() != stmts.len() + 3 || errors.len() != results.len() {
            return Err(RemoteError::sql("batch step count mismatch"));
        }
        let outcomes: Vec<StepOutcome> = results
            .iter()
            .zip(errors)
            .enumerate()
            .map(|(i, (result, error))| {
                if !error.is_null() {
                    Ok(StepOutcome::Error(hrana_error(error).at(i)))
                } else if result.is_null() {
                    Ok(StepOutcome::Skipped)
                } else {
                    parse_stmt_result(result)
                        .map(StepOutcome::Ok)
                        .map_err(|e| e.at(i))
                }
            })
            .collect::<Result<_, _>>()?;
        if entries[1]["type"] == "error"
            && !matches!(outcomes.get(stmts.len() + 1), Some(StepOutcome::Ok(_)))
            && !outcomes
                .iter()
                .any(|outcome| matches!(outcome, StepOutcome::Error(_)))
        {
            return Err(hrana_error(&entries[1]["error"]).at(stmts.len() + 3));
        }
        if entries[1]["type"] != "ok" && entries[1]["type"] != "error" {
            return Err(RemoteError::sql("invalid batch close result"));
        }
        Ok(outcomes)
    }
    /// Retries only Busy. After Timeout or Unreachable the outcome is unknown and may have
    /// committed; every statement set passed here must be re-runnable.
    pub fn transaction(&self, stmts: &[Stmt]) -> Result<Vec<StmtResult>, RemoteError> {
        for attempt in 0..=3 {
            let result = self.transaction_once(stmts);
            match result {
                Err(e) if e.kind == ErrorKind::Busy && attempt < 3 => {
                    (self.sleeper)(Duration::from_millis([100, 300, 900][attempt]))
                }
                other => return other,
            }
        }
        Err(RemoteError::sql("retry loop exhausted"))
    }
    fn transaction_once(&self, stmts: &[Stmt]) -> Result<Vec<StmtResult>, RemoteError> {
        let outcomes = self.transaction_steps(stmts)?;
        // First error in step order wins, even if rollback also fails.
        for outcome in &outcomes {
            if let StepOutcome::Error(error) = outcome {
                return Err(error.clone());
            }
        }
        if !matches!(outcomes.get(stmts.len() + 1), Some(StepOutcome::Ok(_))) {
            return Err(RemoteError::sql("commit skipped"));
        }
        outcomes
            .into_iter()
            .skip(1)
            .take(stmts.len())
            .enumerate()
            .map(|(i, outcome)| match outcome {
                StepOutcome::Ok(result) => Ok(result),
                _ => Err(RemoteError::sql("statement skipped").at(i + 1)),
            })
            .collect()
    }
}

/// In-memory Hrana server for integration tests; it executes the posted JSON. Its `rows_read`
/// (returned rows) and `rows_written` (affected rows) counters are approximations.
pub struct SqliteTransport {
    conn: std::sync::Mutex<rusqlite::Connection>,
    failures: std::sync::Mutex<Vec<InjectedFailure>>,
}
#[derive(Clone)]
enum InjectedFailure {
    Status(u16),
    Code(String),
    Step(usize, String),
    Cell(Value),
    Close(String),
}
impl SqliteTransport {
    pub fn new() -> Result<Self, RemoteError> {
        Ok(Self {
            conn: std::sync::Mutex::new(
                rusqlite::Connection::open_in_memory()
                    .map_err(|_| RemoteError::sql("SQLite open failed"))?,
            ),
            failures: std::sync::Mutex::new(Vec::new()),
        })
    }
    pub fn fail_next_status(&self, status: u16) {
        if let Ok(mut q) = self.failures.lock() {
            q.push(InjectedFailure::Status(status));
        }
    }
    pub fn fail_next_code(&self, code: &str) {
        if let Ok(mut q) = self.failures.lock() {
            q.push(InjectedFailure::Code(code.to_owned()));
        }
    }
    pub fn fail_next_step(&self, step: usize, code: &str) {
        if let Ok(mut q) = self.failures.lock() {
            q.push(InjectedFailure::Step(step, code.to_owned()));
        }
    }
    pub fn return_next_cell(&self, cell: Value) {
        if let Ok(mut q) = self.failures.lock() {
            q.push(InjectedFailure::Cell(cell));
        }
    }
    pub fn fail_next_close(&self, code: &str) {
        if let Ok(mut q) = self.failures.lock() {
            q.push(InjectedFailure::Close(code.to_owned()));
        }
    }
}
fn sqlite_error(error: &rusqlite::Error) -> Value {
    let code = match error {
        rusqlite::Error::SqliteFailure(e, _) => match e.code {
            rusqlite::ErrorCode::DatabaseBusy => "SQLITE_BUSY",
            rusqlite::ErrorCode::ConstraintViolation => match e.extended_code {
                rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE => "SQLITE_CONSTRAINT_UNIQUE",
                rusqlite::ffi::SQLITE_CONSTRAINT_PRIMARYKEY => "SQLITE_CONSTRAINT_PRIMARYKEY",
                rusqlite::ffi::SQLITE_CONSTRAINT_FOREIGNKEY => "SQLITE_CONSTRAINT_FOREIGNKEY",
                rusqlite::ffi::SQLITE_CONSTRAINT_NOTNULL => "SQLITE_CONSTRAINT_NOTNULL",
                rusqlite::ffi::SQLITE_CONSTRAINT_CHECK => "SQLITE_CONSTRAINT_CHECK",
                _ => "SQLITE_CONSTRAINT",
            },
            _ => "SQLITE_ERROR",
        },
        _ => "SQLITE_ERROR",
    };
    json!({"code":code,"message":error.to_string()})
}
fn sqlite_stmt(conn: &rusqlite::Connection, stmt: &Value) -> Result<Value, Value> {
    use rusqlite::types::{Value as SqlValue, ValueRef};
    let sql = stmt
        .get("sql")
        .and_then(Value::as_str)
        .ok_or_else(|| json!({"code":"SQLITE_ERROR","message":"missing SQL"}))?;
    let args = stmt
        .get("args")
        .and_then(Value::as_array)
        .ok_or_else(|| json!({"code":"SQLITE_ERROR","message":"missing args"}))?;
    let params: Vec<SqlValue> = args
        .iter()
        .map(|v| match Cell::decode(v) {
            Ok(Cell::Null) => Ok(SqlValue::Null),
            Ok(Cell::Int(n)) => Ok(SqlValue::Integer(n)),
            Ok(Cell::Text(s)) => Ok(SqlValue::Text(s)),
            Err(_) => Err(json!({"code":"SQLITE_ERROR","message":"invalid cell"})),
        })
        .collect::<Result<_, _>>()?;
    let before = conn.total_changes();
    let mut prepared = conn.prepare(sql).map_err(|e| sqlite_error(&e))?;
    let cols: Vec<Value> = prepared
        .column_names()
        .iter()
        .map(|name| json!({"name":name}))
        .collect();
    let mut rows = Vec::new();
    let mut cursor = prepared
        .query(rusqlite::params_from_iter(params))
        .map_err(|e| sqlite_error(&e))?;
    while let Some(row) = cursor.next().map_err(|e| sqlite_error(&e))? {
        let mut cells = Vec::new();
        for i in 0..cols.len() {
            cells.push(match row.get_ref(i).map_err(|e| sqlite_error(&e))? {
                ValueRef::Null => Cell::Null.encode(),
                ValueRef::Integer(n) => Cell::Int(n).encode(),
                ValueRef::Text(b) => Cell::Text(String::from_utf8_lossy(b).into_owned()).encode(),
                _ => {
                    return Err(json!({"code":"SQLITE_ERROR","message":"unsupported SQLite cell"}))
                }
            });
        }
        rows.push(cells);
    }
    let rows_read = rows.len();
    let rows_written = conn.total_changes() - before;
    Ok(
        json!({"cols":cols,"rows":rows,"affected_row_count":rows_written,"rows_read":rows_read,"rows_written":rows_written}),
    )
}
impl Transport for SqliteTransport {
    fn post(
        &self,
        _url: &str,
        body: &[u8],
        _headers: &[(&str, &str)],
    ) -> Result<Vec<u8>, RemoteError> {
        let request: Value =
            serde_json::from_slice(body).map_err(|_| RemoteError::sql("invalid request JSON"))?;
        let requests = request["requests"]
            .as_array()
            .ok_or_else(|| RemoteError::sql("missing requests"))?;
        let failure = self
            .failures
            .lock()
            .map_err(|_| RemoteError::sql("failure queue poisoned"))?
            .is_empty();
        let injected = if failure {
            None
        } else {
            Some(
                self.failures
                    .lock()
                    .map_err(|_| RemoteError::sql("failure queue poisoned"))?
                    .remove(0),
            )
        };
        let single_execute = requests.len() == 2
            && requests[0]["type"] == "execute"
            && requests[1]["type"] == "close";
        let single_batch =
            requests.len() == 2 && requests[0]["type"] == "batch" && requests[1]["type"] == "close";
        match injected.as_ref() {
            Some(InjectedFailure::Code(_) | InjectedFailure::Cell(_)) => assert!(
                single_execute,
                "injected failure does not match request shape"
            ),
            Some(InjectedFailure::Step(index, _)) => {
                assert!(
                    single_batch
                        && requests[0]["batch"]["steps"]
                            .as_array()
                            .is_some_and(|steps| *index < steps.len()),
                    "injected failure does not match request shape"
                );
            }
            Some(InjectedFailure::Close(_)) => assert!(
                requests.last().is_some_and(|r| r["type"] == "close"),
                "injected failure does not match request shape"
            ),
            _ => {}
        }
        // Locked only after the shape checks, so a mismatch panic cannot poison the connection.
        let conn = self
            .conn
            .lock()
            .map_err(|_| RemoteError::sql("SQLite lock poisoned"))?;
        if let Some(InjectedFailure::Status(status)) = injected.as_ref() {
            return Err(RemoteError::new(
                classify(None, None, Some(*status), None),
                format!("HTTP {status}"),
            ));
        }
        if let Some(InjectedFailure::Code(code)) = injected.as_ref() {
            return serde_json::to_vec(&json!({"results":[{"type":"error","error":{"code":code,"message":code},"response":null},{"type":"ok","response":null}]})).map_err(|_| RemoteError::sql("test response encode failed"));
        }
        if let Some(InjectedFailure::Cell(cell)) = injected.as_ref() {
            return serde_json::to_vec(&json!({"results":[{"type":"ok","response":{"type":"execute","result":{"cols":[{"name":"x"}],"rows":[[cell]],"affected_row_count":0}}},{"type":"ok","response":null}]})).map_err(|_| RemoteError::sql("test response encode failed"));
        }
        let mut results = Vec::new();
        for request in requests {
            match request["type"].as_str() {
                Some("close") => {
                    if let Some(InjectedFailure::Close(code)) = injected.as_ref() {
                        results.push(json!({"type":"error","error":{"code":code,"message":code},"response":null}));
                        continue;
                    }
                    if !conn.is_autocommit() {
                        match conn.execute_batch("ROLLBACK") {
                            Ok(()) => results.push(json!({"type":"ok","response":null})),
                            Err(error) => results.push(json!({"type":"error","error":sqlite_error(&error),"response":null})),
                        }
                    } else {
                        results.push(json!({"type":"ok","response":null}));
                    }
                }
                Some("execute") => match sqlite_stmt(&conn, &request["stmt"]) {
                    Ok(result) => results
                        .push(json!({"type":"ok","response":{"type":"execute","result":result}})),
                    Err(error) => {
                        results.push(json!({"type":"error","error":error,"response":null}))
                    }
                },
                Some("batch") => {
                    let steps = request["batch"]["steps"]
                        .as_array()
                        .ok_or_else(|| RemoteError::sql("missing batch steps"))?;
                    let mut step_results: Vec<Value> = Vec::new();
                    let mut step_errors: Vec<Value> = Vec::new();
                    for (i, step) in steps.iter().enumerate() {
                        let runs = match step.get("condition") {
                            None => true,
                            Some(cond) if cond["type"] == "ok" => cond["step"]
                                .as_u64()
                                .and_then(|j| {
                                    step_results
                                        .get(j as usize)
                                        .zip(step_errors.get(j as usize))
                                })
                                .is_some_and(|(r, e)| !r.is_null() && e.is_null()),
                            Some(cond) if cond["type"] == "not" => !cond["cond"]["step"]
                                .as_u64()
                                .and_then(|j| {
                                    step_results
                                        .get(j as usize)
                                        .zip(step_errors.get(j as usize))
                                })
                                .is_some_and(|(r, e)| !r.is_null() && e.is_null()),
                            _ => return Err(RemoteError::sql("unsupported batch condition")),
                        };
                        let outcome = if !runs {
                            None
                        } else if let Some(InjectedFailure::Step(index, code)) = injected.as_ref() {
                            if *index == i {
                                Some(Err(json!({"code":code,"message":code})))
                            } else {
                                Some(sqlite_stmt(&conn, &step["stmt"]))
                            }
                        } else {
                            Some(sqlite_stmt(&conn, &step["stmt"]))
                        };
                        match outcome {
                            Some(Ok(result)) => {
                                step_results.push(result);
                                step_errors.push(Value::Null);
                            }
                            Some(Err(error)) => {
                                step_results.push(Value::Null);
                                step_errors.push(error);
                            }
                            None => {
                                step_results.push(Value::Null);
                                step_errors.push(Value::Null);
                            }
                        }
                    }
                    // A failed BEGIN leaves no transaction; a successful BEGIN must be closed.
                    if !conn.is_autocommit() {
                        let _ = conn.execute_batch("ROLLBACK");
                    }
                    results.push(json!({"type":"ok","response":{"type":"batch","result":{"step_results":step_results,"step_errors":step_errors}}}));
                }
                _ => return Err(RemoteError::sql("unsupported request type")),
            }
        }
        serde_json::to_vec(&json!({"results":results}))
            .map_err(|_| RemoteError::sql("test response encode failed"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn network_recorder_phase_order_and_frozen_return() {
        let recorder = NetworkRecorder::new();
        assert_eq!(recorder.snapshot_at(1), None);
        for (phase, (name, start)) in [
            ("dns", 1),
            ("connect", 120_000_001),
            ("tls", 160_000_001),
            ("response", 460_000_001),
        ]
        .into_iter()
        .enumerate()
        {
            recorder.boundaries[phase].store(start, Ordering::Release);
            let snapshot = recorder.snapshot_at(start + 35_000_000).unwrap();
            assert_eq!(snapshot["phase"], name);
            assert_eq!(snapshot[format!("{name}_ms")], 35);
            assert_eq!(snapshot.as_object().unwrap().len(), phase + 2);
        }
        recorder.stopped.store(510_000_001, Ordering::Release);
        let frozen = json!({"phase":"response", "dns_ms":120, "connect_ms":40, "tls_ms":300, "response_ms":50});
        assert_eq!(recorder.snapshot_at(900_000_001), Some(frozen.clone()));
        assert_eq!(recorder.snapshot_at(9_000_000_001), Some(frozen));
        recorder.reset();
        assert_eq!(recorder.snapshot(), None);
    }

    #[test]
    fn network_recorder_budget_and_torn_reads() {
        let recorder = NetworkRecorder::new();
        // Budget spent before any request: the phase alone.
        recorder.mark_budget();
        assert_eq!(recorder.snapshot_at(5), Some(json!({"phase": "budget"})));
        // Budget spent after a completed request keeps that request's timings.
        recorder.reset();
        for (phase, start) in [1, 100_000_001, 200_000_001, 300_000_001]
            .into_iter()
            .enumerate()
        {
            recorder.boundaries[phase].store(start, Ordering::Release);
        }
        recorder.stopped.store(400_000_001, Ordering::Release);
        recorder.mark_budget();
        assert_eq!(
            recorder.snapshot_at(900_000_001),
            Some(
                json!({"phase":"budget","dns_ms":100,"connect_ms":100,"tls_ms":100,"response_ms":100})
            )
        );
        // A reset caught halfway (older boundaries cleared, a newer one not yet) reports nothing.
        recorder.reset();
        recorder.boundaries[3].store(300_000_001, Ordering::Release);
        assert_eq!(recorder.snapshot_at(900_000_001), None);
    }

    #[test]
    fn network_recorder_snapshot_from_another_thread_mid_phase() {
        let recorder = Arc::new(NetworkRecorder::new());
        let worker_recorder = Arc::clone(&recorder);
        let (entered, waiting) = std::sync::mpsc::channel();
        let (resume, released) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            for phase in 0..=2 {
                worker_recorder.enter(phase);
            }
            entered.send(()).unwrap();
            released.recv().unwrap();
            worker_recorder.enter(3);
            worker_recorder.stop();
        });
        waiting.recv().unwrap();
        let snapshot = recorder.snapshot().unwrap();
        assert_eq!(snapshot["phase"], "tls");
        for phase in ["dns_ms", "connect_ms", "tls_ms"] {
            assert!(snapshot[phase].is_u64());
        }
        assert!(snapshot.get("response_ms").is_none());
        resume.send(()).unwrap();
        worker.join().unwrap();
        assert_eq!(recorder.snapshot().unwrap()["phase"], "response");
    }

    #[test]
    fn network_recorder_tls_timeout_without_sockets() {
        #[derive(Debug)]
        struct SilentPeer;
        impl std::io::Read for SilentPeer {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "silent peer",
                ))
            }
        }
        impl std::io::Write for SilentPeer {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        impl ureq::ReadWrite for SilentPeer {
            fn socket(&self) -> Option<&std::net::TcpStream> {
                None
            }
        }

        NETWORK.reset();
        let recording = NetworkRequest;
        NETWORK.enter(0);
        NETWORK.enter(1);
        let error =
            ureq::TlsConnector::connect(tls().unwrap().as_ref(), "localhost", Box::new(SilentPeer))
                .unwrap_err();
        drop(recording);
        let snapshot = network_snapshot().unwrap();
        assert_eq!(snapshot["phase"], "tls");
        for phase in ["dns_ms", "connect_ms", "tls_ms"] {
            assert!(snapshot[phase].is_u64(), "{snapshot}");
        }
        assert!(snapshot.get("response_ms").is_none());
        assert_eq!(
            std::error::Error::source(&error)
                .unwrap()
                .downcast_ref::<std::io::Error>()
                .unwrap()
                .kind(),
            std::io::ErrorKind::TimedOut
        );
    }

    #[test]
    fn classification_table() {
        for (code, message, status, expected) in [
            (None, None, Some(401), ErrorKind::Unauthorized),
            (None, None, Some(403), ErrorKind::Unauthorized),
            (
                Some("AUTH_JWT_EXPIRED"),
                None,
                Some(401),
                ErrorKind::Unauthorized,
            ),
            (Some("BLOCKED"), None, Some(403), ErrorKind::Blocked),
            (Some("SQLITE_ERROR"), None, Some(503), ErrorKind::Sql),
            (Some("BLOCKED"), None, None, ErrorKind::Blocked),
            (None, Some("quota blocked"), None, ErrorKind::Blocked),
            (Some("SQLITE_BUSY"), None, None, ErrorKind::Busy),
            (
                Some("SQLITE_LOCKED_SHAREDCACHE"),
                None,
                None,
                ErrorKind::Busy,
            ),
            (None, Some("SQLITE_BUSY: locked"), None, ErrorKind::Busy),
            (
                Some("SQLITE_CONSTRAINT_PRIMARYKEY"),
                None,
                None,
                ErrorKind::Constraint,
            ),
            (Some("SQLITE_ERROR"), None, None, ErrorKind::Sql),
            (
                Some("SQLITE_ERROR"),
                Some("no such table: blocked_users"),
                None,
                ErrorKind::Sql,
            ),
        ] {
            assert_eq!(classify(code, message, status, None), expected);
        }
        assert_eq!(
            classify(None, None, None, Some(ErrorKind::Unreachable)),
            ErrorKind::Unreachable
        );
        assert_eq!(
            classify(None, None, None, Some(ErrorKind::Timeout)),
            ErrorKind::Timeout
        );
    }
    #[test]
    fn batch_shape() {
        let body =
            RemoteDb::batch_body(&[Stmt::new("SELECT 1", vec![]), Stmt::new("SELECT 2", vec![])]);
        assert_eq!(body["steps"][0]["stmt"]["sql"], "BEGIN TRANSACTION");
        assert_eq!(body["steps"][1]["condition"], json!({"type":"ok","step":0}));
        assert_eq!(body["steps"][2]["condition"], json!({"type":"ok","step":1}));
        assert_eq!(body["steps"][3]["condition"], json!({"type":"ok","step":2}));
        assert_eq!(
            body["steps"][4]["condition"],
            json!({"type":"not","cond":{"type":"ok","step":3}})
        );
    }
    #[test]
    fn http_status_table() {
        for (status, body, expected) in [
            (401, "{}", ErrorKind::Unauthorized),
            (403, "{}", ErrorKind::Unauthorized),
            (
                403,
                r#"{"results":[{"type":"error","error":{"code":"BLOCKED","message":"secret"}}]}"#,
                ErrorKind::Blocked,
            ),
            (408, "{}", ErrorKind::Timeout),
            (429, "{}", ErrorKind::Unreachable),
            (500, "{}", ErrorKind::Unreachable),
            (502, "{}", ErrorKind::Unreachable),
            (503, "{}", ErrorKind::Unreachable),
            (504, "{}", ErrorKind::Timeout),
            (418, "{}", ErrorKind::Sql),
        ] {
            let error = status_error(status, body.as_bytes());
            assert_eq!(error.kind, expected, "status {status}");
            assert!(!error.detail().contains("secret"));
        }
        assert_eq!(
            status_error(403, br#"{"error":"denied","code":"BLOCKED"}"#).kind,
            ErrorKind::Blocked
        );
        assert_eq!(
            status_error(403, br#"{"error":{"code":"BLOCKED","message":"secret"}}"#).kind,
            ErrorKind::Blocked
        );
    }
}
