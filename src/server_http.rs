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

    let mut request_line = String::new();
    if reader.read_line(&mut request_line)? == 0 {
        return Ok(());
    }
    let mut parts = request_line.split_whitespace();
    let _method = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("/").to_string();

    // Headers: capture Content-Length and skip the rest.
    let mut content_length = 0usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some((k, v)) = line.split_once(':')
            && k.eq_ignore_ascii_case("content-length")
        {
            content_length = v.trim().parse().unwrap_or(0);
        }
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
        500 => "Internal Server Error",
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
}
