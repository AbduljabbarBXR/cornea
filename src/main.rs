use cornea::{analyze, inspect, layout, rest};
use serde::Serialize;
use serde_json::json;
use std::io::{BufRead, Write};

mod server_http;

#[derive(Serialize)]
struct CliOutput {
    html_file: String,
    viewport_w: f64,
    element_count: usize,
    json_bytes: usize,
    est_tokens: usize,
    report: inspect::InspectReport,
}

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if let Some(i) = args.iter().position(|a| a == "--serve-http") {
        let addr = args
            .get(i + 1)
            .filter(|a| a.contains(':'))
            .cloned()
            .unwrap_or_else(|| "127.0.0.1:8080".to_string());
        if let Err(e) = server_http::serve(&addr) {
            eprintln!("error: {}", e);
            std::process::exit(1);
        }
        return;
    }

    if args.iter().any(|a| a == "--serve") {
        serve_stdio();
        return;
    }

    if args.len() < 2 {
        eprintln!("usage: cornea <file.html> [viewport_width] | --serve | --serve-http [addr]");
        std::process::exit(2);
    }

    let path = &args[1];
    let viewport_w = args
        .get(2)
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(layout::DEFAULT_WIDTH);

    let html = match std::fs::read_to_string(path) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("error reading {}: {}", path, e);
            std::process::exit(1);
        }
    };

    let (_model, report) = analyze(&html, viewport_w);

    // token estimate measured on the report only (the part an agent consumes)
    let report_json = serde_json::to_string(&report).unwrap();
    let json_bytes = report_json.len();
    let est_tokens = rest::est_tokens(json_bytes);

    let output = CliOutput {
        html_file: path.clone(),
        viewport_w,
        element_count: report.total_elements,
        json_bytes,
        est_tokens,
        report,
    };

    println!("{}", serde_json::to_string_pretty(&output).unwrap());
}

// ---------------------------------------------------------------------------
// Minimal dependency-free MCP server over stdio (newline-delimited JSON-RPC).
// Tools: layout.inspect / layout.overlaps / layout.overflow / layout.contrast /
//        layout.quality / layout.fidelity
// The page source is supplied per-call via the `html` and optional `width`
// arguments (an agent reads its built file and passes it in).
// ---------------------------------------------------------------------------

fn serve_stdio() {
    let stdin = std::io::stdin();
    let mut server = McpServer;
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let resp = server.handle(line);
        if let Some(r) = resp {
            let mut out = std::io::stdout().lock();
            let _ = writeln!(out, "{}", r);
            let _ = out.flush();
        }
    }
}

struct McpServer;

impl McpServer {
    fn handle(&mut self, line: &str) -> Option<String> {
        let req: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => return None,
        };
        let id = req.get("id").cloned();
        let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("");

        let result = match method {
            "initialize" => {
                json!({
                    "protocolVersion": "2025-03-26",
                    "capabilities": { "tools": {} },
                    "serverInfo": { "name": "cornea", "version": env!("CARGO_PKG_VERSION") }
                })
            }
            "notifications/initialized" => return None,
            "tools/list" => self.tools_list(),
            "tools/call" => {
                let params = req.get("params").cloned().unwrap_or(json!({}));
                self.tools_call(&params)
            }
            _ => {
                return Some(error_json(
                    id,
                    -32601,
                    format!("method not found: {}", method),
                ));
            }
        };

        let mut resp = json!({ "jsonrpc": "2.0" });
        if let Some(id) = id {
            resp["id"] = id;
        }
        resp["result"] = result;
        Some(resp.to_string())
    }

    fn tools_list(&self) -> serde_json::Value {
        let tools = [
            tool_def(
                "layout.inspect",
                "Full deterministic visual model + quality summary of a page.",
            ),
            tool_def(
                "layout.overlaps",
                "Elements whose boxes collide in viewport space.",
            ),
            tool_def(
                "layout.overflow",
                "Clipped / collapsed / off-screen elements.",
            ),
            tool_def(
                "layout.contrast",
                "WCAG AA contrast ratios for text elements.",
            ),
            tool_def(
                "layout.quality",
                "Combined 0..1 health score with issue list.",
            ),
            tool_def(
                "layout.fidelity",
                "Which CSS features are exact vs approximated in the engine.",
            ),
        ];
        json!({ "tools": tools })
    }

    fn tools_call(&self, params: &serde_json::Value) -> serde_json::Value {
        let name = params.get("name").and_then(|n| n.as_str()).unwrap_or("");
        let args = params.get("arguments").cloned().unwrap_or(json!({}));

        if name == "layout.fidelity" {
            let (_status, payload) = rest::dispatch("fidelity", "");
            return json!({ "content": [ text_content(serde_json::to_string_pretty(&serde_json::from_str::<serde_json::Value>(&payload).unwrap()).unwrap()) ] });
        }

        let html = args.get("html").and_then(|h| h.as_str()).unwrap_or("");
        if html.is_empty() {
            return json!({ "content": [ text_content("error: missing 'html' argument (the page source)") ], "isError": true });
        }
        let width = args
            .get("width")
            .and_then(|w| w.as_f64())
            .unwrap_or(layout::DEFAULT_WIDTH);

        // route through the same canonical dispatch as HTTP/CLI for identical output
        let endpoint = match name {
            "layout.inspect" => "inspect",
            "layout.overlaps" => "overlaps",
            "layout.overflow" => "overflow",
            "layout.contrast" => "contrast",
            "layout.quality" => "quality",
            _ => "",
        };
        let req = serde_json::json!({ "html": html, "width": width }).to_string();
        let (status, payload) = rest::dispatch(endpoint, &req);
        if endpoint.is_empty() || status >= 400 {
            return json!({ "content": [ text_content(format!("error: {}", payload)) ], "isError": true });
        }
        json!({ "content": [ text_content(payload) ] })
    }
}

fn tool_def(name: &str, description: &str) -> serde_json::Value {
    let input_schema = json!({
        "type": "object",
        "properties": {
            "html": { "type": "string", "description": "The HTML source of the page to inspect" },
            "width": { "type": "number", "description": "Viewport width in px (default 360)" }
        },
        "required": ["html"]
    });
    json!({
        "name": name,
        "description": description,
        "inputSchema": input_schema
    })
}

fn text_content(text: impl Into<String>) -> serde_json::Value {
    json!({ "type": "text", "text": text.into() })
}

fn error_json(id: Option<serde_json::Value>, code: i64, message: String) -> String {
    let mut resp = json!({ "jsonrpc": "2.0", "error": { "code": code, "message": message } });
    if let Some(id) = id {
        resp["id"] = id;
    }
    resp.to_string()
}
