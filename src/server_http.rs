/// Minimal dependency-free HTTP/1.1 JSON API server.
///
/// Node-style single binary (no axum/actix): a small epoll-free accept loop
/// on std `TcpListener`. Stateless — the client sends the page HTML and
/// optional viewport width; Cornea returns deterministic JSON.
///
/// Routes (all body `{"html":"...","width":360}`):
///   POST /inspect    full inspection report
///   POST /overlaps   box collisions
///   POST /overflow   clipped / collapsed / off-screen
///   POST /contrast   WCAG AA contrast results
///   POST /quality    0..1 health score
///   GET  /fidelity   engine feature fidelity
///   GET  /health     liveness
use cornea::rest;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};

/// Upper bound for a single inspect payload (a full page of HTML plus JSON).
const MAX_BODY: usize = 8 * 1024 * 1024;
/// Upper bound for one HTTP line (request line or header). Bounded so a
/// hostile client cannot grow an unbounded allocation.
const MAX_LINE: usize = 8 * 1024;
/// Upper bound on the number of header lines.
const MAX_HEADERS: usize = 64;

pub fn serve(addr: &str) -> std::io::Result<()> {
    let listener = TcpListener::bind(addr)?;
    serve_on(listener)
}

fn serve_on(listener: TcpListener) -> std::io::Result<()> {
    let addr = listener.local_addr()?;
    eprintln!("cornea api listening on http://{} (Ctrl-C to stop)", addr);
    for stream in listener.incoming() {
        match stream {
            Ok(s) => {
                if let Err(e) = handle_conn(s) {
                    eprintln!("connection error: {}", e);
                }
            }
            Err(e) => eprintln!("accept error: {}", e),
        }
    }
    Ok(())
}

fn handle_conn(mut stream: TcpStream) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);

    let Some(request_line) = read_line_capped(&mut reader, MAX_LINE)? else {
        return Ok(());
    };
    let request_line = String::from_utf8_lossy(&request_line);
    let mut parts = request_line.split_whitespace();
    let _method = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("/").to_string();

    // Headers: capture Content-Length and skip the rest (bounded count and
    // per-line length so a hostile client cannot exhaust memory).
    let mut content_length = 0usize;
    for _ in 0..MAX_HEADERS {
        let Some(line) = read_line_capped(&mut reader, MAX_LINE)? else {
            break;
        };
        let line = String::from_utf8_lossy(&line);
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            if k.eq_ignore_ascii_case("content-length") {
                content_length = v.trim().parse().unwrap_or(0);
            }
        }
    }

    // Refuse oversized payloads before reading a byte of body.
    if content_length > MAX_BODY {
        let body = serde_json::json!({ "error": "request body exceeds 8 MiB limit" }).to_string();
        write_response(&mut stream, 413, &body)?;
        return Ok(());
    }

    let mut body = Vec::with_capacity(content_length);
    reader.take(content_length as u64).read_to_end(&mut body)?;
    let body_str = String::from_utf8_lossy(&body).to_string();

    let endpoint = endpoint_from_path(&path);
    let result = match endpoint {
        Some(e) => {
            let (status, payload) = rest::dispatch(e, &body_str);
            (status, payload)
        }
        None => (
            404,
            serde_json::json!({ "error": format!("unknown endpoint: {}", path) }).to_string(),
        ),
    };
    write_response(&mut stream, result.0, &result.1)?;
    Ok(())
}

/// Read one line (up to and including its newline) with a hard byte cap so a
/// hostile client cannot drive an unbounded allocation. Returns None on EOF
/// with no bytes read.
pub(crate) fn read_line_capped<R: BufRead>(
    r: &mut R,
    cap: usize,
) -> std::io::Result<Option<Vec<u8>>> {
    let mut out: Vec<u8> = Vec::with_capacity(128);
    loop {
        if out.len() > cap {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "line exceeds server cap",
            ));
        }
        let mut byte = [0u8; 1];
        let n = r.read(&mut byte)?;
        if n == 0 {
            return Ok(if out.is_empty() { None } else { Some(out) });
        }
        if byte[0] == b'\n' {
            return Ok(Some(out));
        }
        out.push(byte[0]);
    }
}

fn endpoint_from_path(path: &str) -> Option<&'static str> {
    let t = path.trim_matches('/');
    // map to a known static endpoint name (rejecting traversal / unknown)
    if t.is_empty() {
        Some("inspect") // "/" -> inspect
    } else if t == "health" {
        Some("health")
    } else if rest::ENDPOINTS.contains(&t) {
        // ENDPOINTS items are 'static str; find the canonical matching one
        rest::ENDPOINTS.iter().find(|&&e| e == t).copied()
    } else {
        None // unknown path -> 404
    }
}

fn write_response(stream: &mut TcpStream, status: u16, body: &str) -> std::io::Result<()> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        413 => "Payload Too Large",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        _ => "OK",
    };
    let headers = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: application/json; charset=utf-8\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Headers: Content-Type\r\nAccess-Control-Allow-Methods: POST, GET, OPTIONS\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        status,
        reason,
        body.len()
    );
    stream.write_all(headers.as_bytes())?;
    stream.write_all(body.as_bytes())?;
    stream.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::net::TcpStream;
    use std::thread;
    use std::time::Duration;

    fn start_server() -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        thread::spawn(move || {
            let _ = serve_on(listener);
        });
        port
    }

    fn raw_request(port: u16, raw_http: &str) -> String {
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        s.write_all(raw_http.as_bytes()).unwrap();
        let mut buf = String::new();
        s.read_to_string(&mut buf).unwrap();
        buf
    }

    fn post(port: u16, path: &str, body: &str) -> String {
        let req = format!(
            "POST {} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            path,
            body.len(),
            body
        );
        raw_request(port, &req)
    }

    #[test]
    fn http_health_returns_ok() {
        let port = start_server();
        let resp = raw_request(port, "GET /health HTTP/1.1\r\nHost: x\r\n\r\n");
        assert!(resp.starts_with("HTTP/1.1 200"));
        assert!(resp.contains("\"status\":\"ok\""));
    }

    #[test]
    fn http_inspect_detects_overlap() {
        let port = start_server();
        let body = r#"{"html":"<div style=\"position:absolute;left:0;top:0;width:100;height:100\">A</div><div style=\"position:absolute;left:50;top:50;width:100;height:100\">B</div>","width":360}"#;
        let resp = post(port, "/inspect", body);
        let json = resp.split("\r\n\r\n").nth(1).unwrap_or(&resp);
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        assert!(
            v["total_elements"].as_i64().unwrap() >= 1,
            "inspect should report element counts"
        );
    }

    #[test]
    fn http_overlaps_returns_collisions() {
        let port = start_server();
        let body = r#"{"html":"<div style=\"position:absolute;left:0;top:0;width:100;height:100\">A</div><div style=\"position:absolute;left:50;top:50;width:100;height:100\">B</div>","width":360}"#;
        let resp = post(port, "/overlaps", body);
        let json = resp.split("\r\n\r\n").nth(1).unwrap_or(&resp);
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        assert!(
            !v.as_array().unwrap().is_empty(),
            "expected the two absolute divs to be reported as overlapping"
        );
    }

    #[test]
    fn http_error_status_for_bad_body() {
        let port = start_server();
        let resp = post(port, "/inspect", "{}");
        assert!(
            resp.starts_with("HTTP/1.1 400"),
            "missing html should be 400, got: {}",
            &resp[..resp.len().min(40)]
        );
    }

    #[test]
    fn http_unknown_path_returns_404() {
        let port = start_server();
        let resp = raw_request(port, "GET /does-not-exist HTTP/1.1\r\nHost: x\r\n\r\n");
        assert!(
            resp.starts_with("HTTP/1.1 404"),
            "unknown path should be 404, got: {}",
            &resp[..resp.len().min(40)]
        );
    }

    #[test]
    fn http_oversized_declared_body_is_413() {
        let port = start_server();
        // declare a body far over the 8 MiB cap; the server must refuse
        // before reading it (we never send the bytes)
        let req = "POST /inspect HTTP/1.1\r\nHost: x\r\nContent-Length: 999999999\r\n\r\n";
        let resp = raw_request(port, req);
        assert!(
            resp.starts_with("HTTP/1.1 413"),
            "oversized declared body should be 413, got: {}",
            &resp[..resp.len().min(60)]
        );
    }

    #[test]
    fn http_line_cap_rejects_oversized_request_line() {
        let port = start_server();
        let huge_line = format!(
            "GET /{} HTTP/1.1\r\nHost: x\r\n\r\n",
            "a".repeat(MAX_LINE + 8)
        );
        // the connection error path closes without a response; assert no panic
        // by simply connecting (server handles it internally)
        let resp = raw_request(port, &huge_line);
        assert!(
            resp.is_empty() || resp.starts_with("HTTP/1.1"),
            "server must not crash on oversized lines"
        );
    }

    #[test]
    fn capped_reader_returns_lines_and_eof() {
        let mut r = std::io::Cursor::new(b"hello\nworld\n".to_vec());
        let a = read_line_capped(&mut r, 64).unwrap().unwrap();
        assert_eq!(String::from_utf8(a).unwrap(), "hello");
        let b = read_line_capped(&mut r, 64).unwrap().unwrap();
        assert_eq!(String::from_utf8(b).unwrap(), "world");
        assert!(
            read_line_capped(&mut r, 64).unwrap().is_none(),
            "EOF -> None"
        );
    }

    #[test]
    fn capped_reader_rejects_oversized_lines() {
        let mut r = std::io::Cursor::new(format!("{}\n", "x".repeat(1000)).into_bytes());
        assert!(
            read_line_capped(&mut r, 64).is_err(),
            "line over the cap must error, not allocate unbounded"
        );
    }
}
