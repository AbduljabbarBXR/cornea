//! Headless browser capture.
//!
//! When a desktop chrome like browser exists on the machine, cornea can ask
//! it to render a URL for real (JavaScript, React, Tailwind, media queries)
//! and dump the resulting DOM. The DOM is then inlined exactly like a fetched
//! page and fed to the deterministic engine unchanged. This is the phase two
//! fidelity bridge: engine stays the source of truth, browser is an optional
//! capture backend.
//!
//! Honest limits: chrome like families only (chrome, chromium, edge, brave).
//! Firefox headless has no DOM dump mode, and on Android no such binary
//! exists at all, so capture there reports a clean error and callers fall
//! back to plain http capture.

use crate::fetch;
use std::io::Read;
use std::time::{Duration, Instant};

/// Virtual time budget for page scripts (chromium flag, milliseconds).
const VIRTUAL_BUDGET_MS: &str = "4000";
/// Hard wall clock cap for one capture attempt.
const CAPTURE_TIMEOUT: Duration = Duration::from_secs(25);

/// (name, path) of the preferred headless candidate, if any.
pub fn headless_browser() -> Option<(String, String)> {
    crate::probe::detect()
        .into_iter()
        .find(|b| {
            b.kind == "chrome_like"
                && b.headless_capable
                && (b.name.contains("chrome")
                    || b.name.contains("chromium")
                    || b.name.contains("edge")
                    || b.name.contains("brave"))
        })
        .map(|b| (b.name, b.path))
}

/// Render `url` headless and return the page with its assets inlined, plus
/// the browser name used.
pub fn capture_url(url: &str) -> Result<(fetch::FetchedPage, String), String> {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("capture needs an http(s) URL".into());
    }
    let (name, bin) = headless_browser().ok_or_else(|| {
        "no headless chrome like browser detected on this machine (try --browsers)".to_string()
    })?;

    // modern headless first, classic flag as fallback for older binaries
    let modern = run_dump(&bin, url, vec!["--headless=new"]);
    let dom = match modern {
        Ok(d) if looks_like_dom(&d) => d,
        _ => {
            let classic = run_dump(&bin, url, vec!["--headless"])?;
            if !looks_like_dom(&classic) {
                return Err("headless browser produced no DOM output".into());
            }
            classic
        }
    };

    let mut page = fetch::inline_dom(url, dom)?;
    page.notes
        .insert(0, format!("captured with {} headless", name));
    Ok((page, name))
}

fn run_dump(bin: &str, url: &str, headless: Vec<&str>) -> Result<String, String> {
    let mut cmd = std::process::Command::new(bin);
    cmd.args(headless)
        .args([
            "--disable-gpu",
            "--no-sandbox",
            "--disable-dev-shm-usage",
            "--disable-software-rasterizer",
            "--virtual-time-budget",
            VIRTUAL_BUDGET_MS,
            "--dump-dom",
        ])
        .arg(url)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let mut child = cmd.spawn().map_err(|e| format!("spawn {}: {}", bin, e))?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "no stdout pipe".to_string())?;
    let reader = std::thread::spawn(move || {
        let mut buf = String::new();
        let mut locked = stdout;
        let _ = locked.read_to_string(&mut buf);
        buf
    });

    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_status)) => break,
            Ok(None) => {}
            Err(e) => return Err(format!("wait {}: {}", bin, e)),
        }
        if started.elapsed() > CAPTURE_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("headless capture timed out after 25s ({})", bin));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let dom = reader
        .join()
        .map_err(|_| "stdout thread panicked".to_string())?;
    Ok(dom)
}

fn looks_like_dom(d: &str) -> bool {
    let lower = d.to_ascii_lowercase();
    lower.contains("<html") || lower.contains("<!doctype html")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use std::process::Command;

    /// A fake "chrome" whose --dump-dom prints a fixed rendered DOM.
    fn fake_browser() -> String {
        let path = std::env::temp_dir().join("cornea_fake_chrome.sh");
        std::fs::write(
            &path,
            "#!/bin/sh\necho '<!doctype html><html><body><section class=\"hero\"><h1>Rendered</h1></section></body></html>'\n",
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn run_dump_captures_stdout_from_a_browser_binary() {
        let bin = fake_browser();
        let dom = run_dump(&bin, "http://example.test/", vec!["--headless=new"]).unwrap();
        assert!(dom.contains("Rendered"), "dom: {}", dom);
        let _ = std::fs::remove_file(&bin);
    }

    #[test]
    fn looks_like_dom_accepts_doctypes() {
        assert!(looks_like_dom("<!doctype html><html>"));
        assert!(looks_like_dom("<HTML><BODY>x"));
        assert!(!looks_like_dom("error: chrome crashed"));
    }
}
