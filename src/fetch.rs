//! Live capture for the inspection engine.
//!
//! `fetch_and_inline` performs a synchronous HTTP GET against a page URL,
//! then rewrites the returned HTML so the deterministic engine can consume a
//! real live document:
//!
//!   - `<link rel="stylesheet" ...>` tags are fetched and replaced with inline
//!     `<style>` blocks (relative URLs resolved against the page URL),
//!   - `<script src="...">` tags are fetched and their bodies inlined,
//!   - every failed or skipped asset becomes a note, so the caller can surface
//!     honesty next to the report instead of silently inspecting a shell.
//!
//! Determinism note: fetching a URL is inherently a snapshot in time, so
//! `--fetch`/`url:` is opt-in. The promise that holds is the engine's: given
//! the same fetched bytes, the same report comes out byte-identical. All
//! reads are hard capped and timeouts are set, so a hostile server cannot
//! make the client allocate unbounded memory or hang forever.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::time::Duration;

/// Upper bound for the main document body.
const MAX_PAGE: usize = 8 * 1024 * 1024;
/// Upper bound for one inlined asset (stylesheet or script).
const MAX_ASSET: usize = 4 * 1024 * 1024;
/// Max assets inlined per page (stylesheet + script combined).
const MAX_ASSETS: usize = 24;

/// Either a plain TCP stream or a TLS session over one. Lets every reader
/// helper below stay transport agnostic.
enum Transport {
    Plain(TcpStream),
    Tls(rustls::StreamOwned<rustls::ClientConnection, TcpStream>),
}

impl Read for Transport {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Transport::Plain(s) => s.read(buf),
            Transport::Tls(s) => s.read(buf),
        }
    }
}

impl Write for Transport {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Transport::Plain(s) => s.write(buf),
            Transport::Tls(s) => s.write(buf),
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Transport::Plain(s) => s.flush(),
            Transport::Tls(s) => s.flush(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct FetchedPage {
    pub html: String,
    pub notes: Vec<String>,
}

/// A parsed absolute URL: scheme, host, port, path (query stripped).
struct Target {
    scheme: String,
    host: String,
    port: u16,
    path: String,
}

/// Fetch `url`, inline its stylesheets and scripts, return the result.
/// Any asset that fails is reported in `notes` and left in place, so the
/// report warnings layer still sees it.
pub fn fetch_and_inline(url: &str) -> Result<FetchedPage, String> {
    let target = parse_target(url)?;
    let html = http_get(url, MAX_PAGE)?;
    let mut page = FetchedPage {
        html,
        notes: Vec::new(),
    };
    inline_assets(&target, &mut page);
    Ok(page)
}

// ---------------------------------------------------------------------------
// HTTP client
// ---------------------------------------------------------------------------

fn parse_target(url: &str) -> Result<Target, String> {
    let https = url.starts_with("https://");
    let rest = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))
        .ok_or_else(|| format!("unsupported URL scheme (only http/https): {}", url))?;
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) if p.parse::<u16>().is_ok() => (h, p.parse::<u16>().unwrap()),
        _ => (authority, if https { 443 } else { 80 }),
    };
    if host.is_empty() {
        return Err(format!("URL has no host: {}", url));
    }
    // drop query/fragment from the path
    let path = match path.find(['?', '#']) {
        Some(i) => &path[..i],
        None => path,
    };
    Ok(Target {
        scheme: if https { "https" } else { "http" }.into(),
        host: host.to_string(),
        port,
        path: path.to_string(),
    })
}

/// Default TLS client configuration trusting the public webpki root store.
/// Built lazily once; rustls needs the ring provider installed process wide.
pub fn default_tls_config() -> Result<Arc<rustls::ClientConfig>, String> {
    static INSTALL: std::sync::Once = std::sync::Once::new();
    INSTALL.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    Ok(Arc::new(
        rustls::ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth(),
    ))
}

/// Connect to the target and return a transport, plain or TLS with SNI.
fn connect_transport(target: &Target) -> Result<Transport, String> {
    let addr = format!("{}:{}", target.host, target.port);
    let tcp = TcpStream::connect(&addr).map_err(|e| format!("connect {}: {}", addr, e))?;
    tcp.set_read_timeout(Some(Duration::from_secs(15)))
        .map_err(|e| format!("timeout setup: {}", e))?;
    if target.scheme == "http" {
        return Ok(Transport::Plain(tcp));
    }
    let config = default_tls_config()?;
    let name = rustls::pki_types::ServerName::try_from(target.host.clone())
        .map_err(|_| format!("invalid TLS hostname: {}", target.host))?;
    let conn =
        rustls::ClientConnection::new(config, name).map_err(|e| format!("tls setup: {}", e))?;
    Ok(Transport::Tls(rustls::StreamOwned::new(conn, tcp)))
}

fn http_get(url: &str, cap: usize) -> Result<String, String> {
    let target = parse_target(url)?;
    let mut transport = connect_transport(&target)?;
    let req = format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: cornea\r\nAccept: */*\r\nConnection: close\r\n\r\n",
        target.path, target.host
    );
    transport
        .write_all(req.as_bytes())
        .map_err(|e| format!("write request: {}", e))?;
    let mut reader = BufReader::new(transport);
    let status_line =
        read_line_capped(&mut reader, 8 * 1024)?.ok_or_else(|| "empty response".to_string())?;
    let status_line = String::from_utf8_lossy(&status_line);
    let mut parts = status_line.split_whitespace();
    let _version = parts.next();
    let status: u16 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);

    let mut content_length: Option<usize> = None;
    let mut chunked = false;
    while let Some(line) = read_line_capped(&mut reader, 8 * 1024)? {
        let line = String::from_utf8_lossy(&line);
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            if k.eq_ignore_ascii_case("content-length") {
                content_length = v.trim().parse::<usize>().ok();
            } else if k.eq_ignore_ascii_case("transfer-encoding") {
                chunked = v.to_ascii_lowercase().contains("chunked");
            }
        }
    }

    if status != 200 {
        return Err(format!("HTTP status {}", status));
    }

    let body = if chunked {
        read_chunked(&mut reader, cap)?
    } else {
        match content_length {
            Some(n) if n > cap => return Err(format!("response body {} bytes exceeds cap", n)),
            Some(n) => {
                let mut buf = Vec::with_capacity(n);
                reader
                    .take(n as u64 + 1)
                    .read_to_end(&mut buf)
                    .map_err(|e| format!("read body: {}", e))?;
                buf.truncate(n);
                buf
            }
            None => {
                let mut buf = Vec::new();
                reader
                    .take(cap as u64 + 1)
                    .read_to_end(&mut buf)
                    .map_err(|e| format!("read body: {}", e))?;
                if buf.len() > cap {
                    return Err("response body exceeds cap".into());
                }
                buf
            }
        }
    };
    let body = String::from_utf8_lossy(&body).to_string();
    Ok(body)
}

/// Decode an HTTP/1.1 chunked body (hex size lines, CRLF separated data).
fn read_chunked<R: BufRead>(r: &mut R, cap: usize) -> Result<Vec<u8>, String> {
    let mut out: Vec<u8> = Vec::new();
    loop {
        let Some(line) = read_line_capped(r, 8 * 1024)? else {
            return Err("truncated chunked body".into());
        };
        let line = String::from_utf8_lossy(&line);
        let size_str = line.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size_str, 16)
            .map_err(|_| format!("bad chunk size: {:?}", size_str))?;
        if size == 0 {
            // trailers until blank line, then done
            while let Some(t) = read_line_capped(r, 8 * 1024)? {
                if t.is_empty() {
                    break;
                }
            }
            return Ok(out);
        }
        if out.len() + size > cap {
            return Err("chunked body exceeds cap".into());
        }
        let mut chunk = vec![0u8; size];
        r.read_exact(&mut chunk)
            .map_err(|e| format!("read chunk: {}", e))?;
        out.extend_from_slice(&chunk);
        // trailing CRLF after chunk data; tolerate a missing terminator only
        // when the stream simply ends there
        let mut crlf = [0u8; 2];
        match r.read_exact(&mut crlf) {
            Ok(_) => {
                if crlf != *b"\r\n" {
                    return Err("malformed chunk terminator".into());
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                return Ok(out);
            }
            Err(e) => return Err(format!("read chunk terminator: {}", e)),
        }
    }
}

fn read_line_capped<R: BufRead>(r: &mut R, cap: usize) -> Result<Option<Vec<u8>>, String> {
    let mut out: Vec<u8> = Vec::with_capacity(128);
    loop {
        if out.len() > cap {
            return Err("line exceeds cap".into());
        }
        let mut byte = [0u8; 1];
        let n = r.read(&mut byte).map_err(|e| format!("read line: {}", e))?;
        if n == 0 {
            return Ok(if out.is_empty() { None } else { Some(out) });
        }
        if byte[0] == b'\n' {
            return Ok(Some(out));
        }
        out.push(byte[0]);
    }
}

// ---------------------------------------------------------------------------
// Asset inlining
// ---------------------------------------------------------------------------

/// Fetch and inline stylesheet `<link>`s and `<script src>`s in place.
fn inline_assets(target: &Target, page: &mut FetchedPage) {
    let mut cache: HashMap<String, Result<String, String>> = HashMap::new();
    let mut inlined = 0usize;
    let lower = page.html.to_ascii_lowercase();
    let mut out = String::with_capacity(page.html.len());
    let mut lower_rest = lower.as_str();
    let mut cursor = 0usize;

    while inlined < MAX_ASSETS {
        // find the next <link or <script in the lowercase view
        let (tag, start) = find_next_tag(lower_rest);
        if start == usize::MAX {
            break;
        }
        let abs = cursor + start;
        // copy everything up to this tag
        out.push_str(&page.html[cursor..abs]);
        // locate the end of the open tag in the original string
        let open_end = page.html[abs..]
            .find('>')
            .map(|i| abs + i)
            .unwrap_or(page.html.len());
        let open_tag = &page.html[abs..=open_end];
        let open_lower = open_tag.to_ascii_lowercase();

        let is_link_stylesheet =
            tag == "link" && (open_lower.contains("stylesheet") || open_lower.contains("text/css"));
        let is_external_script = tag == "script"
            && extract_attr(&open_lower, "src")
                .map(|s| !s.is_empty())
                .unwrap_or(false);

        if is_link_stylesheet || is_external_script {
            if let Some(href) =
                extract_attr(&open_lower, if is_link_stylesheet { "href" } else { "src" })
            {
                let url = resolve(target, &href);
                let got = match cache.get(&url) {
                    Some(r) => r.clone(),
                    None => {
                        let r = http_get(&url, MAX_ASSET);
                        cache.insert(url.clone(), r.clone());
                        r
                    }
                };
                match got {
                    Ok(css_or_js) => {
                        let wrapped = if is_link_stylesheet {
                            format!("<style>\n{}\n</style>", css_or_js)
                        } else {
                            format!("<script>\n{}\n</script>", css_or_js)
                        };
                        out.push_str(&wrapped);
                        inlined += 1;
                    }
                    Err(e) => {
                        page.notes.push(format!("could not fetch {}: {}", url, e));
                        // keep the original tag: the warnings layer flags it
                        out.push_str(open_tag);
                    }
                }
                // skip past the open tag; find matching close if script/link pair
                let after_open = &page.html[open_end + 1..];
                if is_external_script
                    && let Some(close) = after_open.to_ascii_lowercase().find("</script")
                {
                    // skip past the close tag, dropping the original body
                    cursor = open_end + 1 + close + "</script".len() + 1;
                    lower_rest = &lower[cursor.min(lower.len())..];
                    continue;
                }
                cursor = open_end + 1;
            } else {
                out.push_str(open_tag);
                cursor = open_end + 1;
            }
        } else {
            out.push_str(open_tag);
            cursor = open_end + 1;
        }
        lower_rest = &lower[cursor.min(lower.len())..];
    }
    if inlined >= MAX_ASSETS {
        page.notes
            .push(format!("stopped inlining after {} assets", MAX_ASSETS));
    }
    if cursor < page.html.len() {
        out.push_str(&page.html[cursor..]);
    }
    page.html = out;
    if inlined > 0 {
        page.notes.push(format!(
            "inlined {} external asset(s) from {}",
            inlined, target.host
        ));
    }
}

/// Find the next `<link` or `<script` in a lowercase html slice.
/// Returns (tag, relative index) or (tag, usize::MAX) when none.
fn find_next_tag(lower: &str) -> (String, usize) {
    let l = lower.find("<link").map(|i| ("link", i));
    let s = lower.find("<script").map(|i| ("script", i));
    match (l, s) {
        (Some((lt, li)), Some((st, si))) => {
            if li <= si {
                (lt.to_string(), li)
            } else {
                (st.to_string(), si)
            }
        }
        (Some((lt, li)), None) => (lt.to_string(), li),
        (None, Some((st, si))) => (st.to_string(), si),
        (None, None) => ("".into(), usize::MAX),
    }
}

/// Extract an attribute value from a lowercased open tag.
fn extract_attr(open_lower: &str, name: &str) -> Option<String> {
    let name = name.to_ascii_lowercase();
    let mut search = open_lower;
    while let Some(idx) = search.find(&name) {
        let before = &search[..idx];
        // word boundary: previous char must not be an ident char
        if let Some(prev) = before.chars().last()
            && (prev.is_ascii_alphanumeric() || prev == '-' || prev == '_')
        {
            search = &search[idx + name.len()..];
            continue;
        }
        let after = &search[idx + name.len()..];
        let after = after.trim_start();
        if let Some(v) = after.strip_prefix('=') {
            let v = v.trim_start();
            let (val, _) = if let Some(q) = v.strip_prefix('"') {
                let end = q.find('"')?;
                (&q[..end], end + 1)
            } else if let Some(q) = v.strip_prefix('\'') {
                let end = q.find('\'')?;
                (&q[..end], end + 1)
            } else {
                let end = v
                    .find(|c: char| c.is_whitespace() || c == '>')
                    .unwrap_or(v.len());
                (&v[..end], end)
            };
            return Some(val.to_string());
        }
        return None;
    }
    None
}

/// Resolve a (possibly relative) asset href against the page target.
fn resolve(target: &Target, href: &str) -> String {
    let h = href.trim();
    let base = format!(
        "{}://{}{}",
        target.scheme,
        target.host,
        if target.port != 80 {
            format!(":{}", target.port)
        } else {
            String::new()
        }
    );
    if h.starts_with("http://") || h.starts_with("https://") {
        return h.to_string();
    }
    if let Some(rest) = h.strip_prefix("//") {
        return format!("{}://{}", target.scheme, rest);
    }
    if let Some(p) = h.strip_prefix('/') {
        return format!("{}/{}", base, p);
    }
    // relative: directory of the page path, then normalize ../ segments
    let dir = match target.path.rfind('/') {
        Some(i) => &target.path[..=i],
        None => "/",
    };
    let combined = format!("{}{}", dir, h);
    let mut segs: Vec<String> = Vec::new();
    for seg in combined.split('/') {
        match seg {
            ".." => {
                segs.pop();
            }
            "." | "" => {}
            s => segs.push(s.to_string()),
        }
    }
    format!("{}/{}", base, segs.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_relative_asset_urls() {
        let t = parse_target("http://localhost:3000/app/page.html").unwrap();
        assert_eq!(resolve(&t, "/site.css"), "http://localhost:3000/site.css");
        assert_eq!(
            resolve(&t, "style.css"),
            "http://localhost:3000/app/style.css"
        );
        assert_eq!(resolve(&t, "../up.css"), "http://localhost:3000/up.css");
        assert_eq!(
            resolve(&t, "https://cdn.example.com/x.css"),
            "https://cdn.example.com/x.css"
        );
        assert_eq!(
            resolve(&t, "//cdn.example.com/x.css"),
            "http://cdn.example.com/x.css"
        );
    }

    #[test]
    fn extracts_attributes_from_open_tags() {
        assert_eq!(
            extract_attr(r#"<link rel="stylesheet" href='/a.css'>"#, "href"),
            Some("/a.css".into())
        );
        assert_eq!(
            extract_attr(r#"<script defer src=app.js></script>"#, "src"),
            Some("app.js".into())
        );
        assert_eq!(extract_attr(r#"<script src></script>"#, "src"), None);
        // href inside a different attr name must not match
        assert_eq!(extract_attr(r#"<div myhref="/nope"></div>"#, "href"), None);
    }

    #[test]
    fn chunked_reader_decodes_hex_frames() {
        let raw = b"4\r\nWiki\r\n5\r\npedia\r\nE\r\n in\r\n\r\nchunks.\r\n0\r\n\r\n";
        let mut r = std::io::Cursor::new(raw.to_vec());
        let out = read_chunked(&mut r, 1024).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "Wikipedia in\r\n\r\nchunks."
        );
    }

    #[test]
    fn parse_target_handles_ports_and_paths() {
        let t = parse_target("http://localhost:8080/").unwrap();
        assert_eq!(
            (t.host.as_str(), t.port, t.path.as_str()),
            ("localhost", 8080, "/")
        );
        let t2 = parse_target("http://example.com").unwrap();
        assert_eq!(
            (t2.host.as_str(), t2.port, t2.path.as_str()),
            ("example.com", 80, "/")
        );
        assert!(parse_target("ftp://x/y").is_err());
    }
}
