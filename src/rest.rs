//! Canonical REST-ish dispatch shared by every transport.
//!
//! All surfaces (CLI, MCP, HTTP API, tests) funnel inspection through the
//! single `dispatch` function so behavior is identical everywhere and is
//! directly testable end-to-end.

use crate::{analyze_vh, fetch, layout};
use serde::Deserialize;
use serde_json::json;

/// Valid endpoint names, used by MCP tool names, HTTP paths, and CLI.
pub const ENDPOINTS: &[&str] = &[
    "inspect", "overlaps", "overflow", "contrast", "quality", "fidelity",
];

#[derive(Debug, Deserialize)]
pub struct Request {
    #[serde(default)]
    pub html: String,
    /// Alternative to `html`: fetch this live URL and inline its stylesheets
    /// and scripts before inspecting (opt-in snapshot, inherently non
    /// deterministic across time).
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub width: Option<f64>,
    #[serde(default)]
    pub height: Option<f64>,
    #[serde(default)]
    pub full: bool,
    #[serde(default)]
    pub js: bool,
}

/// Handle one request against an endpoint. Returns `(http_status, json_payload)`.
/// Errors are returned as `{"error": "..."}` with 4xx/5xx status.
pub fn dispatch(endpoint: &str, body: &str) -> (u16, String) {
    match endpoint {
        "fidelity" => (200, fidelity_json()),
        "health" => (200, json!({ "status": "ok" }).to_string()),
        _ => {
            let req: Request = match serde_json::from_str(body) {
                Ok(r) => r,
                Err(e) => return (400, error_json(&format!("bad json: {}", e))),
            };
            let width = req.width.unwrap_or(layout::DEFAULT_WIDTH);
            let height = req.height.unwrap_or(0.0);
            // url replaces html when html is absent: fetch + inline first
            if req.html.trim().is_empty() {
                match req.url.as_deref() {
                    Some(u) if !u.trim().is_empty() => match fetch::fetch_and_inline(u) {
                        Ok(page) => run(endpoint, &page.html, width, height, req.js, &page.notes),
                        Err(e) => (502, error_json(&format!("cannot fetch {}: {}", u, e))),
                    },
                    _ => (400, error_json("missing 'html' (the page source) or 'url'")),
                }
            } else {
                run(endpoint, &req.html, width, height, req.js, &[])
            }
        }
    }
}

/// Analyze and serialize a single endpoint; shared by MCP/HTTP/CLI/tests.
/// When `js` is true, inline scripts are executed first (boA + DOM shim).
/// `height` of 0 means an unbounded scrolling page; a positive height enables
/// below-the-fold clipping detection. `extra_warnings` (e.g. capture notes
/// from a live fetch) ride along in the report's warnings list.
pub fn run(
    endpoint: &str,
    html: &str,
    width: f64,
    height: f64,
    js: bool,
    extra_warnings: &[String],
) -> (u16, String) {
    if html.trim().is_empty() {
        return (400, error_json("missing 'html' (the page source)"));
    }
    let (calc_html, js_notes) = if js {
        crate::js::execute_checked(html)
    } else {
        (html.to_string(), Vec::new())
    };
    let (_model, mut report) = analyze_vh(&calc_html, width, height);
    report.js_notes = js_notes;
    report.warnings.extend(extra_warnings.iter().cloned());
    let value = match endpoint {
        "inspect" => serde_json::to_value(&report).unwrap_or(json!({})),
        "overlaps" => serde_json::to_value(&report.overlaps).unwrap_or(json!([])),
        "overflow" => serde_json::to_value(&report.overflows).unwrap_or(json!([])),
        "contrast" => serde_json::to_value(&report.contrast).unwrap_or(json!([])),
        "quality" => {
            let mut q = serde_json::to_value(&report.quality).unwrap_or(json!({}));
            if let Some(obj) = q.as_object_mut() {
                obj.insert("js_notes".into(), json!(report.js_notes));
            }
            q
        }
        _ => return (404, error_json(&format!("unknown endpoint: {}", endpoint))),
    };
    (200, value.to_string())
}

/// Human- and agent-readable account of engine fidelity.
pub fn fidelity() -> serde_json::Value {
    json!({
        "exact": ["box model", "block flow", "inline text estimates", "flex row/column (no wrap)", "z-index", "visibility", "absolute/fixed left/top", "inline styles", "class/id/tag selectors", "WCAG contrast (hex, rgb, hsl, alpha)"],
        "approximate": ["text glyph width (not shaping)", "flex-grow/flex-basis distribution", "percentage widths", "overlap semantics ignore intentional stacking (a modal over content is still reported)"],
        "deferred": ["grid (parsed as block flow)", "media queries", "border-radius", "external stylesheet <link> when not captured", "complex selectors (combinators, pseudo)"],
        "js": {
            "engine": "boa",
            "phase": "A",
            "enabled": "opt-in via --js / js:true",
            "dom_shim": "static HTML mirrored in first; scripts may attach via getElementById; innerHTML parses real markup; className/classList/id supported",
            "unsupported": ["async APIs (setTimeout, fetch, XHR)", "event dispatch / addEventListener handlers", "CSS selector engine (querySelector)", "React/SPA mounting (Phase B)"],
            "limits": "only inline scripts run unless a capture layer inlined <script src> bodies"
        }
    })
}

fn fidelity_json() -> String {
    fidelity().to_string()
}

fn error_json(msg: &str) -> String {
    json!({ "error": msg }).to_string()
}

/// Convenience: token estimate for a JSON payload (~0.27 tokens/byte).
pub fn est_tokens(json_bytes: usize) -> usize {
    ((json_bytes as f64) * 0.27).round() as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    const OVERLAP_HTML: &str = r#"
    <html><body>
      <div style="position:absolute;left:0;top:0;width:100;height:100">A</div>
      <div style="position:absolute;left:50;top:50;width:100;height:100">B</div>
    </body></html>"#;

    fn body(endpoint: &str) -> String {
        let (status, payload) = dispatch(
            endpoint,
            &format!(
                r#"{{"html":{}}}"#,
                serde_json::to_string(OVERLAP_HTML).unwrap()
            ),
        );
        assert_eq!(status, 200, "{} should return 200", endpoint);
        payload
    }

    #[test]
    fn dispatch_returns_valid_json_for_every_endpoint() {
        for e in ["inspect", "overlaps", "overflow", "contrast", "quality"] {
            let payload = body(e);
            assert!(
                serde_json::from_str::<serde_json::Value>(&payload).is_ok(),
                "endpoint '{}' must return valid JSON",
                e
            );
        }
    }

    #[test]
    fn overlaps_endpoint_detects_collision() {
        let payload = body("overlaps");
        let v: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert!(
            !v.as_array().unwrap().is_empty(),
            "expected the two absolute divs to collide"
        );
    }

    #[test]
    fn inspect_endpoint_is_byte_deterministic() {
        let a = body("inspect");
        let b = body("inspect");
        assert_eq!(a, b, "shared dispatch must be deterministic");
    }

    #[test]
    fn missing_html_is_a_400_error() {
        let (status, payload) = dispatch("inspect", "{}");
        assert_eq!(status, 400);
        let v: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert!(v.get("error").is_some());
    }

    #[test]
    fn malformed_json_is_a_400_error() {
        let (status, payload) = dispatch("inspect", "{not json");
        assert_eq!(status, 400);
        let v: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert!(v.get("error").is_some());
    }

    #[test]
    fn fidelity_and_health_do_not_need_a_body() {
        let (s1, p1) = dispatch("fidelity", "");
        assert_eq!(s1, 200);
        assert!(p1.contains("exact"));
        let (s2, p2) = dispatch("health", "");
        assert_eq!(s2, 200);
        assert!(p2.contains("ok"));
    }

    #[test]
    fn token_estimate_scales_with_size() {
        assert_eq!(est_tokens(0), 0);
        assert!(est_tokens(1000) > est_tokens(100));
    }

    #[test]
    fn height_enables_below_fold_clipping_detection() {
        let html = "<html><body><div style=\"width:50;height:500\">tall</div></body></html>";
        let (s0, p0) = dispatch(
            "overflow",
            &format!(
                r#"{{"html":{},"width":360}}"#,
                serde_json::to_string(html).unwrap()
            ),
        );
        assert_eq!(s0, 200);
        let no_height: serde_json::Value = serde_json::from_str(&p0).unwrap();
        assert!(
            no_height.as_array().unwrap().is_empty(),
            "without a height the page is unbounded, nothing clips"
        );
        let (s1, p1) = dispatch(
            "overflow",
            &format!(
                r#"{{"html":{},"width":360,"height":100}}"#,
                serde_json::to_string(html).unwrap()
            ),
        );
        assert_eq!(s1, 200);
        let with_height: serde_json::Value = serde_json::from_str(&p1).unwrap();
        assert!(
            with_height
                .as_array()
                .unwrap()
                .iter()
                .any(|o| o["kind"] == "clipped"
                    && o["detail"].as_str().unwrap_or("").contains("500")),
            "a 500px tall element inside a 100px viewport must be clipped: {}",
            p1
        );
    }
}
