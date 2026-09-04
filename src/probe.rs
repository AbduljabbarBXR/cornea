//! Host browser discovery and launching.
//!
//! Cornea stays a deterministic engine; these helpers let an agent find out
//! what browser a machine actually has and ask for a URL to be opened in it.
//! Detection is a filesystem probe only, no process is started by `detect`.
//! `open_url` refuses any scheme other than http/https.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// One detected browser or default opener.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Browser {
    pub name: String,
    pub path: String,
    /// chrome_like, firefox, safari, or opener (xdg-open, termux-open-url).
    pub kind: String,
    /// Whether the binary family supports headless capture.
    pub headless_capable: bool,
}

const NAMES: &[&str] = &[
    "google-chrome",
    "google-chrome-stable",
    "chromium",
    "chromium-browser",
    "microsoft-edge",
    "msedge",
    "brave-browser",
    "firefox",
    "safari",
    "chrome",
    "edge",
    "opera",
];

/// Detect browsers on this machine. Looks on PATH plus the conventional
/// per OS install roots, then reports default openers when present.
pub fn detect() -> Vec<Browser> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(path) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&path));
    }
    dirs.push(PathBuf::from("/usr/bin"));
    dirs.push(PathBuf::from("/usr/local/bin"));

    #[cfg(target_os = "macos")]
    dirs.extend([
        PathBuf::from("/Applications/Google Chrome.app/Contents/MacOS"),
        PathBuf::from("/Applications/Chromium.app/Contents/MacOS"),
        PathBuf::from("/Applications/Firefox.app/Contents/MacOS"),
        PathBuf::from("/Applications/Microsoft Edge.app/Contents/MacOS"),
        PathBuf::from("/Applications/Brave Browser.app/Contents/MacOS"),
        PathBuf::from("/Applications/Safari.app/Contents/MacOS"),
    ]);

    #[cfg(target_os = "windows")]
    {
        if let Ok(pf) = std::env::var("ProgramFiles") {
            dirs.push(PathBuf::from(pf).join("Google/Chrome/Application"));
            dirs.push(PathBuf::from(pf).join("Microsoft/Edge/Application"));
            dirs.push(PathBuf::from(pf).join("Mozilla Firefox"));
            dirs.push(PathBuf::from(pf).join("BraveSoftware/Brave-Browser/Application"));
        }
        if let Ok(pf) = std::env::var("ProgramFiles(x86)") {
            dirs.push(PathBuf::from(pf).join("Google/Chrome/Application"));
            dirs.push(PathBuf::from(pf).join("Microsoft/Edge/Application"));
            dirs.push(PathBuf::from(pf).join("Mozilla Firefox"));
        }
    }

    detect_from_dirs(&dirs)
}

/// Core probe used by `detect` and by tests with synthetic directories.
fn detect_from_dirs(dirs: &[PathBuf]) -> Vec<Browser> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut out: Vec<Browser> = Vec::new();
    for dir in dirs {
        for name in NAMES.iter().copied() {
            let candidate = dir.join(name);
            if seen.contains(name) || !is_executable(&candidate) {
                continue;
            }
            seen.insert(name.to_string());
            let (kind, headless) = classify(name);
            out.push(Browser {
                name: (*name).to_string(),
                path: candidate.to_string_lossy().into_owned(),
                kind: kind.to_string(),
                headless_capable: headless,
            });
        }
    }
    // default openers, reported last so they only win when nothing else did
    for (name, path_str) in [
        ("xdg-open", "/usr/bin/xdg-open"),
        ("gio-open", "/usr/bin/gio"),
        (
            "termux-open-url",
            "/data/data/com.termux/files/usr/bin/termux-open-url",
        ),
    ] {
        if seen.contains(name) {
            continue;
        }
        // gio only matters when xdg-open is missing
        if name == "gio-open" && seen.contains("xdg-open") {
            continue;
        }
        let path = PathBuf::from(path_str);
        if is_executable(&path) {
            seen.insert(name.to_string());
            out.push(Browser {
                name: name.to_string(),
                path: path.to_string_lossy().into_owned(),
                kind: "opener".into(),
                headless_capable: false,
            });
        }
    }
    out.sort_by(|a, b| {
        let rank = |k: &str| match k {
            "chrome_like" => 0,
            "firefox" => 1,
            "safari" => 2,
            "opener" => 3,
            _ => 4,
        };
        rank(&a.kind).cmp(&rank(&b.kind))
    });
    out
}

/// executable check: exists, is a file, and (unix) has any exec bit
fn is_executable(p: &Path) -> bool {
    if !p.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = match std::fs::metadata(p) {
            Ok(m) => m.permissions().mode(),
            Err(_) => return false,
        };
        mode & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn classify(name: &str) -> (&'static str, bool) {
    let n = name.to_ascii_lowercase();
    if n.contains("firefox") {
        ("firefox", true)
    } else if n.contains("safari") {
        ("safari", false)
    } else if n.contains("chrome")
        || n.contains("chromium")
        || n.contains("edge")
        || n.contains("brave")
        || n.contains("opera")
    {
        ("chrome_like", true)
    } else {
        ("other", false)
    }
}

/// Launch a browser for `url`. Pick order: explicit name, first headless
/// capable binary (chrome like or firefox), any desktop browser, then the
/// default opener (xdg-open or termux-open-url on Android).
pub fn open_url(url: &str, preferred: Option<&str>) -> Result<String, String> {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("only http/https URLs can be opened".into());
    }
    let found = detect();
    if found.is_empty() {
        return Err("no browser or opener detected on this machine".into());
    }
    let pick = |f: &dyn Fn(&Browser) -> bool| found.iter().find(|b| f(b));

    let chosen: Option<&Browser> = match preferred {
        Some(name) => found
            .iter()
            .find(|b| b.name == name || b.name.contains(name)),
        None => pick(&|b| b.headless_capable)
            .or_else(|| pick(&|b| b.kind != "opener"))
            .or_else(|| pick(&|b| b.kind == "opener")),
    };
    let browser = chosen.ok_or_else(|| "no usable browser found".to_string())?;

    let mut cmd = std::process::Command::new(&browser.path);
    match browser.kind.as_str() {
        "opener" => {
            if browser.name == "termux-open-url" {
                cmd.arg(url); // opens the Android default browser
            } else if browser.name == "gio-open" {
                cmd.arg("open").arg(url);
            } else {
                cmd.arg(url); // xdg-open
            }
        }
        _ => {
            cmd.arg(url);
        }
    }
    cmd.spawn()
        .map_err(|e| format!("launch {}: {}", browser.path, e))?;
    Ok(format!("opened {} with {}", url, browser.name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_browsers_in_synthetic_dir() {
        let dir = std::env::temp_dir().join("cornea_probe_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["google-chrome", "firefox", "safari"] {
            let p = dir.join(name);
            std::fs::write(&p, "#!/bin/sh\nexit 0\n").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
        }
        let found = detect_from_dirs(&[dir.clone()]);
        let names: Vec<&str> = found.iter().map(|b| b.name.as_str()).collect();
        assert!(
            names.contains(&"google-chrome"),
            "chrome detected: {:?}",
            names
        );
        assert!(names.contains(&"firefox"));
        assert!(
            !names.contains(&"safari")
                || found
                    .iter()
                    .any(|b| b.name == "safari" && !b.headless_capable)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn classifies_families() {
        assert_eq!(classify("google-chrome"), ("chrome_like", true));
        assert_eq!(classify("msedge"), ("chrome_like", true));
        assert_eq!(classify("firefox"), ("firefox", true));
        assert_eq!(classify("Safari"), ("safari", false));
    }

    #[test]
    fn open_url_rejects_non_http() {
        assert!(open_url("file:///etc/passwd", None).is_err());
        assert!(open_url("javascript:alert(1)", None).is_err());
    }
}
