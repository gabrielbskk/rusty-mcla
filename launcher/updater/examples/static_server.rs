//! Serves a folder over http for design checks in a browser: `static_server <dir> <port>` (used for `launcher/ui`).

use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let dir = PathBuf::from(args.first().expect("static_server <dir> <port>"));
    let port: u16 = args.get(1).expect("port").parse().unwrap();
    let server = tiny_http::Server::http(format!("127.0.0.1:{port}")).unwrap();
    println!("serving {} on http://127.0.0.1:{port}", dir.display());
    for req in server.incoming_requests() {
        let path = req.url().split('?').next().unwrap_or("/").trim_start_matches('/').to_string();
        let path = if path.is_empty() { "index.html".to_string() } else { path };
        let file = dir.join(&path);
        let ok = !path.contains("..") && file.is_file();
        if !ok {
            let _ = req.respond(tiny_http::Response::from_string("not found").with_status_code(404));
            continue;
        }
        let ctype = match file.extension().and_then(|e| e.to_str()) {
            Some("html") => "text/html; charset=utf-8",
            Some("css") => "text/css",
            Some("js") => "text/javascript",
            Some("json") => "application/json",
            Some("svg") => "image/svg+xml",
            _ => "application/octet-stream",
        };
        let mut r = tiny_http::Response::from_data(std::fs::read(&file).unwrap());
        r = r.with_header(tiny_http::Header::from_bytes(&b"Content-Type"[..], ctype.as_bytes()).unwrap());
        r = r.with_header(tiny_http::Header::from_bytes(&b"Cache-Control"[..], &b"no-store"[..]).unwrap());
        let _ = req.respond(r);
    }
}
