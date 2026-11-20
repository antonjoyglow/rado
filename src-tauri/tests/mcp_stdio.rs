//! Runs the built `rado-mcp` the way Claude Code does: stdout must carry JSON-RPC replies only, the log goes to stderr.

use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn rado_mcp_logs_to_stderr_and_keeps_stdout_for_the_protocol() {
    let data = std::env::temp_dir().join(format!("rado-mcp-stdio-{}", std::process::id()));
    std::fs::create_dir_all(&data).unwrap();
    let mut server = Command::new(env!("CARGO_BIN_EXE_rado-mcp"))
        .env("RADO_DATA_DIR", &data)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let requests = [
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25"}}"#,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"list_review_queue","arguments":{}}}"#,
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"nope"}}"#,
    ];
    writeln!(server.stdin.take().unwrap(), "{}", requests.join("\n")).unwrap();

    let output = server.wait_with_output().unwrap();
    let _ = std::fs::remove_dir_all(&data);

    let replies: Vec<serde_json::Value> =
        String::from_utf8(output.stdout).unwrap().lines().map(|line| serde_json::from_str(line).unwrap()).collect();
    assert_eq!(replies.iter().map(|r| r["id"].as_i64().unwrap()).collect::<Vec<_>>(), [1, 2, 3]);
    assert_eq!(replies[0]["result"]["protocolVersion"], "2025-11-25");
    let log = String::from_utf8(output.stderr).unwrap();
    let lines: Vec<&str> = log.lines().collect();
    assert_eq!(lines.len(), 3, "a start line and one per failed call: {log}");
    assert!(lines.iter().all(|line| line.starts_with("rado-mcp: ")), "{log}");
    assert!(lines[0].contains("started; data folder"), "{log}");
}

#[test]
fn rado_mcp_answers_a_line_that_isnt_utf8_and_keeps_serving() {
    let data = std::env::temp_dir().join(format!("rado-mcp-stdio-utf8-{}", std::process::id()));
    std::fs::create_dir_all(&data).unwrap();
    let mut server = Command::new(env!("CARGO_BIN_EXE_rado-mcp"))
        .env("RADO_DATA_DIR", &data)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = server.stdin.take().unwrap();
    stdin.write_all(b"\xff\xfe\n").unwrap();
    stdin.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"ping\"}\r\n").unwrap();
    drop(stdin);

    let output = server.wait_with_output().unwrap();
    let _ = std::fs::remove_dir_all(&data);

    let replies: Vec<serde_json::Value> =
        String::from_utf8(output.stdout).unwrap().lines().map(|line| serde_json::from_str(line).unwrap()).collect();
    assert_eq!(replies.len(), 2, "{replies:?}");
    assert_eq!((replies[0]["id"].clone(), replies[0]["error"]["code"].clone()), (serde_json::Value::Null, serde_json::json!(-32700)));
    assert_eq!((replies[1]["id"].clone(), replies[1]["result"].clone()), (serde_json::json!(2), serde_json::json!({})));
}
