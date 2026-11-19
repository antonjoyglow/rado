//! Rado's MCP server, for scheduled review agents. It serves the review queue that the Rado app writes after each
//! poll (`queue.json`, in the active workspace's folder) over stdio, and records which PRs already have a draft
//! review. It never reads a token; the review itself goes through the Azure DevOps or GitHub MCP server.

#[allow(dead_code)]
#[path = "../queue.rs"]
mod queue;

use std::fs::File;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde_json::{Value, json};

use queue::{DRAFTS_FILE, DRAFTS_LOCK_FILE, Drafts, QUEUE_FILE, QUEUE_SCHEMA, Queue, ReadError, SETTINGS_FILE};

/// The newest protocol version this server speaks, answered to a client asking for one it doesn't know.
const PROTOCOL: &str = "2025-11-25";
const PROTOCOLS: [&str; 4] = [PROTOCOL, "2025-06-18", "2025-03-26", "2024-11-05"];

fn main() {
    let dir = data_dir();
    log(&startup(&dir));
    let mut stdout = std::io::stdout().lock();
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        if let Some(response) = handle(&dir, Utc::now(), &line) {
            if let Some(message) = error_message(&response) {
                log(message);
            }
            let _ = writeln!(stdout, "{response}");
            let _ = stdout.flush();
        }
    }
}

/// Stdout carries the protocol, so the log goes to stderr, which Claude Code keeps in its MCP logs.
fn log(line: &str) {
    let _ = writeln!(std::io::stderr(), "rado-mcp: {line}");
}

/// Logged at start, so a wrong data folder shows before the first tool call.
fn startup(root: &Path) -> String {
    let workspace = workspace(root, None).map_or_else(|e| e, |ws| format!("active workspace {} ({})", ws.name, ws.dir.display()));
    format!("{} started; data folder {}; {workspace}", env!("CARGO_PKG_VERSION"), root.display())
}

/// The error a reply carries, as a JSON-RPC error or as a failed tool call.
fn error_message(response: &Value) -> Option<&str> {
    let tool_error = response["result"]["isError"] == true;
    response["error"]["message"].as_str().or_else(|| tool_error.then(|| response["result"]["content"][0]["text"].as_str()).flatten())
}

/// Rado's data folder, `%APPDATA%\dev.antonis.rado`; `RADO_DATA_DIR` overrides it.
fn data_dir() -> PathBuf {
    std::env::var_os("RADO_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("APPDATA").unwrap_or_default()).join("dev.antonis.rado"))
}

/// One JSON-RPC message in, at most one out; notifications get no reply.
fn handle(root: &Path, now: DateTime<Utc>, line: &str) -> Option<Value> {
    let request: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(e) => return Some(error(Value::Null, -32700, &format!("parse error: {e}"))),
    };
    if request.is_array() {
        return Some(error(Value::Null, -32600, "batches are not supported; send one request per line"));
    }
    let id = request.get("id").cloned()?;
    let params = request.get("params").cloned().unwrap_or(Value::Null);
    let result = match request["method"].as_str().unwrap_or_default() {
        "initialize" => json!({
            "protocolVersion": params["protocolVersion"].as_str().filter(|v| PROTOCOLS.contains(v)).unwrap_or(PROTOCOL),
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "rado", "version": env!("CARGO_PKG_VERSION") },
        }),
        "ping" => json!({}),
        "tools/list" => json!({ "tools": tools() }),
        "tools/call" => match call(root, now, &params) {
            Ok(result) => result,
            Err(message) => return Some(error(id, -32602, &message)),
        },
        other => return Some(error(id, -32601, &format!("unknown method {other}"))),
    };
    Some(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
}

fn error(id: Value, code: i32, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn tools() -> Value {
    json!([
        {
            "name": "list_review_queue",
            "description": "The pull requests waiting on the user's review, from Rado, best-ranked first, on Azure DevOps \
                and GitHub. PRs are grouped by source branch: one branch is one change, even across repos. Hidden PRs, the \
                user's own PRs and PRs already drafted at their current source commit are left out. Each PR has a provider: \
                for azureDevOps, project, repo, repoId and prId are what the Azure DevOps MCP tools take; for gitHub, project \
                is the owner, repo the repository name and number the PR number. sourceCommit is the head commit to review. \
                agentAuthored (GitHub only; always false on Azure DevOps) is true for a PR opened by a bot account or \
                labelled agent, ai, ai-generated or ai-assisted. A reReview PR with lastVotedCommit has new pushes since \
                the user's vote, so review only what changed since that commit; without it, review the whole PR. \
                missingSources names a provider whose last poll failed or came back partial: some of its PRs may be \
                absent, which doesn't mean they are done. Fails when Rado's data is stale; then review nothing. The reply \
                names the workspace and workspaceId it read: the one active in Rado, unless workspace picks another.",
            "inputSchema": {
                "type": "object",
                "properties": { "workspace": workspace_argument() },
                "additionalProperties": false,
            },
            "annotations": { "title": "List review queue", "readOnlyHint": true, "openWorldHint": false },
        },
        {
            "name": "mark_drafted",
            "description": "Records that a draft review now exists for this PR at this source commit, so later runs skip it \
                until someone pushes again. Call it once per PR, after the draft is published, with the prId (not the GitHub \
                PR number) and sourceCommit from list_review_queue. While missingSources is set, a PR that has since dropped \
                out of the queue is still accepted, unless the id is a queued PR's GitHub number at that commit. A queued PR at another \
                commit is refused: list again. Pass the workspaceId list_review_queue returned as workspace, so a switch \
                in Rado between the two calls can't send the mark elsewhere. Changes nothing on Azure DevOps or GitHub.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "prId": { "type": "integer", "description": "The prId from list_review_queue." },
                    "commit": { "type": "string", "description": "The sourceCommit the draft reviewed." },
                    "workspace": workspace_argument(),
                },
                "required": ["prId", "commit"],
                "additionalProperties": false,
            },
            "annotations": { "title": "Mark PR drafted", "readOnlyHint": false, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false },
        },
    ])
}

fn workspace_argument() -> Value {
    json!({
        "type": "string",
        "description": "The workspaceId (or name) your instructions name, or the workspaceId list_review_queue \
            returned; the one active in Rado when left out. Never guess one. The id stays the same when the user \
            renames the workspace.",
    })
}

/// The tool's result, which carries a failed call as `isError`; an unknown tool is a protocol error instead.
fn call(root: &Path, now: DateTime<Utc>, params: &Value) -> Result<Value, String> {
    let args = &params["arguments"];
    let name = params["name"].as_str().unwrap_or_default();
    let in_workspace = |tool: &dyn Fn(&Workspace) -> Result<Value, String>| {
        known_arguments(name, args)?;
        workspace(root, args.get("workspace")).and_then(|ws| named(&ws, tool(&ws)))
    };
    let outcome = match params["name"].as_str() {
        Some("list_review_queue") => in_workspace(&|ws| list(ws, now)),
        Some("mark_drafted") => in_workspace(&|ws| mark_drafted(ws, args)),
        other => return Err(format!("Unknown tool: {}", other.unwrap_or_default())),
    };
    Ok(match outcome {
        Ok(value) => json!({ "content": [{ "type": "text", "text": value.to_string() }] }),
        Err(message) => json!({ "content": [{ "type": "text", "text": message }], "isError": true }),
    })
}

/// Refuses an argument the tool's schema doesn't list: ignoring a misnamed `workspaceId` would use the active
/// workspace instead of the one the agent meant.
fn known_arguments(tool: &str, args: &Value) -> Result<(), String> {
    let tools = tools();
    let schema = tools.as_array().and_then(|all| all.iter().find(|t| t["name"] == tool));
    let (Some(properties), Some(args)) = (schema.and_then(|t| t["inputSchema"]["properties"].as_object()), args.as_object()) else {
        return Ok(());
    };
    match args.keys().find(|key| !properties.contains_key(*key)) {
        Some(key) => Err(format!("{tool} takes no argument {key:?}; it takes {}", properties.keys().cloned().collect::<Vec<_>>().join(", "))),
        None => Ok(()),
    }
}

/// Names the workspace in a tool's reply, an object, or in its error.
fn named(ws: &Workspace, outcome: Result<Value, String>) -> Result<Value, String> {
    outcome
        .map(|mut reply| {
            if let Some(fields) = reply.as_object_mut() {
                fields.insert("workspace".into(), json!(ws.name));
                fields.insert("workspaceId".into(), json!(ws.id));
            }
            reply
        })
        .map_err(|message| format!("Workspace {} (id {}): {message}", ws.name, ws.id))
}

/// The error with what to do about it: a file another program holds needs only a retry, a damaged one `repair`.
fn advise(error: ReadError, repair: &str) -> String {
    match error {
        ReadError::Unreadable(message) => format!("{message}. Try again."),
        ReadError::Invalid(message) => format!("{message}. {repair}"),
    }
}

#[derive(Clone)]
struct Workspace {
    id: String,
    name: String,
    dir: PathBuf,
    active: bool,
}

impl Workspace {
    /// What to do about a missing, damaged or old queue: `if_active`, since only the active workspace is polled.
    fn poll_advice(&self, if_active: &str) -> String {
        if self.active {
            if_active.into()
        } else {
            format!("Rado polls only the active workspace: tell the user to switch to {} in Rado, and review nothing until then.", self.name)
        }
    }
}

/// The workspace `requested` names, by id first, then by name ignoring case; the active one when it is absent. Read
/// on every call, so a switch in Rado is followed without restarting this server. Only ids the settings list are
/// used, so the argument never becomes a path.
fn workspace(root: &Path, requested: Option<&Value>) -> Result<Workspace, String> {
    let settings: Value = queue::read_json_file(&root.join(SETTINGS_FILE))
        .map_err(|e| {
            advise(e, "Fix it, or change any setting in Rado while it runs. Starting Rado on a damaged file sets it aside as settings.unreadable-*.json and starts over with one empty workspace.")
        })?
        .unwrap_or_default();
    let active = settings["activeWorkspace"].as_str();
    let all: Vec<Workspace> = (settings["workspaces"].as_array().into_iter().flatten())
        .filter_map(|w| {
            let id = w["id"].as_str().filter(|id| queue::is_workspace_id(id))?;
            let name = w["name"].as_str().filter(|name| !name.trim().is_empty()).unwrap_or(id);
            Some(Workspace { id: id.into(), name: name.into(), dir: queue::workspace_dir(root, id), active: active == Some(id) })
        })
        .collect();
    let no_workspace = || format!("Rado has no active workspace in {}. Start Rado once to set it up, or set RADO_DATA_DIR to Rado's data folder.", root.display());
    let requested = match requested {
        None | Some(Value::Null) => return all.into_iter().find(|w| w.active).ok_or_else(no_workspace),
        Some(Value::String(requested)) => requested.trim(),
        Some(other) => return Err(format!("workspace must be a workspace's name or id, not {other}")),
    };
    if all.is_empty() {
        return Err(no_workspace());
    }
    if let Some(ws) = all.iter().find(|w| w.id == requested) {
        return Ok(ws.clone());
    }
    let key = requested.to_lowercase();
    let named: Vec<&Workspace> = all.iter().filter(|w| w.name.trim().to_lowercase() == key).collect();
    let listed = |workspaces: &[&Workspace]| workspaces.iter().map(|w| format!("{} (id {})", w.name, w.id)).collect::<Vec<_>>().join(", ");
    match named[..] {
        [ws] => Ok(ws.clone()),
        [] => Err(format!(
            "Rado has no workspace {requested:?}; it has {}. Don't pick another: review nothing and tell the user.",
            listed(&all.iter().collect::<Vec<_>>())
        )),
        _ => Err(format!(
            "{} workspaces are named {requested:?}: {}. Don't pick one: review nothing and tell the user to name a workspaceId in your instructions.",
            named.len(),
            listed(&named)
        )),
    }
}

/// Checks the schema before the shape, so a newer file is reported as newer rather than as damaged.
fn read_queue(ws: &Workspace) -> Result<Queue, String> {
    let path = ws.dir.join(QUEUE_FILE);
    let rewrite = |e| advise(e, &ws.poll_advice("Rado rewrites it after its next poll that reaches a source; start Rado if it isn't running."));
    let file: Value = queue::read_json_file(&path)
        .map_err(rewrite)?
        .ok_or_else(|| {
            let advice = ws.poll_advice("Start Rado, add a token to the workspace, and wait for one refresh.");
            format!("Rado has not written a review queue to {}. {advice}", ws.dir.display())
        })?;
    let schema = file.get("schemaVersion").cloned().unwrap_or(json!(1));
    if schema.as_u64().is_none_or(|v| v > u64::from(QUEUE_SCHEMA)) {
        return Err(format!(
            "{} has schema {schema}, but this rado-mcp {} ({}) reads up to {QUEUE_SCHEMA}. Restart this Claude session. If \
             it still refuses, that exe isn't the newer one: run the Rado installer from Explorer with no Claude session \
             open, or register the installed rado-mcp.exe (a source build: npm run mcp:build).",
            path.display(),
            env!("CARGO_PKG_VERSION"),
            std::env::current_exe().unwrap_or_default().display(),
        ));
    }
    serde_json::from_value(file).map_err(|e| rewrite(ReadError::Invalid(format!("{} is not valid: {e}", path.display()))))
}

/// An unreadable or invalid file is an error, not an empty one: listing would bring every drafted PR back, and
/// marking would replace all recorded drafts with one.
fn read_drafts(path: &Path) -> Result<Drafts, String> {
    queue::read_json_file(path)
        .map(Option::unwrap_or_default)
        .map_err(|e| advise(e, "Delete it to start over; the PRs drafted so far will then be listed again."))
}

fn list(ws: &Workspace, now: DateTime<Utc>) -> Result<Value, String> {
    let queue = read_queue(ws)?;
    let age = (now - queue.refreshed).num_minutes();
    let limit = queue::stale_after_minutes(queue.poll_minutes);
    if age > limit {
        let advice = ws.poll_advice("Rado is not running or cannot reach Azure DevOps or GitHub; start Rado and run again.");
        return Err(format!("Rado's review queue is {age} minutes old (the limit is {limit}). Review nothing from it. {advice}"));
    }
    let drafts = read_drafts(&ws.dir.join(DRAFTS_FILE))?;
    Ok(json!({
        "organization": queue.organization,
        "refreshedMinutesAgo": age,
        "missingSources": queue.missing_sources,
        "branches": queue::pending(&queue, &drafts),
    }))
}

fn mark_drafted(ws: &Workspace, args: &Value) -> Result<Value, String> {
    let pr_id = args["prId"].as_i64().ok_or("prId (an integer) is required")?;
    let commit = args["commit"].as_str().filter(|c| !c.is_empty()).ok_or("commit (the reviewed sourceCommit) is required")?;
    let queue = read_queue(ws)?;
    let dir = &ws.dir;
    // Matching the commit too keeps a PR id from another workspace's organization from being marked here. While a
    // source is missing, a PR listed before it dropped out is accepted, since a draft hides a PR only at this id and
    // commit; a GitHub PR number passed as the id is still refused.
    let prs = || queue.branches.iter().flat_map(|b| &b.prs);
    if let Some(queued) = prs().find(|p| p.pr_id == pr_id) {
        if queued.source_commit.as_deref() != Some(commit) {
            let current = queued.source_commit.as_deref().unwrap_or("no known commit");
            return Err(format!("PR {pr_id} is queued at {current}, not {commit}; list it again and review that commit"));
        }
    } else if queue.missing_sources.is_empty() {
        return Err(format!("PR {pr_id} is not in Rado's review queue; list it again"));
    } else if prs().any(|p| p.number == pr_id && p.source_commit.as_deref() == Some(commit)) {
        return Err(format!("{pr_id} is a PR number; pass the prId from list_review_queue"));
    }
    let _lock = lock_drafts(dir)?;
    let path = dir.join(DRAFTS_FILE);
    let mut drafts = read_drafts(&path)?;
    queue::record_draft(&mut drafts, pr_id, commit);
    queue::write_json_atomic(&path, &drafts)?;
    Ok(json!({ "prId": pr_id, "commit": commit }))
}

/// Waits for other `rado-mcp` processes to finish recording; the lock goes when the file is dropped or its process
/// ends, so a crashed agent can't leave it held.
fn lock_drafts(dir: &Path) -> Result<File, String> {
    let path = dir.join(DRAFTS_LOCK_FILE);
    let file = File::options().create(true).truncate(false).write(true).open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    file.lock().map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(file)
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;
    use queue::tests::{pr, queue as make_queue};
    use queue::{Branch, write_json_atomic};

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("rado-mcp-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn at(minutes_after_refresh: i64) -> DateTime<Utc> {
        make_queue(vec![]).refreshed + chrono::Duration::minutes(minutes_after_refresh)
    }

    /// Writes the settings file as Rado does: `workspaces` as (id, name), and the active one's id.
    fn settings(root: &Path, active: &str, workspaces: &[(&str, &str)]) {
        let workspaces: Vec<Value> = workspaces.iter().map(|(id, name)| json!({ "id": id, "name": name })).collect();
        write_json_atomic(&root.join(SETTINGS_FILE), &json!({ "activeWorkspace": active, "workspaces": workspaces })).unwrap();
    }

    /// A data folder whose active workspace `w1`, named Work, has a queue.
    fn with_queue(name: &str) -> TempDir {
        let dir = TempDir::new(name);
        let workspace = queue::workspace_dir(&dir.0, "w1");
        std::fs::create_dir_all(&workspace).unwrap();
        settings(&dir.0, "w1", &[("w1", "Work")]);
        let q = make_queue(vec![Branch { branch: "ABC-1".into(), prs: vec![pr(1, "c1"), pr(2, "c2")] }]);
        write_json_atomic(&workspace.join(QUEUE_FILE), &q).unwrap();
        dir
    }

    fn send(dir: &Path, now: DateTime<Utc>, request: Value) -> Value {
        handle(dir, now, &request.to_string()).expect("a reply")
    }

    fn call_tool(dir: &Path, now: DateTime<Utc>, name: &str, arguments: Value) -> Value {
        send(dir, now, json!({ "jsonrpc": "2.0", "id": 7, "method": "tools/call", "params": { "name": name, "arguments": arguments } }))["result"].clone()
    }

    fn text(result: &Value) -> Value {
        serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap()
    }

    #[test]
    fn initialize_echoesProtocolAndOffersTools() {
        let dir = TempDir::new("init");
        let reply = send(&dir.0, at(0), json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": "2025-03-26" } }));
        assert_eq!(reply["id"], 1);
        assert_eq!(reply["result"]["protocolVersion"], "2025-03-26");
        assert!(reply["result"]["capabilities"]["tools"].is_object());
    }

    #[test]
    fn initialize_unknownProtocol_answersTheNewestItSpeaks() {
        let dir = TempDir::new("init-unknown");
        let reply = send(&dir.0, at(0), json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": "1.0.0" } }));
        assert_eq!(reply["result"]["protocolVersion"], PROTOCOL);
    }

    #[test]
    fn unknownTool_isAProtocolError() {
        let dir = TempDir::new("unknown-tool");
        let reply = send(&dir.0, at(0), json!({ "jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": { "name": "drop_queue" } }));
        assert_eq!(reply["error"]["code"], -32602);
        assert_eq!(reply["error"]["message"], "Unknown tool: drop_queue");
    }

    #[test]
    fn startup_namesTheDataFolderAndWhatIsWrongWithIt() {
        let dir = with_queue("startup");
        assert!(startup(&dir.0).ends_with(&format!("active workspace Work ({})", queue::workspace_dir(&dir.0, "w1").display())));

        let empty = TempDir::new("startup-empty");
        let line = startup(&empty.0);
        assert!(line.contains(&format!("data folder {}", empty.0.display())) && line.contains("RADO_DATA_DIR"), "{line}");
    }

    #[test]
    fn notification_getsNoReply() {
        let dir = TempDir::new("notify");
        assert_eq!(handle(&dir.0, at(0), r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#), None);
    }

    #[test]
    fn unknownMethod_isMethodNotFound() {
        let dir = TempDir::new("unknown");
        let reply = send(&dir.0, at(0), json!({ "jsonrpc": "2.0", "id": 2, "method": "resources/list" }));
        assert_eq!(reply["error"]["code"], -32601);
    }

    #[test]
    fn toolsList_namesBothTools() {
        let dir = TempDir::new("list");
        let reply = send(&dir.0, at(0), json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/list" }));
        let names: Vec<&str> = reply["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["list_review_queue", "mark_drafted"]);
    }

    #[test]
    fn listReviewQueue_freshQueue_returnsBranches() {
        let dir = with_queue("fresh");
        let result = call_tool(&dir.0, at(5), "list_review_queue", json!({}));
        assert_eq!(result.get("isError"), None);
        let body = text(&result);
        assert_eq!(body["refreshedMinutesAgo"], 5);
        assert_eq!(body["branches"][0]["prs"][1]["prId"], 2);
        assert_eq!(body["branches"][0]["prs"][1]["sourceCommit"], "c2");
        assert_eq!(body["branches"][0]["prs"][1]["provider"], "azureDevOps");
        assert_eq!(body["missingSources"], json!([]));
        assert_eq!((&body["workspace"], &body["workspaceId"]), (&json!("Work"), &json!("w1")));
    }

    #[test]
    fn listReviewQueue_staleQueue_refuses() {
        let dir = with_queue("stale");
        let result = call_tool(&dir.0, at(31), "list_review_queue", json!({}));
        assert_eq!(result["isError"], true);
        assert!(result["content"][0]["text"].as_str().unwrap().contains("31 minutes old"));
    }

    #[test]
    fn listReviewQueue_noQueueFile_saysToStartRado() {
        let dir = with_queue("missing");
        std::fs::remove_file(queue::workspace_dir(&dir.0, "w1").join(QUEUE_FILE)).unwrap();
        let result = call_tool(&dir.0, at(0), "list_review_queue", json!({}));
        assert_eq!(result["isError"], true);
        assert!(result["content"][0]["text"].as_str().unwrap().contains("has not written a review queue"));
    }

    #[test]
    fn listReviewQueue_invalidQueue_saysSoRatherThanMissing() {
        let dir = with_queue("invalid");
        std::fs::write(queue::workspace_dir(&dir.0, "w1").join(QUEUE_FILE), "{").unwrap();
        let text = call_tool(&dir.0, at(0), "list_review_queue", json!({}))["content"][0]["text"].as_str().unwrap().to_string();
        assert!(text.contains("is not valid") && text.contains("next poll"), "{text}");
    }

    #[test]
    fn listReviewQueue_queueFromBeforeTheSchema_isRead() {
        let dir = with_queue("unversioned");
        let path = queue::workspace_dir(&dir.0, "w1").join(QUEUE_FILE);
        let mut file: Value = queue::read_json(&path).unwrap();
        file.as_object_mut().unwrap().remove("schemaVersion");
        std::fs::write(&path, file.to_string()).unwrap();

        assert_eq!(text(&call_tool(&dir.0, at(1), "list_review_queue", json!({})))["branches"][0]["prs"][0]["prId"], 1);
    }

    #[test]
    fn listReviewQueue_newerSchema_isRefusedAsNewerEvenWhenItsShapeChanged() {
        let dir = with_queue("newer");
        let path = queue::workspace_dir(&dir.0, "w1").join(QUEUE_FILE);
        for (schema, shown) in [(json!(QUEUE_SCHEMA + 1), (QUEUE_SCHEMA + 1).to_string()), (json!("2"), r#""2""#.to_string())] {
            std::fs::write(&path, json!({ "schemaVersion": schema, "queue": [] }).to_string()).unwrap();

            for (tool, arguments) in [("list_review_queue", json!({})), ("mark_drafted", json!({ "prId": 1, "commit": "c1" }))] {
                let result = call_tool(&dir.0, at(1), tool, arguments);
                assert!(result["content"][0]["text"].as_str().unwrap().contains(&format!("has schema {shown}")), "{tool}: {result}");
            }
        }
    }

    #[test]
    fn invalidDraftsFile_isAnErrorForBothTools_andIsKept() {
        let dir = with_queue("bad-drafts");
        let path = queue::workspace_dir(&dir.0, "w1").join(DRAFTS_FILE);
        std::fs::write(&path, "{\"1\":").unwrap();

        let listed = call_tool(&dir.0, at(1), "list_review_queue", json!({}));
        let marked = call_tool(&dir.0, at(1), "mark_drafted", json!({ "prId": 2, "commit": "c2" }));

        for result in [&listed, &marked] {
            assert_eq!(result["isError"], true, "{result}");
            assert!(result["content"][0]["text"].as_str().unwrap().contains("is not valid"), "{result}");
        }
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{\"1\":");
    }

    #[test]
    fn markDrafted_thenList_leavesThePrOut() {
        let dir = with_queue("mark");
        let marked = call_tool(&dir.0, at(1), "mark_drafted", json!({ "prId": 1, "commit": "c1" }));
        assert_eq!(marked.get("isError"), None);

        let body = text(&call_tool(&dir.0, at(2), "list_review_queue", json!({})));

        let ids: Vec<i64> = body["branches"][0]["prs"].as_array().unwrap().iter().map(|p| p["prId"].as_i64().unwrap()).collect();
        assert_eq!(ids, [2]);
    }

    #[test]
    fn markDrafted_agentsMarkingAtOnce_allMarksAreKept() {
        const AGENTS: i64 = 8;
        const MARKS_EACH: i64 = 8;
        let dir = TempDir::new("parallel");
        let workspace = queue::workspace_dir(&dir.0, "w1");
        std::fs::create_dir_all(&workspace).unwrap();
        settings(&dir.0, "w1", &[("w1", "Work")]);
        let prs = (1..=AGENTS * MARKS_EACH).map(|id| pr(id, &format!("c{id}"))).collect();
        write_json_atomic(&workspace.join(QUEUE_FILE), &make_queue(vec![Branch { branch: "x".into(), prs }])).unwrap();
        let start = std::sync::Barrier::new(AGENTS as usize);

        std::thread::scope(|s| {
            for agent in 0..AGENTS {
                let (root, start) = (&dir.0, &start);
                s.spawn(move || {
                    start.wait();
                    for id in (1..=MARKS_EACH).map(|n| agent * MARKS_EACH + n) {
                        let result = call_tool(root, at(1), "mark_drafted", json!({ "prId": id, "commit": format!("c{id}") }));
                        assert_eq!(result.get("isError"), None, "{result}");
                    }
                });
            }
        });

        let drafts: Drafts = queue::read_json(&workspace.join(DRAFTS_FILE)).unwrap();
        assert_eq!(drafts.len() as i64, AGENTS * MARKS_EACH);
    }

    /// Windows only: elsewhere a file lock doesn't stop a read.
    #[cfg(windows)]
    #[test]
    fn markDrafted_draftsFileLockedByAnotherProgram_isAnErrorAndKeepsTheDrafts() {
        let dir = with_queue("locked");
        let path = queue::workspace_dir(&dir.0, "w1").join(DRAFTS_FILE);
        write_json_atomic(&path, &Drafts::from([(9, "c9".to_string())])).unwrap();
        let held = File::options().read(true).write(true).open(&path).unwrap();
        held.lock().unwrap();

        let result = call_tool(&dir.0, at(1), "mark_drafted", json!({ "prId": 1, "commit": "c1" }));
        drop(held);

        let text = result["content"][0]["text"].as_str().unwrap();
        assert!(text.starts_with("Workspace Work (id w1): Cannot read") && text.ends_with("Try again."), "{result}");
        assert_eq!(queue::read_json::<Drafts>(&path).unwrap(), Drafts::from([(9, "c9".to_string())]));
    }

    #[test]
    fn markDrafted_prThatLeftTheQueueAndCameBack_isStillSkipped() {
        let dir = with_queue("came-back");
        let path = queue::workspace_dir(&dir.0, "w1").join(QUEUE_FILE);
        let full: Queue = queue::read_json(&path).unwrap();
        assert_eq!(call_tool(&dir.0, at(1), "mark_drafted", json!({ "prId": 1, "commit": "c1" })).get("isError"), None);
        write_json_atomic(&path, &make_queue(vec![Branch { branch: "ABC-1".into(), prs: vec![pr(2, "c2")] }])).unwrap();
        assert_eq!(call_tool(&dir.0, at(1), "mark_drafted", json!({ "prId": 2, "commit": "c2" })).get("isError"), None);

        write_json_atomic(&path, &full).unwrap();

        assert_eq!(text(&call_tool(&dir.0, at(2), "list_review_queue", json!({})))["branches"], json!([]));
    }

    #[test]
    fn markDrafted_prThatDroppedOutWithItsSource_isStillRecorded() {
        let dir = with_queue("dropped");
        let path = queue::workspace_dir(&dir.0, "w1");
        let q = Queue { missing_sources: vec![queue::AZURE_DEV_OPS.into()], ..make_queue(vec![]) };
        write_json_atomic(&path.join(QUEUE_FILE), &q).unwrap();

        assert_eq!(call_tool(&dir.0, at(1), "mark_drafted", json!({ "prId": 5, "commit": "c5" })).get("isError"), None);

        assert_eq!(queue::read_json::<Drafts>(&path.join(DRAFTS_FILE)).unwrap(), Drafts::from([(5, "c5".to_string())]));
    }

    #[test]
    fn markDrafted_whileASourceIsMissing_aPrNumberAsTheId_isAnError_butASiblingAtTheSameCommitIsRecorded() {
        let dir = with_queue("wrong-id");
        let path = queue::workspace_dir(&dir.0, "w1");
        let mut q: Queue = queue::read_json(&path.join(QUEUE_FILE)).unwrap();
        q.missing_sources = vec![queue::GITHUB.into()];
        q.branches[0].prs[0].number = 12;
        write_json_atomic(&path.join(QUEUE_FILE), &q).unwrap();

        let number = call_tool(&dir.0, at(1), "mark_drafted", json!({ "prId": 12, "commit": "c1" }));
        assert_eq!(number["isError"], true);
        assert!(!path.join(DRAFTS_FILE).exists());

        let sibling = call_tool(&dir.0, at(1), "mark_drafted", json!({ "prId": 5, "commit": "c1" }));
        assert_eq!(sibling.get("isError"), None);
        let same_number_other_commit = call_tool(&dir.0, at(1), "mark_drafted", json!({ "prId": 12, "commit": "c9" }));
        assert_eq!(same_number_other_commit.get("isError"), None);
        let drafts: Drafts = queue::read_json(&path.join(DRAFTS_FILE)).unwrap();
        assert_eq!(drafts, Drafts::from([(5, "c1".to_string()), (12, "c9".to_string())]));
    }

    #[test]
    fn markDrafted_whileASourceIsMissing_queuedPrAtAnotherCommit_isAnError() {
        let dir = with_queue("stale-commit");
        let path = queue::workspace_dir(&dir.0, "w1");
        let mut q: Queue = queue::read_json(&path.join(QUEUE_FILE)).unwrap();
        q.missing_sources = vec![queue::GITHUB.into()];
        write_json_atomic(&path.join(QUEUE_FILE), &q).unwrap();

        let result = call_tool(&dir.0, at(1), "mark_drafted", json!({ "prId": 1, "commit": "old" }));

        assert_eq!(result["isError"], true);
        assert!(result["content"][0]["text"].as_str().unwrap().contains("queued at c1"));
    }

    #[test]
    fn markDrafted_prNotInQueue_isAnError() {
        let dir = with_queue("notqueued");
        let result = call_tool(&dir.0, at(1), "mark_drafted", json!({ "prId": 99, "commit": "c" }));
        assert_eq!(result["isError"], true);
    }

    #[test]
    fn markDrafted_sameIdAtAnotherCommit_isAnError() {
        let dir = with_queue("othercommit");
        let result = call_tool(&dir.0, at(1), "mark_drafted", json!({ "prId": 1, "commit": "c2" }));
        assert_eq!(result["isError"], true);
        assert!(!queue::workspace_dir(&dir.0, "w1").join(DRAFTS_FILE).exists());
    }

    /// Workspaces `a1` (Alpha) and `b2` (Beta), each queueing one PR at commit `c`: PR 1 and PR 2.
    fn two_workspaces(name: &str) -> (TempDir, PathBuf, PathBuf) {
        let root = TempDir::new(name);
        let (a, b) = (queue::workspace_dir(&root.0, "a1"), queue::workspace_dir(&root.0, "b2"));
        for (dir, id) in [(&a, 1), (&b, 2)] {
            std::fs::create_dir_all(dir).unwrap();
            write_json_atomic(&dir.join(QUEUE_FILE), &make_queue(vec![Branch { branch: "x".into(), prs: vec![pr(id, "c")] }])).unwrap();
        }
        (root, a, b)
    }

    fn error_text(result: &Value) -> String {
        assert_eq!(result["isError"], true, "{result}");
        result["content"][0]["text"].as_str().unwrap().to_string()
    }

    #[test]
    fn activeWorkspace_listsAndMarksInItsOwnFolder() {
        let (root, a, b) = two_workspaces("workspaces");
        let activate = |id: &str| settings(&root.0, id, &[("a1", "Alpha"), ("b2", "Beta")]);

        activate("b2");
        assert_eq!(text(&call_tool(&root.0, at(1), "list_review_queue", json!({})))["branches"][0]["prs"][0]["prId"], 2);
        assert_eq!(call_tool(&root.0, at(1), "mark_drafted", json!({ "prId": 2, "commit": "c" })).get("isError"), None);
        assert!(b.join(DRAFTS_FILE).exists() && !a.join(DRAFTS_FILE).exists());

        activate("a1");
        assert_eq!(text(&call_tool(&root.0, at(1), "list_review_queue", json!({})))["branches"][0]["prs"][0]["prId"], 1);
    }

    #[test]
    fn workspaceArgument_byIdOrName_usesThatOneWhileAnotherIsActive() {
        let (root, a, b) = two_workspaces("pick");
        settings(&root.0, "a1", &[("a1", "Alpha"), ("b2", "Beta")]);

        for requested in ["b2", " b2 ", "Beta", " beta "] {
            let body = text(&call_tool(&root.0, at(1), "list_review_queue", json!({ "workspace": requested })));
            assert_eq!((&body["workspace"], &body["workspaceId"]), (&json!("Beta"), &json!("b2")), "{requested}");
            assert_eq!(body["branches"][0]["prs"][0]["prId"], 2);
        }
        let marked = text(&call_tool(&root.0, at(1), "mark_drafted", json!({ "prId": 2, "commit": "c", "workspace": "b2" })));

        assert_eq!(marked["workspaceId"], "b2");
        assert!(b.join(DRAFTS_FILE).exists() && !a.join(DRAFTS_FILE).exists());
    }

    #[test]
    fn workspaceArgument_unknown_isAnErrorListingTheWorkspaces() {
        let (root, _, _) = two_workspaces("unknown-ws");
        settings(&root.0, "a1", &[("a1", "Alpha"), ("b2", "Beta"), ("../..", "Escape")]);

        for requested in ["Gamma", "", "../..", "Escape"] {
            let text = error_text(&call_tool(&root.0, at(1), "list_review_queue", json!({ "workspace": requested })));
            assert_eq!(
                text,
                format!("Rado has no workspace {requested:?}; it has Alpha (id a1), Beta (id b2). Don't pick another: review nothing and tell the user.")
            );
        }
    }

    #[test]
    fn workspaceArgument_nameTwoWorkspacesShare_asksForTheId() {
        let (root, a, b) = two_workspaces("shared-name");
        settings(&root.0, "a1", &[("a1", "Work"), ("b2", "work")]);

        let text = error_text(&call_tool(&root.0, at(1), "mark_drafted", json!({ "prId": 1, "commit": "c", "workspace": "Work" })));

        assert_eq!(
            text,
            r#"2 workspaces are named "Work": Work (id a1), work (id b2). Don't pick one: review nothing and tell the user to name a workspaceId in your instructions."#
        );
        assert!(!a.join(DRAFTS_FILE).exists() && !b.join(DRAFTS_FILE).exists());
    }

    #[test]
    fn unknownArgument_isAnErrorNotTheActiveWorkspace() {
        let (root, a, _) = two_workspaces("unknown-arg");
        let q = Queue { missing_sources: vec![queue::GITHUB.into()], ..make_queue(vec![]) };
        write_json_atomic(&a.join(QUEUE_FILE), &q).unwrap();
        settings(&root.0, "a1", &[("a1", "Alpha"), ("b2", "Beta")]);

        let listed = error_text(&call_tool(&root.0, at(1), "list_review_queue", json!({ "workspaceId": "b2" })));
        let marked = error_text(&call_tool(&root.0, at(1), "mark_drafted", json!({ "prId": 2, "commit": "c", "workspaceId": "b2" })));

        assert_eq!(listed, r#"list_review_queue takes no argument "workspaceId"; it takes workspace"#);
        assert_eq!(marked, r#"mark_drafted takes no argument "workspaceId"; it takes commit, prId, workspace"#);
        assert!(!a.join(DRAFTS_FILE).exists());
    }

    #[test]
    fn workspaceArgument_notAString_isAnErrorNotTheActiveWorkspace() {
        let dir = with_queue("not-a-string");
        let text = error_text(&call_tool(&dir.0, at(1), "mark_drafted", json!({ "prId": 1, "commit": "c1", "workspace": 1 })));
        assert_eq!(text, "workspace must be a workspace's name or id, not 1");
        assert!(!queue::workspace_dir(&dir.0, "w1").join(DRAFTS_FILE).exists());
    }

    #[test]
    fn workspaceArgument_inactiveWorkspacesOldOrMissingQueue_saysToSwitchToIt() {
        let (root, _, b) = two_workspaces("inactive");
        settings(&root.0, "a1", &[("a1", "Alpha"), ("b2", "Beta")]);
        let switch = "Rado polls only the active workspace: tell the user to switch to Beta in Rado, and review nothing until then.";

        let stale = error_text(&call_tool(&root.0, at(31), "list_review_queue", json!({ "workspace": "Beta" })));
        assert!(stale.starts_with("Workspace Beta (id b2): Rado's review queue is 31 minutes old") && stale.ends_with(switch), "{stale}");

        std::fs::write(b.join(QUEUE_FILE), "{").unwrap();
        let damaged = error_text(&call_tool(&root.0, at(1), "list_review_queue", json!({ "workspace": "Beta" })));
        assert!(damaged.contains("is not valid") && damaged.ends_with(switch), "{damaged}");

        std::fs::remove_file(b.join(QUEUE_FILE)).unwrap();
        let missing = error_text(&call_tool(&root.0, at(1), "list_review_queue", json!({ "workspace": "Beta" })));
        assert!(missing.contains("has not written a review queue") && missing.ends_with(switch), "{missing}");
    }

    #[test]
    fn activeWorkspace_missingOrUnsafe_isAnError() {
        let cases = [None, Some(json!({ "activeWorkspace": "../.." })), Some(json!({ "activeWorkspace": "../..", "workspaces": [{ "id": "../..", "name": "x" }] }))];
        for file in cases {
            let dir = TempDir::new("unsafe");
            write_json_atomic(&dir.0.join(QUEUE_FILE), &make_queue(vec![])).unwrap();
            if let Some(file) = &file {
                write_json_atomic(&dir.0.join(SETTINGS_FILE), file).unwrap();
            }
            let result = call_tool(&dir.0, at(1), "list_review_queue", json!({}));
            assert!(error_text(&result).contains("no active workspace"), "{file:?}");
        }
    }

    #[test]
    fn activeWorkspace_invalidSettings_saysSoAndHowToRepairIt() {
        let dir = TempDir::new("bad-settings");
        std::fs::write(dir.0.join(queue::SETTINGS_FILE), "{").unwrap();
        let text = call_tool(&dir.0, at(1), "list_review_queue", json!({}))["content"][0]["text"].as_str().unwrap().to_string();
        assert!(text.contains("settings.json is not valid") && text.contains("change any setting in Rado while it runs."), "{text}");
    }

    #[test]
    fn batch_isRefusedWithOneError() {
        let dir = TempDir::new("batch");
        let reply = send(&dir.0, at(0), json!([{ "jsonrpc": "2.0", "id": 1, "method": "ping" }]));
        assert_eq!(reply["error"]["code"], -32600);
    }

    #[test]
    fn markDrafted_missingCommit_isAnError() {
        let dir = with_queue("nocommit");
        let result = call_tool(&dir.0, at(1), "mark_drafted", json!({ "prId": 1 }));
        assert_eq!(result["isError"], true);
    }
}
