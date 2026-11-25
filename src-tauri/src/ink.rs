//! The e-ink endpoint: API v1 for the PocketBook app in `ink/`, served on the laptop's Wi-Fi address while a pairing
//! token exists. It only reads: `GET /v1/list` answers from the same session the list window shows.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::Mutex;
use std::time::Duration;

use base64::Engine;
use chrono::{DateTime, Utc};
use serde::Serialize;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::watch;

use crate::group::{Counts, ReviewRow};
use crate::score::{ReviewState, Tier};
use crate::session::Session;
use crate::store::Secrets;

pub const PORT: u16 = 7878;
const TOKEN_KEY: &str = "ink-token";
const MAX_REQUEST: usize = 16 * 1024;
const CONNECTION_TIMEOUT: Duration = Duration::from_secs(10);
/// How often the listener checks whether the Wi-Fi address changed, and retries after it couldn't listen.
const ADDRESS_CHECK: Duration = Duration::from_secs(60);

/// What the device gets from `GET /v1/list`: the active workspace's list, without links, settings or secrets.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct List {
    workspace: String,
    counts: Counts,
    last_refreshed: Option<DateTime<Utc>>,
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
    age_hours: i64,
    waiting: bool,
    draft: Option<Draft>,
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

/// An agent's draft on one of the row's PRs. Braced, so it serialises as `{}`: a unit struct would be `null`, which
/// the device reads as no draft.
#[derive(Debug, Serialize)]
struct Draft {}

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
        Self {
            tab: tab(&row),
            // As the list window draws it.
            tier: if row.summary.hidden { Tier::Low } else { row.summary.tier },
            title: row.summary.title,
            age_hours: row.summary.age_hours,
            waiting: row.summary.actionable,
            draft: row.members.iter().any(|m| m.agent_draft.is_some()).then_some(Draft {}),
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

/// Rado as the endpoint sees it.
pub trait Rado {
    fn read<T>(&self, f: impl FnOnce(&Session) -> T) -> T;
}

#[derive(Debug)]
pub struct Request {
    pub method: String,
    pub path: String,
    pub authorization: Option<String>,
    pub if_none_match: Option<String>,
}

#[derive(Debug, PartialEq)]
pub struct Response {
    pub status: u16,
    pub etag: Option<String>,
    pub body: Option<String>,
}

impl Response {
    fn error(status: u16, text: &str) -> Self {
        Self { status, etag: None, body: Some(serde_json::json!({ "error": text }).to_string()) }
    }
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
        _ => Response::error(404, "not found"),
    }
}

/// Compares in time that doesn't depend on where the strings differ.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Reads one request's headers; the API takes no body.
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
        match parsed.parse(&buf) {
            Ok(httparse::Status::Complete(_)) => {}
            Ok(httparse::Status::Partial) => continue,
            Err(e) => return Err(Response::error(400, &format!("bad request: {e}"))),
        }
        let header = |name: &str| {
            parsed.headers.iter().find(|h| h.name.eq_ignore_ascii_case(name)).map(|h| String::from_utf8_lossy(h.value).trim().to_string())
        };
        return Ok(Request {
            method: parsed.method.unwrap_or_default().to_string(),
            path: parsed.path.unwrap_or_default().to_string(),
            authorization: header("Authorization"),
            if_none_match: header("If-None-Match"),
        });
    }
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        304 => "Not Modified",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
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

/// Reads a request, answers it with the token current by then, and closes the connection.
async fn serve<R: Rado + Clone + Send + 'static>(stream: &mut (impl AsyncRead + AsyncWrite + Unpin), rado: R, token: &watch::Receiver<Option<String>>) {
    let response = match read_request(stream).await {
        Err(response) => response,
        Ok(request) => {
            let token = token.borrow().clone();
            // The session's lock is held across a poll's file writes; wait for it off the async threads.
            tokio::task::spawn_blocking(move || handle(&rado, token.as_deref(), &request))
                .await
                .unwrap_or_else(|_| Response::error(500, "Rado failed to answer"))
        }
    };
    let _ = stream.write_all(&encode(&response)).await;
    let _ = stream.shutdown().await;
}

/// One connection at a time, each cut off after `CONNECTION_TIMEOUT`: there is one device, and it bounds what a
/// stranger on the network can hold open. Returns when the listener fails, so `run` binds again.
async fn accept<R: Rado + Clone + Send + 'static>(listener: TcpListener, rado: R, token: watch::Receiver<Option<String>>) {
    loop {
        let Ok((mut stream, _)) = listener.accept().await else {
            tokio::time::sleep(Duration::from_secs(1)).await;
            return;
        };
        let _ = tokio::time::timeout(CONNECTION_TIMEOUT, serve(&mut stream, rado.clone(), &token)).await;
    }
}

/// The pairing token, kept in Credential Manager and mirrored in memory for the listener.
pub struct Pairing {
    secrets: Box<dyn Secrets>,
    token: watch::Sender<Option<String>>,
    /// Why the device can't reach Rado while a token exists, or why the token couldn't be read. Its lock is held
    /// across every change to the token, so Credential Manager, `token` and `problem` always agree.
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
        let (paired, problem) = {
            let problem = self.problem.lock().expect("pairing lock");
            (self.token.borrow().is_some(), problem.clone())
        };
        InkStatus { paired, address: wifi_socket().map(|at| at.to_string()), problem }
    }

    /// Creates a token, replacing any earlier one, which stops working at once.
    pub fn pair(&self) -> Result<(), String> {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).map_err(|e| format!("Couldn't make a token: {e}"))?;
        let token = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
        let mut problem = self.problem.lock().expect("pairing lock");
        self.secrets.set(TOKEN_KEY, &token)?;
        // Replacing a token keeps the listener as it is, and with it any reason it can't listen.
        if self.token.send_replace(Some(token)).is_none() {
            *problem = None;
        }
        Ok(())
    }

    pub fn unpair(&self) -> Result<(), String> {
        let mut problem = self.problem.lock().expect("pairing lock");
        self.secrets.delete(TOKEN_KEY)?;
        self.token.send_replace(None);
        *problem = None;
        Ok(())
    }

    /// The text of the device's config file, `system/config/rado.cfg` on the PocketBook.
    pub fn config(&self) -> Result<String, String> {
        let token = self.token.borrow().clone().ok_or("Pair a device first")?;
        let host = wifi_address().ok_or("No Wi-Fi connection: connect the laptop to the Wi-Fi the PocketBook uses")?;
        Ok(format!("host={host}\nport={PORT}\ntoken={token}\n"))
    }

    /// Records why the listener can't listen, unless the token went meanwhile.
    fn report(&self, problem: Option<String>) {
        let mut current = self.problem.lock().expect("pairing lock");
        if self.token.borrow().is_some() {
            *current = problem;
        }
    }

    /// Listens on `address()` (the Wi-Fi address and `PORT` in the app) while a token exists, and moves when the
    /// address changes. Off Wi-Fi it listens nowhere, and Settings shows no address.
    pub async fn run<R: Rado + Clone + Send + 'static>(&self, rado: R, address: impl Fn() -> Option<SocketAddrV4>) {
        let mut token = self.token.subscribe();
        loop {
            if token.borrow_and_update().is_none() {
                if token.changed().await.is_err() {
                    return;
                }
                continue;
            }
            let at = address();
            let bound = match at {
                None => None,
                Some(at) => Some(TcpListener::bind(at).await.map_err(|e| format!("Can't listen on {at} ({e})"))),
            };
            self.report(bound.as_ref().and_then(|b| b.as_ref().err().cloned()));
            let serving = async {
                match bound {
                    Some(Ok(listener)) => accept(listener, rado.clone(), token.clone()).await,
                    _ => tokio::time::sleep(ADDRESS_CHECK).await,
                }
            };
            let moved = async {
                loop {
                    tokio::time::sleep(ADDRESS_CHECK).await;
                    if address() != at {
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

/// The Wi-Fi address with `PORT`, where the app listens.
pub fn wifi_socket() -> Option<SocketAddrV4> {
    wifi_address().map(|ip| SocketAddrV4::new(ip, PORT))
}

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
    use crate::score::{Provider, ReviewItem};
    use crate::store::tests::TempDir;

    const TOKEN: &str = "pairing-token";

    struct Fake {
        session: Mutex<Session>,
        _dir: TempDir,
    }

    impl Rado for Arc<Fake> {
        fn read<T>(&self, f: impl FnOnce(&Session) -> T) -> T {
            f(&self.session.lock().unwrap())
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

    /// Review `pr:1` (urgent, a draft with its link); hidden `pr:8`; re-review `pr:4`; an aggregate of a review and a
    /// PR of mine with a draft that has none; my `pr:6` (older, needs work) and `pr:7` (newer); approved `pr:9`.
    fn fake(name: &str) -> Arc<Fake> {
        use Provider::{Ado, GitHub};
        use ReviewState::*;
        let items = vec![
            ReviewItem { tier: Tier::Urgent, ..item(1, Ado, "api", "feature/ABC-12-login", NeedsReview, 90, 30) },
            ReviewItem { tier: Tier::Urgent, hidden: true, actionable: false, ..item(8, Ado, "api", "chore/deps", NeedsReview, 85, 2) },
            item(4, GitHub, "web", "fix/header", ReReview, 80, 12),
            item(2, GitHub, "web", "feature/shared", NeedsReview, 70, 48),
            item(3, GitHub, "app", "feature/shared", Mine, 60, 47),
            ReviewItem { objections: 1, actionable: true, ..item(6, Ado, "api", "feature/old", Mine, 40, 50) },
            item(7, Ado, "api", "feature/new", Mine, 30, 5),
            item(9, GitHub, "web", "feature/done", Approved, 20, 72),
        ];
        let drafts = HashMap::from([(1, Some("https://drafts.example.test/1".to_string())), (3, None)]);
        let (session, dir) = crate::session::tests::listing(&format!("ink-{name}"), items, drafts, refreshed());
        Arc::new(Fake { session: Mutex::new(session), _dir: dir })
    }

    fn request(method: &str, path: &str) -> Request {
        Request { method: method.into(), path: path.into(), authorization: Some(format!("Bearer {TOKEN}")), if_none_match: None }
    }

    fn send(rado: &Arc<Fake>, request: &Request) -> Response {
        handle(rado, Some(TOKEN), request)
    }

    fn get_list(rado: &Arc<Fake>) -> Value {
        let response = send(rado, &request("GET", "/v1/list"));
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
        assert_eq!((&list["lastRefreshed"], &list["error"]), (&json!("2026-11-25T09:30:00Z"), &Value::Null));
        assert_eq!(
            row(&list, "pr:1"),
            &json!({ "key": "pr:1", "tab": "needsReview", "tier": "urgent", "title": "Change 1", "ageHours": 30, "waiting": true, "draft": {} })
        );
        assert_eq!(row(&list, "branch:GitHub:feature/shared")["draft"], json!({}), "an unlinked draft counts too");
        assert_eq!(row(&list, "pr:4")["draft"], Value::Null);
        assert_eq!(row(&list, "pr:8")["tier"], "low", "hidden rows are drawn low, as in the list window");
    }

    #[test]
    fn list_holdsNoLinksSettingsOrSecrets_evenWhenTheSourceErrorQuotesAUrl() {
        let rado = fake("leaks");
        rado.session.lock().unwrap().report("Azure DevOps: network: error sending request for url (https://dev.azure.com/acme/_apis/git)".into());
        let body = send(&rado, &request("GET", "/v1/list")).body.unwrap();
        for leak in ["://", "example.test", "settings", "hasPat", "oken", "saved", "url\""] {
            assert!(!body.contains(leak), "{leak} in {body}");
        }
        assert_eq!(serde_json::from_str::<Value>(&body).unwrap()["error"], "Azure DevOps: network: error sending request for url (");
    }

    #[test]
    fn list_sameETag_isNotModified_untilTheListChanges() {
        let rado = fake("etag");
        let first = send(&rado, &request("GET", "/v1/list"));
        let again = Request { if_none_match: first.etag.clone(), ..request("GET", "/v1/list") };
        assert_eq!(send(&rado, &again), Response { status: 304, etag: first.etag.clone(), body: None });

        rado.session.lock().unwrap().set_hidden("pr:4", true).unwrap();
        let changed = send(&rado, &again);
        assert_eq!(changed.status, 200);
        assert_ne!(changed.etag, first.etag);
    }

    #[test]
    fn everyRequest_withoutTheToken_isUnauthorized() {
        let rado = fake("auth");
        let unauthorized = Some(r#"{"error":"unauthorized"}"#);
        let bearer = format!("Bearer {TOKEN}");
        let attempts = [(Some(TOKEN), None), (Some(TOKEN), Some("Bearer wrong")), (Some(TOKEN), Some(TOKEN)), (None, Some(bearer.as_str())), (None, Some("Bearer "))];
        for (token, header) in attempts {
            for (method, path) in [("GET", "/v1/list"), ("POST", "/v1/refresh"), ("GET", "/elsewhere")] {
                let req = Request { authorization: header.map(String::from), ..request(method, path) };
                let response = handle(&rado, token, &req);
                assert_eq!((response.status, response.body.as_deref()), (401, unauthorized), "{method} {path} with {header:?}");
            }
        }
    }

    #[test]
    fn anythingButGetList_isNotFound_soTheDeviceChangesNothing() {
        let rado = fake("read-only");
        for (method, path) in [("POST", "/v1/refresh"), ("POST", "/v1/open"), ("POST", "/v1/ticket"), ("POST", "/v1/draft"), ("POST", "/v1/hidden"), ("POST", "/v1/list"), ("GET", "/v2/list")] {
            assert_eq!(send(&rado, &request(method, path)).status, 404, "{method} {path}");
        }
        assert_eq!(send(&rado, &request("GET", "/v1/list?x=1")).status, 200);
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
    async fn readRequest_readsTheHeadersTheEndpointUses() {
        let request = read(b"GET /v1/list HTTP/1.1\r\nauthorization: Bearer t\r\nIf-None-Match: \"e1\"\r\n\r\n").await.unwrap();
        let read = (request.method.as_str(), request.path.as_str(), request.authorization.as_deref(), request.if_none_match.as_deref());
        assert_eq!(read, ("GET", "/v1/list", Some("Bearer t"), Some("\"e1\"")));
    }

    #[tokio::test]
    async fn readRequest_refusesWhatItCantRead_withoutPanicking() {
        let huge = format!("GET / HTTP/1.1\r\nX: {}\r\n", "a".repeat(MAX_REQUEST));
        assert_eq!(read(huge.as_bytes()).await.err(), Some(413));
        assert_eq!(read(b"GET / HTTP/1.1\r\n").await.err(), Some(400), "cut off");
        assert!(read(b"GET /\xff\xfe HTTP/1.1\r\n\r\n").await.is_err());
    }

    /// Over a real socket, a request whose headers arrive in two parts is read whole before the answer.
    #[tokio::test]
    async fn accept_answersOnlyAfterTheWholeRequestArrived() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let rado = fake("socket");
        let (_sender, token) = watch::channel(Some(TOKEN.to_string()));
        tokio::spawn(accept(listener, rado.clone(), token));

        let mut client = tokio::net::TcpStream::connect(address).await.unwrap();
        client.write_all(b"GET /v1/list HTTP/1.1\r\nHost: x\r\n").await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        client.write_all(format!("Authorization: Bearer {TOKEN}\r\n\r\n").as_bytes()).await.unwrap();
        let mut answer = String::new();
        client.read_to_string(&mut answer).await.unwrap();

        assert!(answer.starts_with("HTTP/1.1 200 OK\r\n"), "{answer}");
    }

    /// Whether connecting to `at` succeeds (`open`) or fails within about two seconds.
    async fn becomes(at: SocketAddrV4, open: bool) -> bool {
        for _ in 0..100 {
            if tokio::net::TcpStream::connect(at).await.is_ok() == open {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        false
    }

    #[tokio::test]
    async fn run_listensOnlyWhileATokenExists() {
        let free = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let at = SocketAddrV4::new(Ipv4Addr::LOCALHOST, free);
        let pairing = Arc::new(Pairing::load(Box::new(crate::store::tests::MemorySecrets::default())));
        let (running, rado) = (pairing.clone(), fake("run"));
        tokio::spawn(async move { running.run(rado, move || Some(at)).await });

        pairing.pair().unwrap();
        assert!(becomes(at, true).await, "paired: listening");
        pairing.unpair().unwrap();
        assert!(becomes(at, false).await, "removed: not listening");
        assert_eq!(pairing.status().problem, None);
    }

    #[tokio::test]
    async fn run_cantListen_saysWhy_untilTheTokenIsRemoved() {
        let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let at = SocketAddrV4::new(Ipv4Addr::LOCALHOST, taken.local_addr().unwrap().port());
        let pairing = Arc::new(Pairing::load(Box::new(crate::store::tests::MemorySecrets::default())));
        let (running, rado) = (pairing.clone(), fake("cant-listen"));
        tokio::spawn(async move { running.run(rado, move || Some(at)).await });

        pairing.pair().unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(pairing.status().problem.is_some_and(|p| p.starts_with(&format!("Can't listen on {at}"))));
        pairing.pair().unwrap();
        assert!(pairing.status().problem.is_some(), "a new token doesn't free the port");

        pairing.unpair().unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(pairing.status().problem, None);
    }

    #[tokio::test]
    async fn run_keepsTheReasonTheTokenCouldntBeRead_untilPaired() {
        let secrets = crate::store::tests::MemorySecrets::default();
        secrets.fail_reads(true);
        let pairing = Arc::new(Pairing::load(Box::new(secrets.clone())));
        let (running, rado) = (pairing.clone(), fake("unreadable"));
        tokio::spawn(async move { running.run(rado, || None).await });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(pairing.status().problem.is_some_and(|p| p.starts_with("Can't read the pairing token")));

        pairing.pair().unwrap();
        assert_eq!(pairing.status().problem, None);
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
