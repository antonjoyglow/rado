//! Rado's MCP server, for scheduled review agents. It serves the review queue that the Rado app writes after each
//! poll (`queue.json`) over stdio, and records which PRs already have a draft review. It never reads the PAT;
//! the review itself goes through the Azure DevOps MCP server.

#[allow(dead_code)]
#[path = "../queue.rs"]
mod queue;

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde_json::{Value, json};

use queue::{DRAFTS_FILE, Drafts, QUEUE_FILE, Queue};

const PROTOCOL: &str = "2025-06-18";

fn main() {
    let dir = data_dir();
    let mut stdout = std::io::stdout().lock();
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        if let Some(response) = handle(&dir, Utc::now(), &line) {
            let _ = writeln!(stdout, "{response}");
            let _ = stdout.flush();
        }
    }
}

/// Rado's data folder, `%APPDATA%\dev.antonis.rado`; `RADO_DATA_DIR` overrides it.
fn data_dir() -> PathBuf {
    std::env::var_os("RADO_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("APPDATA").unwrap_or_default()).join("dev.antonis.rado"))
}

/// One JSON-RPC message in, at most one out; notifications get no reply.
fn handle(dir: &Path, now: DateTime<Utc>, line: &str) -> Option<Value> {
    let request: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(e) => return Some(error(Value::Null, -32700, &format!("parse error: {e}"))),
    };
    let id = request.get("id").cloned()?;
    let params = request.get("params").cloned().unwrap_or(Value::Null);
    let result = match request["method"].as_str().unwrap_or_default() {
        "initialize" => json!({
            "protocolVersion": params["protocolVersion"].as_str().unwrap_or(PROTOCOL),
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "rado", "version": env!("CARGO_PKG_VERSION") },
        }),
        "ping" => json!({}),
        "tools/list" => json!({ "tools": tools() }),
        "tools/call" => call(dir, now, &params),
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
            "description": "The Azure DevOps pull requests waiting on the user's review, from Rado, best-ranked first. \
                PRs are grouped by source branch: one branch is one change, even across repos. Hidden PRs, the user's own PRs \
                and PRs already drafted at their current source commit are left out. Each PR has project, repo, repoId, prId \
                and sourceCommit for the Azure DevOps MCP tools. A reReview PR has new pushes since the user's vote: \
                lastVotedCommit is the commit they voted on, so review only what changed since then. \
                Fails when Rado's data is stale; then review nothing.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false },
            "annotations": { "title": "List review queue", "readOnlyHint": true, "openWorldHint": false },
        },
        {
            "name": "mark_drafted",
            "description": "Records that a draft review now exists for this PR at this source commit, so later runs skip it \
                until someone pushes again. Call it once per PR, after the draft is published. Changes nothing in Azure DevOps.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "prId": { "type": "integer", "description": "The prId from list_review_queue." },
                    "commit": { "type": "string", "description": "The sourceCommit the draft reviewed." },
                },
                "required": ["prId", "commit"],
                "additionalProperties": false,
            },
            "annotations": { "title": "Mark PR drafted", "readOnlyHint": false, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false },
        },
    ])
}

fn call(dir: &Path, now: DateTime<Utc>, params: &Value) -> Value {
    let outcome = match params["name"].as_str() {
        Some("list_review_queue") => list(dir, now),
        Some("mark_drafted") => mark_drafted(dir, &params["arguments"]),
        other => Err(format!("Unknown tool {other:?}")),
    };
    match outcome {
        Ok(value) => json!({ "content": [{ "type": "text", "text": value.to_string() }] }),
        Err(message) => json!({ "content": [{ "type": "text", "text": message }], "isError": true }),
    }
}

fn read_queue(dir: &Path) -> Result<Queue, String> {
    queue::read_json(&dir.join(QUEUE_FILE)).ok_or_else(|| {
        format!("Rado has not written a review queue to {}. Start Rado, add its PAT, and wait for one refresh.", dir.display())
    })
}

fn list(dir: &Path, now: DateTime<Utc>) -> Result<Value, String> {
    let queue = read_queue(dir)?;
    let age = (now - queue.refreshed).num_minutes();
    let limit = queue::stale_after_minutes(queue.poll_minutes);
    if age > limit {
        return Err(format!(
            "Rado's review queue is {age} minutes old (the limit is {limit}): Rado is not running or cannot reach Azure DevOps. \
             Review nothing from it; start Rado and run again."
        ));
    }
    let drafts: Drafts = queue::read_json(&dir.join(DRAFTS_FILE)).unwrap_or_default();
    Ok(json!({
        "organization": queue.organization,
        "refreshedMinutesAgo": age,
        "branches": queue::pending(&queue, &drafts),
    }))
}

fn mark_drafted(dir: &Path, args: &Value) -> Result<Value, String> {
    let pr_id = args["prId"].as_i64().ok_or("prId (an integer) is required")?;
    let commit = args["commit"].as_str().filter(|c| !c.is_empty()).ok_or("commit (the reviewed sourceCommit) is required")?;
    let queue = read_queue(dir)?;
    if !queue.contains(pr_id) {
        return Err(format!("PR {pr_id} is not in Rado's review queue"));
    }
    let path = dir.join(DRAFTS_FILE);
    let mut drafts: Drafts = queue::read_json(&path).unwrap_or_default();
    drafts.insert(pr_id, commit.to_string());
    queue::prune(&mut drafts, &queue);
    queue::write_json_atomic(&path, &drafts)?;
    Ok(json!({ "prId": pr_id, "commit": commit }))
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

    fn with_queue(name: &str) -> TempDir {
        let dir = TempDir::new(name);
        let q = make_queue(vec![Branch { branch: "ABC-1".into(), prs: vec![pr(1, "c1"), pr(2, "c2")] }]);
        write_json_atomic(&dir.0.join(QUEUE_FILE), &q).unwrap();
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
        let dir = TempDir::new("missing");
        let result = call_tool(&dir.0, at(0), "list_review_queue", json!({}));
        assert_eq!(result["isError"], true);
        assert!(result["content"][0]["text"].as_str().unwrap().contains("Start Rado"));
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
    fn markDrafted_prNotInQueue_isAnError() {
        let dir = with_queue("notqueued");
        let result = call_tool(&dir.0, at(1), "mark_drafted", json!({ "prId": 99, "commit": "c" }));
        assert_eq!(result["isError"], true);
    }

    #[test]
    fn markDrafted_missingCommit_isAnError() {
        let dir = with_queue("nocommit");
        let result = call_tool(&dir.0, at(1), "mark_drafted", json!({ "prId": 1 }));
        assert_eq!(result["isError"], true);
    }
}
