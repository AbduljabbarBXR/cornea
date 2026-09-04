//! End-to-end tests that exercise the compiled `cornea` binary (CLI + MCP).

use std::io::Write;
use std::process::{Command, Stdio};

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_cornea"))
}

#[test]
fn cli_reports_bugs_in_fixture() {
    let out = bin()
        .args(["tests/fixtures/sample-bugs.html", "360"])
        .output()
        .expect("run CLI on fixture");
    assert!(out.status.success(), "CLI should exit 0");
    let stdout = String::from_utf8(out.stdout).expect("utf8 stdout");
    let report: serde_json::Value =
        serde_json::from_str(&stdout).expect("CLI output should be valid JSON");
    assert!(!report["report"]["overlaps"].as_array().unwrap().is_empty());
    assert!(!report["report"]["overflows"].as_array().unwrap().is_empty());
    assert!(report["report"]["quality"]["score"].as_f64().unwrap() < 1.0);
    assert!(
        report["est_tokens"].as_u64().unwrap() > 0,
        "token estimate present"
    );
}

#[test]
fn cli_without_args_shows_usage() {
    let out = bin().output().expect("run with no args");
    assert_eq!(
        out.status.code(),
        Some(2),
        "no args should print usage and exit 2"
    );
}

#[test]
fn cli_version_flag_reports_version() {
    let out = bin().args(["--version"]).output().expect("run --version");
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).expect("utf8 stdout");
    assert!(
        stdout.starts_with("cornea "),
        "expected 'cornea <version>', got {:?}",
        stdout
    );
}

#[test]
fn mcp_initialize_and_tools_list() {
    let mut child = bin()
        .arg("--serve")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn MCP server");
    {
        let stdin = child.stdin.as_mut().unwrap();
        stdin
            .write_all(
                b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{}}\n\
                  {\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\"}\n",
            )
            .unwrap();
        stdin.flush().unwrap();
    }
    drop(child.stdin.take()); // close stdin so the server exits after processing
    let mut buf = String::new();
    use std::io::Read;
    child
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut buf)
        .unwrap();
    child.wait().unwrap();
    let lines: Vec<&str> = buf.lines().collect();
    assert_eq!(lines.len(), 2, "expected init + tools/list responses");
    let init: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(init["id"], 1);
    assert!(init["result"]["serverInfo"]["name"] == "cornea");
    let tools: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
    let names: Vec<&str> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    for expected in [
        "layout.inspect",
        "layout.overlaps",
        "layout.overflow",
        "layout.contrast",
        "layout.quality",
        "layout.fidelity",
    ] {
        assert!(names.contains(&expected), "missing tool {}", expected);
    }
}

#[test]
fn mcp_tool_call_detects_overlap() {
    let mut child = bin()
        .arg("--serve")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn MCP server");
    let call = "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"layout.overlaps\",\"arguments\":{\"html\":\"<div style=\\\"position:absolute;left:0;top:0;width:100;height:100\\\">A</div><div style=\\\"position:absolute;left:50;top:50;width:100;height:100\\\">B</div>\",\"width\":360}}}\n";
    {
        let stdin = child.stdin.as_mut().unwrap();
        stdin.write_all(call.as_bytes()).unwrap();
        stdin.flush().unwrap();
    }
    drop(child.stdin.take()); // close stdin so the server exits
    let mut buf = String::new();
    use std::io::Read;
    child
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut buf)
        .unwrap();
    child.wait().unwrap();
    let resp: serde_json::Value = serde_json::from_str(buf.lines().next().unwrap()).unwrap();
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    let arr: serde_json::Value = serde_json::from_str(text).unwrap();
    assert!(
        !arr.as_array().unwrap().is_empty(),
        "expected an overlap via MCP"
    );
}
