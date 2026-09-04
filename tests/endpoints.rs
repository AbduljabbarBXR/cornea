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
                  {\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\"}\n\
                  {\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"ping\"}\n",
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
    assert_eq!(
        lines.len(),
        3,
        "expected init + tools/list + ping responses"
    );
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
    let ping: serde_json::Value = serde_json::from_str(lines[2]).unwrap();
    assert_eq!(ping["id"], 3, "ping response echoes the request id");
    assert!(
        ping["result"]
            .as_object()
            .map(|o| o.is_empty())
            .unwrap_or(false),
        "ping must answer with an empty result, got: {}",
        lines[2]
    );
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

#[test]
fn cli_js_flag_runs_inline_script_and_detects_built_overlap() {
    // A page whose only content is built by inline JS (two overlapping absolute
    // boxes). Without --js there is nothing to overlap; with --js the engine
    // must detect the collision the script creates.
    let js_html = "<!DOCTYPE html><html><body><script>\
        var a=document.createElement('div');a.style.position='absolute';a.style.left='0px';a.style.top='0px';a.style.width='200px';a.style.height='100px';a.textContent='A';document.body.appendChild(a);\
        var b=document.createElement('div');b.style.position='absolute';b.style.left='120px';b.style.top='50px';b.style.width='200px';b.style.height='100px';b.textContent='B';document.body.appendChild(b);\
        </script></body></html>";

    let dir = std::env::temp_dir();
    let path = dir.join("cornea_js_endtoend.html");
    std::fs::write(&path, js_html).expect("write temp js page");

    let off = bin()
        .arg(&path)
        .arg("360")
        .output()
        .expect("run CLI without --js");
    assert!(off.status.success());
    let off_report: serde_json::Value =
        serde_json::from_str(&String::from_utf8(off.stdout).unwrap()).unwrap();
    let off_overlaps = off_report["report"]["overlaps"].as_array().unwrap().len();

    let on = bin()
        .arg(&path)
        .arg("360")
        .arg("--js")
        .output()
        .expect("run CLI with --js");
    assert!(on.status.success());
    let on_report: serde_json::Value =
        serde_json::from_str(&String::from_utf8(on.stdout).unwrap()).unwrap();
    let on_overlaps = on_report["report"]["overlaps"].as_array().unwrap().len();

    assert_eq!(
        off_overlaps, 0,
        "without --js the script-built boxes do not exist"
    );
    assert_eq!(
        on_overlaps, 1,
        "with --js the two absolute boxes the script builds must overlap"
    );

    let _ = std::fs::remove_file(&path);
}

#[test]
fn cli_height_flag_detects_below_fold_clipping() {
    let dir = std::env::temp_dir();
    let path = dir.join("cornea_height_e2e.html");
    std::fs::write(
        &path,
        "<html><body><div style=\"width:50;height:500\">tall</div></body></html>",
    )
    .expect("write temp page");
    // unbounded page (no height): nothing clips vertically
    let plain = bin().arg(&path).arg("360").output().expect("run CLI");
    assert!(plain.status.success());
    let plain_report: serde_json::Value =
        serde_json::from_str(&String::from_utf8(plain.stdout).unwrap()).unwrap();
    assert!(
        !plain_report["report"]["overflows"]
            .as_array()
            .unwrap()
            .iter()
            .any(|o| o["detail"]
                .as_str()
                .unwrap_or("")
                .contains("exceeds viewport height")),
        "no height means an unbounded page"
    );
    // fixed 100px viewport: the 500px tall element clips below the fold
    let folded = bin()
        .arg(&path)
        .arg("360")
        .arg("100")
        .output()
        .expect("run CLI with height");
    assert!(folded.status.success());
    let folded_report: serde_json::Value =
        serde_json::from_str(&String::from_utf8(folded.stdout).unwrap()).unwrap();
    assert!(
        folded_report["report"]["overflows"]
            .as_array()
            .unwrap()
            .iter()
            .any(|o| o["kind"] == "clipped"
                && o["detail"]
                    .as_str()
                    .unwrap_or("")
                    .contains("exceeds viewport height 100")),
        "fixed viewport must flag the below fold element: {}",
        folded_report
    );
    let _ = std::fs::remove_file(&path);
}
