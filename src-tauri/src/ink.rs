//! The e-ink endpoint: API v1 for the PocketBook app in `ink/`, served on the laptop's Wi-Fi address while a pairing
//! token exists. Every answer comes from the same session and row operations the list window uses.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::net::Ipv4Addr;
use std::sync::Mutex;
use std::time::Duration;

use base64::Engine;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::watch;

use crate::group::{self, Counts, ReviewRow};
use crate::score::{CommentStats, Provider, ReviewState, Tier};
use crate::session::Session;
use crate::store::Secrets;

pub const PORT: u16 = 7878;
const TOKEN_KEY: &str = "ink-token";
const MAX_REQUEST: usize = 16 * 1024;
const READ_TIMEOUT: Duration = Duration::from_secs(10);
/// How often the listener checks whether the Wi-Fi address changed, and retries after it couldn't listen.
const ADDRESS_CHECK: Duration = Duration::from_secs(60);

/// What the device gets from `GET /v1/list`: the active workspace's list, without links, settings or secrets.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct List {
    workspace: String,
    counts: Counts,
    last_refreshed: Option<DateTime<Utc>>,
    polling: bool,
    error: Option<String>,
    rows: Vec<Row>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Row {
    key: String,
    tab: Tab,
    tier: Tier,
    title: String,
    ticket: Option<String>,
    age_hours: i64,
    approvals: u32,
    objections: u32,
    comments: Option<CommentStats>,
    waiting: bool,
    /// The row holds a PR of mine, so hiding it says "Hide" rather than "Not mine".
    holds_mine: bool,
    draft: Option<Draft>,
    prs: Vec<Pr>,
}

/// The list window's tabs, keyed as in `src/tabs.ts`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
enum Tab {
    NeedsReview,
    ReReview,
    Mine,
    WaitingOnAuthor,
    Approved,
    Stale,
    Hidden,
}

#[derive(Debug, Serialize)]
struct Draft {
    linked: bool,
}

#[derive(Debug, Serialize)]
struct Pr {
    repo: String,
    number: i64,
    provider: Provider,
}

fn tab(row: &ReviewRow) -> Tab {
    if row.summary.hidden {
        return Tab::Hidden;
    }
    match row.summary.state {
        ReviewState::NeedsReview => Tab::NeedsReview,
        ReviewState::ReReview => Tab::ReReview,
        ReviewState::Mine => Tab::Mine,
        ReviewState::WaitingOnAuthor => Tab::WaitingOnAuthor,
        ReviewState::Approved => Tab::Approved,
        ReviewState::Stale => Tab::Stale,
    }
}

impl From<ReviewRow> for Row {
    fn from(row: ReviewRow) -> Self {
        let drafts: Vec<_> = row.members.iter().filter_map(|m| m.agent_draft.as_ref()).collect();
        Self {
            tab: tab(&row),
            // As the list window draws it.
            tier: if row.summary.hidden { Tier::Low } else { row.summary.tier },
            title: row.summary.title,
            ticket: row.ticket,
            age_hours: row.summary.age_hours,
            approvals: row.summary.approvals,
            objections: row.summary.objections,
            comments: row.summary.comments,
            waiting: row.summary.actionable,
            holds_mine: row.members.iter().any(|m| m.state == Some(ReviewState::Mine)),
            draft: (!drafts.is_empty()).then(|| Draft { linked: drafts.iter().any(|d| d.host.is_some()) }),
            prs: row.members.into_iter().map(|m| Pr { repo: m.repo, number: m.number, provider: m.provider }).collect(),
            key: row.key,
        }
    }
}

pub fn list(session: &Session) -> List {
    let status = session.status();
    let mut rows = status.items;
    // The list window shows its Mine tab newest first (`newestFirst` in `src/format.ts`); the other rows keep their places.
    let slots: Vec<usize> = (0..rows.len()).filter(|&i| tab(&rows[i]) == Tab::Mine).collect();
    let mut mine: Vec<ReviewRow> = slots.iter().map(|&i| rows[i].clone()).collect();
    mine.sort_by_key(|r| r.newest_age_hours);
    for (slot, row) in slots.into_iter().zip(mine) {
        rows[slot] = row;
    }
    List {
        workspace: session.settings().active().map(|w| w.name.clone()).unwrap_or_default(),
        counts: status.counts,
        last_refreshed: status.last_refreshed,
        polling: status.polling,
        error: status.error.as_deref().map(without_links),
        rows: rows.into_iter().map(Row::from).collect(),
    }
}

/// A source error can quote the URL a request went to.
fn without_links(text: &str) -> String {
    text.split(' ')
        .map(|word| match word.find("http") {
            Some(at) if word[at..].contains("://") => &word[..at],
            _ => word,
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Rado as the endpoint sees it. The app runs `change` through the same path as its commands, so windows get the
/// new snapshot and the poller wakes when asked, and `open` opens links in the browser chosen in settings.
pub trait Rado {
    fn read<T>(&self, f: impl FnOnce(&Session) -> T) -> T;
    fn change<T>(&self, f: impl FnOnce(&mut Session) -> T) -> T;
    fn open(&self, urls: Vec<String>) -> Result<(), String>;
}

#[derive(Debug, Default)]
pub struct Request {
    pub method: String,
    pub path: String,
    pub authorization: Option<String>,
    pub if_none_match: Option<String>,
    pub body: Vec<u8>,
}

#[derive(Debug, PartialEq)]
pub struct Response {
    pub status: u16,
    pub etag: Option<String>,
    pub body: Option<String>,
}

impl Response {
    fn empty(status: u16) -> Self {
        Self { status, etag: None, body: None }
    }

    fn error(status: u16, text: &str) -> Self {
        Self { status, etag: None, body: Some(serde_json::json!({ "error": text }).to_string()) }
    }
}

#[derive(Deserialize)]
struct RowAction {
    key: String,
}

#[derive(Deserialize)]
struct HiddenAction {
    key: String,
    hidden: bool,
}

/// Answers one request. `token` is the pairing token, `None` once it was removed.
pub fn handle(rado: &impl Rado, token: Option<&str>, request: &Request) -> Response {
    let presented = request.authorization.as_deref().and_then(|a| a.strip_prefix("Bearer "));
    if !matches!((token, presented), (Some(token), Some(presented)) if same(token, presented)) {
        return Response::error(401, "unauthorized");
    }
    let path = request.path.split('?').next().unwrap_or_default();
    match (request.method.as_str(), path) {
        ("GET", "/v1/list") => {
            let body = serde_json::to_string(&rado.read(list)).expect("serializable");
            let mut hasher = DefaultHasher::new();
            body.hash(&mut hasher);
            let etag = format!("\"{:016x}\"", hasher.finish());
            if request.if_none_match.as_deref() == Some(etag.as_str()) {
                return Response { status: 304, etag: Some(etag), body: None };
            }
            Response { status: 200, etag: Some(etag), body: Some(body) }
        }
        ("POST", "/v1/refresh") => {
            rado.change(|s| s.refresh(Utc::now()));
            Response::empty(204)
        }
        ("POST", "/v1/open") => match body::<RowAction>(request) {
            Ok(a) => on_row(rado, &a.key, |row| rado.open(group::open_urls(&row))),
            Err(r) => r,
        },
        ("POST", "/v1/ticket") => match body::<RowAction>(request) {
            Ok(a) => on_row(rado, &a.key, |_| rado.read(|s| s.ticket_url(&a.key)).and_then(|url| rado.open(vec![url]))),
            Err(r) => r,
        },
        ("POST", "/v1/draft") => match body::<RowAction>(request) {
            Ok(a) => on_row(rado, &a.key, |row| rado.read(|s| draft_urls(s, &row)).and_then(|urls| rado.open(urls))),
            Err(r) => r,
        },
        ("POST", "/v1/hidden") => match body::<HiddenAction>(request) {
            Ok(a) => on_row(rado, &a.key, |_| rado.change(|s| s.set_hidden(&a.key, a.hidden))),
            Err(r) => r,
        },
        _ => Response::error(404, "not found"),
    }
}

fn body<T: serde::de::DeserializeOwned>(request: &Request) -> Result<T, Response> {
    serde_json::from_slice(&request.body).map_err(|e| Response::error(400, &format!("bad request: {e}")))
}

/// Runs an action on a row: 410 when the row is gone, so the device reloads; 409 with Rado's text when it fails.
fn on_row(rado: &impl Rado, key: &str, act: impl FnOnce(ReviewRow) -> Result<(), String>) -> Response {
    match rado.read(|s| s.row(key)).map(act) {
        Err(e) => Response::error(410, &e),
        Ok(Err(e)) => Response::error(409, &e),
        Ok(Ok(())) => Response::empty(204),
    }
}

/// Every linked draft among the row's PRs; without one, the session's reason.
fn draft_urls(s: &Session, row: &ReviewRow) -> Result<Vec<String>, String> {
    let found: Vec<Result<String, String>> = row.members.iter().map(|m| s.draft_url(m.id)).collect();
    let urls: Vec<String> = found.iter().filter_map(|r| r.clone().ok()).collect();
    if urls.is_empty() {
        return Err(found.into_iter().find_map(Result::err).unwrap_or_default());
    }
    Ok(urls)
}

/// Compares in time that doesn't depend on where the strings differ.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Reads one whole request: headers, then `Content-Length` bytes of body. Answering before the client finished
/// sending makes Windows reset the connection, and the device sees "connection reset by peer".
async fn read_request(stream: &mut (impl AsyncRead + Unpin)) -> Result<Request, Response> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let n = stream.read(&mut chunk).await.map_err(|e| Response::error(400, &e.to_string()))?;
        if n == 0 {
            return Err(Response::error(400, "incomplete request"));
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.len() > MAX_REQUEST {
            return Err(Response::error(413, "request too large"));
        }
        let mut headers = [httparse::EMPTY_HEADER; 32];
        let mut parsed = httparse::Request::new(&mut headers);
        let head = match parsed.parse(&buf) {
            Ok(httparse::Status::Complete(head)) => head,
            Ok(httparse::Status::Partial) => continue,
            Err(e) => return Err(Response::error(400, &format!("bad request: {e}"))),
        };
        let header = |name: &str| {
            parsed.headers.iter().find(|h| h.name.eq_ignore_ascii_case(name)).map(|h| String::from_utf8_lossy(h.value).trim().to_string())
        };
        if header("Transfer-Encoding").is_some() {
            return Err(Response::error(400, "send a Content-Length instead of Transfer-Encoding"));
        }
        let length = match header("Content-Length") {
            None => 0,
            Some(n) => n.parse::<usize>().map_err(|_| Response::error(400, "bad Content-Length"))?,
        };
        // `head` is within `buf`, which is at most MAX_REQUEST long, so this can't underflow.
        if length > MAX_REQUEST - head {
            return Err(Response::error(413, "request too large"));
        }
        if buf.len() < head + length {
            continue;
        }
        return Ok(Request {
            method: parsed.method.unwrap_or_default().to_string(),
            path: parsed.path.unwrap_or_default().to_string(),
            authorization: header("Authorization"),
            if_none_match: header("If-None-Match"),
            body: buf[head..head + length].to_vec(),
        });
    }
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        204 => "No Content",
        304 => "Not Modified",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        409 => "Conflict",
        410 => "Gone",
        413 => "Content Too Large",
        _ => "Internal Server Error",
    }
}

fn encode(response: &Response) -> Vec<u8> {
    let body = response.body.as_deref().unwrap_or_default();
    let mut head = format!("HTTP/1.1 {} {}\r\nConnection: close\r\n", response.status, reason(response.status));
    if let Some(etag) = &response.etag {
        head.push_str(&format!("ETag: {etag}\r\n"));
    }
    if response.body.is_some() {
        head.push_str(&format!("Content-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\n", body.len()));
    }
    head.push_str("\r\n");
    [head.as_bytes(), body.as_bytes()].concat()
}

/// Reads a request, answers it and closes the connection.
async fn serve<R: Rado + Clone + Send + 'static>(stream: &mut (impl AsyncRead + AsyncWrite + Unpin), rado: R, token: Option<String>) {
    let response = match tokio::time::timeout(READ_TIMEOUT, read_request(stream)).await {
        Err(_) => return,
        Ok(Err(response)) => response,
        // Row actions start the browser and write files; keep them off the async threads.
        Ok(Ok(request)) => tokio::task::spawn_blocking(move || handle(&rado, token.as_deref(), &request))
            .await
            .unwrap_or_else(|_| Response::error(500, "Rado failed to answer")),
    };
    let _ = stream.write_all(&encode(&response)).await;
    let _ = stream.shutdown().await;
}

/// One connection at a time: there is one device, and it bounds what a stranger on the network can hold open.
async fn accept<R: Rado + Clone + Send + 'static>(listener: TcpListener, rado: R, token: watch::Receiver<Option<String>>) {
    loop {
        if let Ok((mut stream, _)) = listener.accept().await {
            let current = token.borrow().clone();
            serve(&mut stream, rado.clone(), current).await;
        }
    }
}

/// The pairing token, kept in Credential Manager and mirrored in memory for the listener.
pub struct Pairing {
    secrets: Box<dyn Secrets>,
    token: watch::Sender<Option<String>>,
    /// Why the device can't reach Rado while a token exists, or why the token couldn't be read.
    problem: Mutex<Option<String>>,
}

/// What Settings shows about pairing; the token itself never leaves Rust.
#[derive(Debug, Clone, PartialEq, Serialize, ts_rs::TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct InkStatus {
    paired: bool,
    /// The address the device uses, such as `192.168.1.14:7878`; `None` without a Wi-Fi connection.
    address: Option<String>,
    problem: Option<String>,
}

impl Pairing {
    pub fn load(secrets: Box<dyn Secrets>) -> Self {
        let (token, problem) = match secrets.read(TOKEN_KEY) {
            Ok(token) => (token, None),
            Err(e) => (None, Some(format!("Can't read the pairing token ({e})"))),
        };
        Self { secrets, token: watch::Sender::new(token), problem: Mutex::new(problem) }
    }

    pub fn status(&self) -> InkStatus {
        InkStatus {
            paired: self.token.borrow().is_some(),
            address: wifi_address().map(|ip| format!("{ip}:{PORT}")),
            problem: self.problem.lock().expect("pairing lock").clone(),
        }
    }

    /// Creates a token, replacing any earlier one, which stops working at once.
    pub fn pair(&self) -> Result<(), String> {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).map_err(|e| format!("Couldn't make a token: {e}"))?;
        let token = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
        self.secrets.set(TOKEN_KEY, &token)?;
        self.token.send_replace(Some(token));
        Ok(())
    }

    pub fn unpair(&self) -> Result<(), String> {
        self.secrets.delete(TOKEN_KEY)?;
        self.token.send_replace(None);
        self.report(None);
        Ok(())
    }

    /// The text of the device's config file, `system/config/rado.cfg` on the PocketBook.
    pub fn config(&self) -> Result<String, String> {
        let token = self.token.borrow().clone().ok_or("Pair a device first")?;
        let host = wifi_address().ok_or(NO_WIFI)?;
        Ok(format!("host={host}\nport={PORT}\ntoken={token}\n"))
    }

    fn report(&self, problem: Option<String>) {
        *self.problem.lock().expect("pairing lock") = problem;
    }

    /// Listens on the Wi-Fi address while a token exists, and moves when the address changes.
    pub async fn run<R: Rado + Clone + Send + 'static>(&self, rado: R) {
        let mut token = self.token.subscribe();
        loop {
            if token.borrow_and_update().is_none() {
                if token.changed().await.is_err() {
                    return;
                }
                continue;
            }
            let address = wifi_address();
            let bound = match address {
                None => Err(NO_WIFI.to_string()),
                Some(ip) => TcpListener::bind((ip, PORT)).await.map_err(|e| format!("Can't listen on {ip}:{PORT} ({e})")),
            };
            self.report(bound.as_ref().err().cloned());
            let serving = async {
                match bound {
                    Ok(listener) => accept(listener, rado.clone(), token.clone()).await,
                    Err(_) => tokio::time::sleep(ADDRESS_CHECK).await,
                }
            };
            let moved = async {
                loop {
                    tokio::time::sleep(ADDRESS_CHECK).await;
                    if wifi_address() != address {
                        return;
                    }
                }
            };
            let mut watching = token.clone();
            tokio::select! {
                () = serving => {}
                () = moved => {}
                _ = watching.wait_for(Option::is_none) => {}
            }
        }
    }
}

const NO_WIFI: &str = "No Wi-Fi connection: connect the laptop to the Wi-Fi the PocketBook uses";

/// One network adapter, as far as choosing the Wi-Fi address goes.
#[derive(Debug, Clone, Copy)]
struct Adapter {
    wifi: bool,
    up: bool,
    /// Mobile Hotspot's Wi-Fi Direct adapter is Wi-Fi and up, but has no gateway.
    gateway: bool,
    ip: Ipv4Addr,
}

/// The address to listen on: a connected Wi-Fi adapter's. Never the VPN's or a wired one's.
fn choose(adapters: &[Adapter]) -> Option<Ipv4Addr> {
    adapters.iter().find(|a| a.wifi && a.up && a.gateway && !a.ip.is_link_local()).map(|a| a.ip)
}

fn wifi_address() -> Option<Ipv4Addr> {
    choose(&adapters())
}

/// Each adapter's IPv4 addresses, from `GetAdaptersAddresses`.
fn adapters() -> Vec<Adapter> {
    use windows_sys::Win32::Foundation::{ERROR_BUFFER_OVERFLOW, NO_ERROR};
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        GAA_FLAG_INCLUDE_GATEWAYS, GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_DNS_SERVER, GAA_FLAG_SKIP_MULTICAST, GetAdaptersAddresses,
        IF_TYPE_IEEE80211, IP_ADAPTER_ADDRESSES_LH,
    };
    use windows_sys::Win32::NetworkManagement::Ndis::IfOperStatusUp;
    use windows_sys::Win32::Networking::WinSock::{AF_INET, SOCKADDR_IN};

    let flags = GAA_FLAG_INCLUDE_GATEWAYS | GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST | GAA_FLAG_SKIP_DNS_SERVER;
    let mut size: u32 = 16 * 1024;
    // u64 words keep the buffer aligned for the structs Windows writes into it.
    let mut buffer: Vec<u64> = Vec::new();
    for _ in 0..3 {
        buffer.resize((size as usize).div_ceil(8), 0);
        let first = buffer.as_mut_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>();
        // SAFETY: `buffer` holds at least `size` writable, aligned bytes.
        let result = unsafe { GetAdaptersAddresses(u32::from(AF_INET), flags, std::ptr::null(), first, &mut size) };
        if result == ERROR_BUFFER_OVERFLOW {
            continue;
        }
        if result != NO_ERROR {
            return Vec::new();
        }
        let mut found = Vec::new();
        let mut adapter = first.cast_const();
        // SAFETY: Windows wrote a linked list inside `buffer`, which outlives this walk; each pointer is null or valid,
        // and with AF_INET every unicast address is a SOCKADDR_IN.
        unsafe {
            while let Some(a) = adapter.as_ref() {
                let mut unicast = a.FirstUnicastAddress.cast_const();
                while let Some(u) = unicast.as_ref() {
                    let addr = u.Address.lpSockaddr;
                    if !addr.is_null() && (*addr).sa_family == AF_INET {
                        found.push(Adapter {
                            wifi: a.IfType == IF_TYPE_IEEE80211,
                            up: a.OperStatus == IfOperStatusUp,
                            gateway: !a.FirstGatewayAddress.is_null(),
                            ip: Ipv4Addr::from(u32::from_be((*addr.cast::<SOCKADDR_IN>()).sin_addr.S_un.S_addr)),
                        });
                    }
                    unicast = u.Next.cast_const();
                }
                adapter = a.Next.cast_const();
            }
        }
        return found;
    }
    Vec::new()
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use serde_json::{Value, json};

    use super::*;
    use crate::score::ReviewItem;
    use crate::store::tests::TempDir;

    const TOKEN: &str = "pairing-token";

    struct Fake {
        session: Mutex<Session>,
        opened: Mutex<Vec<String>>,
        _dir: TempDir,
    }

    impl Rado for Arc<Fake> {
        fn read<T>(&self, f: impl FnOnce(&Session) -> T) -> T {
            f(&self.session.lock().unwrap())
        }

        fn change<T>(&self, f: impl FnOnce(&mut Session) -> T) -> T {
            f(&mut self.session.lock().unwrap())
        }

        fn open(&self, urls: Vec<String>) -> Result<(), String> {
            self.opened.lock().unwrap().extend(urls);
            Ok(())
        }
    }

    fn refreshed() -> DateTime<Utc> {
        "2026-11-25T09:30:00Z".parse().unwrap()
    }

    fn item(id: i64, provider: Provider, repo: &str, branch: &str, state: ReviewState, score: i32, age_hours: i64) -> ReviewItem {
        ReviewItem {
            id,
            provider,
            number: id + 100,
            title: format!("Change {id}"),
            repo: repo.into(),
            branch: branch.into(),
            author: "Author".into(),
            target: "main".into(),
            release_target: false,
            age_hours,
            created_at: refreshed() - chrono::Duration::hours(age_hours),
            updated_at: None,
            required: false,
            via_group: false,
            draft: false,
            state,
            score,
            tier: Tier::Normal,
            hidden: false,
            approvals: 0,
            objections: 0,
            comments: None,
            actionable: state.is_actionable(),
        }
    }

    /// Review `pr:1` (urgent, ticket ABC-12, a linked draft); hidden `pr:8`; re-review `pr:4`; an aggregate of a
    /// review and a PR of mine with an unlinked draft; my `pr:6` (older, needs work) and `pr:7` (newer); approved `pr:9`.
    fn fake(name: &str) -> Arc<Fake> {
        use Provider::{Ado, GitHub};
        use ReviewState::*;
        let items = vec![
            ReviewItem {
                tier: Tier::Urgent,
                approvals: 1,
                comments: Some(CommentStats { total: 3, open_threads: 1 }),
                ..item(1, Ado, "api", "feature/ABC-12-login", NeedsReview, 90, 30)
            },
            ReviewItem { tier: Tier::Urgent, hidden: true, actionable: false, ..item(8, Ado, "api", "chore/deps", NeedsReview, 85, 2) },
            item(4, GitHub, "web", "fix/header", ReReview, 80, 12),
            item(2, GitHub, "web", "feature/shared", NeedsReview, 70, 48),
            item(3, GitHub, "app", "feature/shared", Mine, 60, 47),
            ReviewItem { objections: 1, actionable: true, ..item(6, Ado, "api", "feature/old", Mine, 40, 50) },
            item(7, Ado, "api", "feature/new", Mine, 30, 5),
            item(9, GitHub, "web", "feature/done", Approved, 20, 72),
        ];
        let urls = items.iter().map(|i| (i.id, format!("https://example.test/pr/{}", i.id))).collect();
        let drafts = HashMap::from([(1, Some("https://drafts.example.test/1".to_string())), (3, None)]);
        let (session, dir) = crate::session::tests::listing(&format!("ink-{name}"), items, urls, drafts, refreshed());
        Arc::new(Fake { session: Mutex::new(session), opened: Mutex::new(Vec::new()), _dir: dir })
    }

    fn request(method: &str, path: &str, body: Value) -> Request {
        Request {
            method: method.into(),
            path: path.into(),
            authorization: Some(format!("Bearer {TOKEN}")),
            if_none_match: None,
            body: if body.is_null() { Vec::new() } else { body.to_string().into_bytes() },
        }
    }

    fn send(rado: &Arc<Fake>, request: &Request) -> Response {
        handle(rado, Some(TOKEN), request)
    }

    fn get_list(rado: &Arc<Fake>) -> Value {
        let response = send(rado, &request("GET", "/v1/list", Value::Null));
        assert_eq!(response.status, 200);
        serde_json::from_str(response.body.as_deref().unwrap()).unwrap()
    }

    fn tabs(list: &Value) -> Vec<(String, String)> {
        list["rows"].as_array().unwrap().iter().map(|r| (r["key"].as_str().unwrap().into(), r["tab"].as_str().unwrap().into())).collect()
    }

    fn row<'a>(list: &'a Value, key: &str) -> &'a Value {
        list["rows"].as_array().unwrap().iter().find(|r| r["key"] == key).unwrap()
    }

    #[test]
    fn list_rowsComeInRadosOrder_withTheTabRadoDecides_andMineNewestFirst() {
        let expected = [
            ("pr:1", "needsReview"),
            ("pr:8", "hidden"),
            ("pr:4", "reReview"),
            ("branch:GitHub:feature/shared", "needsReview"),
            ("pr:7", "mine"),
            ("pr:6", "mine"),
            ("pr:9", "approved"),
        ];
        let expected: Vec<(String, String)> = expected.iter().map(|(k, t)| (k.to_string(), t.to_string())).collect();
        assert_eq!(tabs(&get_list(&fake("order"))), expected);
    }

    #[test]
    fn list_carriesTheHeaderAndEachRowsFacts() {
        let list = get_list(&fake("content"));
        assert_eq!(list["workspace"], "Acme");
        assert_eq!(list["counts"], json!({ "reviews": 3, "mine": 1 }));
        assert_eq!((&list["lastRefreshed"], &list["polling"], &list["error"]), (&json!("2026-11-25T09:30:00Z"), &json!(false), &Value::Null));
        assert_eq!(
            row(&list, "pr:1"),
            &json!({
                "key": "pr:1", "tab": "needsReview", "tier": "urgent", "title": "Change 1", "ticket": "ABC-12", "ageHours": 30,
                "approvals": 1, "objections": 0, "comments": { "total": 3, "openThreads": 1 }, "waiting": true,
                "holdsMine": false, "draft": { "linked": true }, "prs": [{ "repo": "api", "number": 101, "provider": "ado" }]
            })
        );
        let shared = row(&list, "branch:GitHub:feature/shared");
        assert_eq!((&shared["holdsMine"], &shared["draft"], shared["prs"].as_array().unwrap().len()), (&json!(true), &json!({ "linked": false }), 2));
        assert_eq!(row(&list, "pr:4")["holdsMine"], false);
        assert_eq!(row(&list, "pr:8")["tier"], "low", "hidden rows are drawn low, as in the list window");
    }

    #[test]
    fn list_holdsNoLinksSettingsOrSecrets_evenWhenTheSourceErrorQuotesAUrl() {
        let rado = fake("leaks");
        rado.change(|s| s.report("Azure DevOps: network: error sending request for url (https://dev.azure.com/acme/_apis/git)".into()));
        let body = send(&rado, &request("GET", "/v1/list", Value::Null)).body.unwrap();
        for leak in ["://", "example.test", "settings", "hasPat", "oken", "saved", "url\""] {
            assert!(!body.contains(leak), "{leak} in {body}");
        }
        assert_eq!(serde_json::from_str::<Value>(&body).unwrap()["error"], "Azure DevOps: network: error sending request for url (");
    }

    #[test]
    fn list_sameETag_isNotModified_untilTheListChanges() {
        let rado = fake("etag");
        let first = send(&rado, &request("GET", "/v1/list", Value::Null));
        let again = Request { if_none_match: first.etag.clone(), ..request("GET", "/v1/list", Value::Null) };
        assert_eq!(send(&rado, &again), Response { status: 304, etag: first.etag.clone(), body: None });

        assert_eq!(send(&rado, &request("POST", "/v1/hidden", json!({ "key": "pr:4", "hidden": true }))).status, 204);
        let changed = send(&rado, &again);
        assert_eq!(changed.status, 200);
        assert_ne!(changed.etag, first.etag);
    }

    #[test]
    fn hidden_movesTheRowToHidden_andShowBringsItBack() {
        let rado = fake("hide");
        assert_eq!(send(&rado, &request("POST", "/v1/hidden", json!({ "key": "pr:4", "hidden": true }))).status, 204);
        let list = get_list(&rado);
        assert_eq!((&row(&list, "pr:4")["tab"], &row(&list, "pr:4")["tier"]), (&json!("hidden"), &json!("low")));

        assert_eq!(send(&rado, &request("POST", "/v1/hidden", json!({ "key": "pr:4", "hidden": false }))).status, 204);
        assert_eq!(row(&get_list(&rado), "pr:4")["tab"], "reReview");
    }

    #[test]
    fn rowActions_unknownRow_isGoneWithRadosText() {
        let rado = fake("gone");
        for path in ["/v1/open", "/v1/ticket", "/v1/draft", "/v1/hidden"] {
            let response = send(&rado, &request("POST", path, json!({ "key": "pr:999", "hidden": true })));
            assert_eq!((response.status, response.body.as_deref()), (410, Some(r#"{"error":"That row is gone; refresh the list"}"#)), "{path}");
        }
        assert!(rado.opened.lock().unwrap().is_empty());
    }

    #[test]
    fn everyRequest_withoutTheToken_isUnauthorized() {
        let rado = fake("auth");
        let unauthorized = Some(r#"{"error":"unauthorized"}"#);
        let bearer = format!("Bearer {TOKEN}");
        let attempts = [(Some(TOKEN), None), (Some(TOKEN), Some("Bearer wrong")), (Some(TOKEN), Some(TOKEN)), (None, Some(bearer.as_str())), (None, Some("Bearer "))];
        for (token, header) in attempts {
            for (method, path) in [("GET", "/v1/list"), ("POST", "/v1/refresh"), ("POST", "/v1/hidden"), ("GET", "/elsewhere")] {
                let req = Request { authorization: header.map(String::from), ..request(method, path, json!({ "key": "pr:4", "hidden": true })) };
                let response = handle(&rado, token, &req);
                assert_eq!((response.status, response.body.as_deref()), (401, unauthorized), "{method} {path} with {header:?}");
            }
        }
        assert_eq!(row(&get_list(&rado), "pr:4")["tab"], "reReview");
    }

    #[test]
    fn open_opensTheRowsLinksTaggedForTheExtension() {
        let rado = fake("open");
        assert_eq!(send(&rado, &request("POST", "/v1/open", json!({ "key": "branch:GitHub:feature/shared" }))).status, 204);
        assert_eq!(
            *rado.opened.lock().unwrap(),
            ["https://example.test/pr/2#rado=feature%2Fshared&group", "https://example.test/pr/3#rado=feature%2Fshared&group"]
        );
    }

    #[test]
    fn ticketAndDraft_openTheirLinks_orAnswerWithRadosReason() {
        let rado = fake("ticket-draft");
        assert_eq!(send(&rado, &request("POST", "/v1/ticket", json!({ "key": "pr:1" }))).status, 204);
        assert_eq!(send(&rado, &request("POST", "/v1/draft", json!({ "key": "pr:1" }))).status, 204);
        assert_eq!(*rado.opened.lock().unwrap(), ["https://acme.atlassian.net/browse/ABC-12", "https://drafts.example.test/1"]);

        let no_ticket = send(&rado, &request("POST", "/v1/ticket", json!({ "key": "pr:4" })));
        assert_eq!((no_ticket.status, no_ticket.body.as_deref()), (409, Some(r#"{"error":"That row's branch names no ticket"}"#)));
        let unlinked = send(&rado, &request("POST", "/v1/draft", json!({ "key": "branch:GitHub:feature/shared" })));
        assert_eq!((unlinked.status, unlinked.body.as_deref()), (409, Some(r#"{"error":"That PR has no linked draft now; refresh the list"}"#)));
    }

    #[test]
    fn refresh_wakesThePoller() {
        let rado = fake("refresh");
        assert_eq!(send(&rado, &request("POST", "/v1/refresh", Value::Null)).status, 204);
        assert!(rado.change(|s| s.take_wake()));
    }

    #[test]
    fn badBodyOrUnknownPath_isRefused() {
        let rado = fake("bad");
        assert_eq!(send(&rado, &request("POST", "/v1/hidden", json!({ "key": "pr:4" }))).status, 400);
        assert_eq!(send(&rado, &request("GET", "/v2/list", Value::Null)).status, 404);
        assert_eq!(send(&rado, &request("GET", "/v1/list?x=1", Value::Null)).status, 200);
    }

    /// The contract with the device: ink-core parses this file.
    #[test]
    fn writeListFixture() {
        let body = serde_json::to_string_pretty(&fake("fixture").read(list)).unwrap() + "\n";
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../ink/fixtures");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("list.json"), body).unwrap();
    }

    async fn read(bytes: &[u8]) -> Result<Request, u16> {
        read_request(&mut &bytes[..]).await.map_err(|r| r.status)
    }

    #[tokio::test]
    async fn readRequest_readsTheBodyItsLengthNames() {
        let request = read(b"POST /v1/hidden HTTP/1.1\r\nauthorization: Bearer t\r\nContent-Length: 4\r\n\r\nbodyEXTRA").await.unwrap();
        let read = (request.method.as_str(), request.path.as_str(), request.authorization.as_deref(), &request.body[..]);
        assert_eq!(read, ("POST", "/v1/hidden", Some("Bearer t"), &b"body"[..]));
    }

    #[tokio::test]
    async fn readRequest_refusesWhatItCantReadWhole_withoutPanicking() {
        let huge = format!("GET / HTTP/1.1\r\nX: {}\r\n", "a".repeat(MAX_REQUEST));
        let cases: [(&[u8], u16); 6] = [
            (b"POST / HTTP/1.1\r\nContent-Length: 99999999999999999999\r\n\r\n", 400),
            (b"POST / HTTP/1.1\r\nContent-Length: 18446744073709551615\r\n\r\n", 413),
            (b"POST / HTTP/1.1\r\nContent-Length: 20000\r\n\r\n", 413),
            (b"POST / HTTP/1.1\r\nContent-Length: 10\r\n\r\nshort", 400),
            (b"POST / HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n", 400),
            (huge.as_bytes(), 413),
        ];
        for (bytes, status) in cases {
            assert_eq!(read(bytes).await.err(), Some(status), "{}", String::from_utf8_lossy(&bytes[..40.min(bytes.len())]));
        }
        assert!(read(b"GET /\xff\xfe HTTP/1.1\r\n\r\n").await.is_err());
    }

    /// Over a real socket, a request whose body arrives after its headers is read whole before the answer.
    #[tokio::test]
    async fn accept_answersOnlyAfterTheWholeRequestArrived() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let rado = fake("socket");
        let (_sender, token) = watch::channel(Some(TOKEN.to_string()));
        tokio::spawn(accept(listener, rado.clone(), token));

        let mut client = tokio::net::TcpStream::connect(address).await.unwrap();
        let body = r#"{"key":"pr:4","hidden":true}"#;
        let head = format!("POST /v1/hidden HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer {TOKEN}\r\nContent-Length: {}\r\n\r\n", body.len());
        client.write_all(head.as_bytes()).await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        client.write_all(body.as_bytes()).await.unwrap();
        let mut answer = String::new();
        client.read_to_string(&mut answer).await.unwrap();

        assert!(answer.starts_with("HTTP/1.1 204 No Content\r\n"), "{answer}");
        assert_eq!(row(&get_list(&rado), "pr:4")["tab"], "hidden");
    }

    #[test]
    fn choose_takesTheConnectedWifiAdapterWithAGateway() {
        let adapter = |wifi, up, gateway, ip: [u8; 4]| Adapter { wifi, up, gateway, ip: Ipv4Addr::from(ip) };
        let vpn = adapter(false, true, true, [10, 212, 0, 5]);
        let hotspot = adapter(true, true, false, [192, 168, 137, 1]);
        let wifi = adapter(true, true, true, [192, 168, 1, 14]);
        assert_eq!(choose(&[vpn, hotspot, wifi]), Some(Ipv4Addr::new(192, 168, 1, 14)));
        assert_eq!(choose(&[vpn, hotspot, Adapter { up: false, ..wifi }]), None);
        assert_eq!(choose(&[adapter(true, true, true, [169, 254, 3, 4])]), None);
    }

    #[test]
    fn pairing_createsReplacesAndRemovesTheToken() {
        let secrets = crate::store::tests::MemorySecrets::default();
        let pairing = Pairing::load(Box::new(secrets.clone()));
        assert!(!pairing.status().paired);
        assert_eq!(pairing.config(), Err("Pair a device first".into()));

        pairing.pair().unwrap();
        let first = secrets.get(TOKEN_KEY).unwrap();
        assert_eq!(first.len(), 43, "32 random bytes, base64url");
        pairing.pair().unwrap();
        let second = secrets.get(TOKEN_KEY).unwrap();
        assert_ne!(first, second);
        assert!(pairing.status().paired);
        assert_eq!(pairing.token.borrow().as_deref(), Some(second.as_str()));

        pairing.unpair().unwrap();
        assert_eq!((secrets.get(TOKEN_KEY), pairing.status().paired), (None, false));
    }
}
