use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

use crate::gallery::{collect_photos, generate_html};
use crate::metadata::Metadata;

/// MIME type from file extension.
fn mime_type(path: &Path) -> &'static str {
    match path
        .extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase()
        .as_str()
    {
        "html" => "text/html; charset=utf-8",
        "json" => "application/json",
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "heic" | "heif" => "image/heic",
        "cr2" | "cr3" | "nef" | "arw" | "dng" | "orf" | "rw2" | "raf" => "application/octet-stream",
        "tiff" | "tif" => "image/tiff",
        "css" => "text/css",
        "js" => "application/javascript",
        _ => "application/octet-stream",
    }
}

/// JSON error response helper.
fn json_error(status: u16, msg: &str) -> Response<std::io::Cursor<Vec<u8>>> {
    let body = format!("{{\"error\":\"{}\"}}", msg.replace('"', "\\\""));
    Response::from_string(body)
        .with_status_code(StatusCode(status))
        .with_header(
            Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap(),
        )
}

/// JSON success response helper.
fn json_ok(msg: &str) -> Response<std::io::Cursor<Vec<u8>>> {
    let body = format!("{{\"ok\":\"{}\"}}", msg.replace('"', "\\\""));
    Response::from_string(body).with_header(
        Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap(),
    )
}

/// Read request body as string.
fn read_body(req: &mut Request) -> Result<String> {
    let mut body = String::new();
    req.as_reader()
        .read_to_string(&mut body)
        .context("Failed to read request body")?;
    Ok(body)
}

/// Parse query string into key-value pairs.
pub fn parse_query(url: &str) -> HashMap<String, String> {
    let mut params = HashMap::new();
    if let Some(qs) = url.split('?').nth(1) {
        for pair in qs.split('&') {
            let mut kv = pair.splitn(2, '=');
            if let (Some(k), Some(v)) = (kv.next(), kv.next()) {
                params.insert(
                    urldecode(k),
                    urldecode(v),
                );
            }
        }
    }
    params
}

/// Minimal URL decode (%XX and +).
pub fn urldecode(s: &str) -> String {
    let mut result = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(val) = u8::from_str_radix(
                &String::from_utf8_lossy(&bytes[i + 1..i + 3]),
                16,
            ) {
                result.push(val);
                i += 3;
                continue;
            }
        }
        if bytes[i] == b'+' {
            result.push(b' ');
        } else {
            result.push(bytes[i]);
        }
        i += 1;
    }
    String::from_utf8_lossy(&result).to_string()
}

/// Validate that a relative path doesn't escape the base dir.
pub fn safe_path(base: &Path, relative: &str) -> Option<PathBuf> {
    let clean = relative.replace('\\', "/");
    // Reject absolute paths and traversal
    if clean.starts_with('/') || clean.contains("..") {
        return None;
    }
    let full = base.join(&clean);
    // Verify it's actually under base
    if full.starts_with(base) {
        Some(full)
    } else {
        None
    }
}

/// Handle a single HTTP request.
pub fn handle_request(
    mut req: Request,
    dir: &Path,
    metadata: &Arc<Mutex<Metadata>>,
) {
    let url = req.url().to_string();
    let method = req.method().clone();
    let path = url.split('?').next().unwrap_or(&url);

    match (&method, path) {
        // Gallery HTML
        (&Method::Get, "/") => {
            let meta = metadata.lock().unwrap();
            let photos = collect_photos(dir);
            let html = generate_html(&photos, &meta);
            let resp = Response::from_string(html).with_header(
                Header::from_bytes(&b"Content-Type"[..], &b"text/html; charset=utf-8"[..])
                    .unwrap(),
            );
            let _ = req.respond(resp);
        }

        // API: Save metadata
        (&Method::Post, "/api/metadata") => {
            match read_body(&mut req) {
                Ok(body) => match serde_json::from_str::<Metadata>(&body) {
                    Ok(new_meta) => {
                        let mut meta = metadata.lock().unwrap();
                        *meta = new_meta;
                        match meta.save(dir) {
                            Ok(()) => {
                                let _ = req.respond(json_ok("Metadata sauvegardé"));
                            }
                            Err(e) => {
                                let _ = req.respond(json_error(500, &e.to_string()));
                            }
                        }
                    }
                    Err(e) => {
                        let _ = req.respond(json_error(400, &e.to_string()));
                    }
                },
                Err(e) => {
                    let _ = req.respond(json_error(400, &e.to_string()));
                }
            }
        }

        // API: Delete photo
        (&Method::Delete, "/api/photo") => {
            let params = parse_query(&url);
            if let Some(file) = params.get("path") {
                if let Some(full_path) = safe_path(dir, file) {
                    if full_path.exists() {
                        match std::fs::remove_file(&full_path) {
                            Ok(()) => {
                                let mut meta = metadata.lock().unwrap();
                                meta.files.remove(file.as_str());
                                let _ = meta.save(dir);
                                let _ = req.respond(json_ok("Fichier supprimé"));
                            }
                            Err(e) => {
                                let _ = req.respond(json_error(500, &e.to_string()));
                            }
                        }
                    } else {
                        let _ = req.respond(json_error(404, "Fichier introuvable"));
                    }
                } else {
                    let _ = req.respond(json_error(400, "Chemin invalide"));
                }
            } else {
                let _ = req.respond(json_error(400, "Paramètre path requis"));
            }
        }

        // API: Move photo
        (&Method::Post, "/api/move") => {
            match read_body(&mut req) {
                Ok(body) => {
                    #[derive(serde::Deserialize)]
                    struct MoveReq {
                        src: String,
                        dest_dir: String,
                    }
                    match serde_json::from_str::<MoveReq>(&body) {
                        Ok(mv) => {
                            let src_path = match safe_path(dir, &mv.src) {
                                Some(p) => p,
                                None => {
                                    let _ = req.respond(json_error(400, "Chemin source invalide"));
                                    return;
                                }
                            };
                            if !src_path.exists() {
                                let _ = req.respond(json_error(404, "Fichier source introuvable"));
                                return;
                            }
                            let dest_subdir = dir.join(&mv.dest_dir);
                            if let Err(e) = std::fs::create_dir_all(&dest_subdir) {
                                let _ = req.respond(json_error(500, &e.to_string()));
                                return;
                            }
                            let filename = src_path
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .to_string();
                            let dest_path = dest_subdir.join(&filename);
                            match std::fs::rename(&src_path, &dest_path) {
                                Ok(()) => {
                                    let new_rel = format!("{}/{}", mv.dest_dir, filename);
                                    let mut meta = metadata.lock().unwrap();
                                    if let Some(info) = meta.files.remove(mv.src.as_str()) {
                                        meta.files.insert(new_rel.clone(), info);
                                    }
                                    let _ = meta.save(dir);
                                    let resp_body = format!(
                                        "{{\"ok\":\"Fichier déplacé\",\"new_path\":\"{}\"}}",
                                        new_rel.replace('"', "\\\"")
                                    );
                                    let resp = Response::from_string(resp_body).with_header(
                                        Header::from_bytes(
                                            &b"Content-Type"[..],
                                            &b"application/json"[..],
                                        )
                                        .unwrap(),
                                    );
                                    let _ = req.respond(resp);
                                }
                                Err(e) => {
                                    let _ = req.respond(json_error(500, &e.to_string()));
                                }
                            }
                        }
                        Err(e) => {
                            let _ = req.respond(json_error(400, &e.to_string()));
                        }
                    }
                }
                Err(e) => {
                    let _ = req.respond(json_error(400, &e.to_string()));
                }
            }
        }

        // API: Rotate photo
        (&Method::Post, "/api/rotate") => {
            match read_body(&mut req) {
                Ok(body) => {
                    #[derive(serde::Deserialize)]
                    struct RotateReq {
                        path: String,
                        angle: u16,
                    }
                    match serde_json::from_str::<RotateReq>(&body) {
                        Ok(rot) => {
                            if !matches!(rot.angle, 90 | 180 | 270) {
                                let _ = req.respond(json_error(
                                    400,
                                    "Angle invalide (90, 180 ou 270)",
                                ));
                                return;
                            }
                            let full_path = match safe_path(dir, &rot.path) {
                                Some(p) => p,
                                None => {
                                    let _ = req.respond(json_error(400, "Chemin invalide"));
                                    return;
                                }
                            };
                            if !full_path.exists() {
                                let _ =
                                    req.respond(json_error(404, "Fichier introuvable"));
                                return;
                            }
                            match rotate_image(&full_path, rot.angle) {
                                Ok(()) => {
                                    let _ = req.respond(json_ok("Photo tournée"));
                                }
                                Err(e) => {
                                    let _ = req.respond(json_error(500, &e.to_string()));
                                }
                            }
                        }
                        Err(e) => {
                            let _ = req.respond(json_error(400, &e.to_string()));
                        }
                    }
                }
                Err(e) => {
                    let _ = req.respond(json_error(400, &e.to_string()));
                }
            }
        }

        // Static file serving
        (&Method::Get, _) => {
            let rel = &path[1..]; // strip leading /
            if let Some(full_path) = safe_path(dir, rel) {
                if full_path.is_file() {
                    match std::fs::File::open(&full_path) {
                        Ok(file) => {
                            let len = file.metadata().map(|m| m.len()).unwrap_or(0);
                            let mime = mime_type(&full_path);
                            let resp = Response::from_file(file)
                                .with_header(
                                    Header::from_bytes(&b"Content-Type"[..], mime.as_bytes())
                                        .unwrap(),
                                )
                                .with_header(
                                    Header::from_bytes(
                                        &b"Content-Length"[..],
                                        len.to_string().as_bytes(),
                                    )
                                    .unwrap(),
                                );
                            let _ = req.respond(resp);
                        }
                        Err(_) => {
                            let _ = req.respond(json_error(500, "Erreur lecture fichier"));
                        }
                    }
                } else {
                    let _ = req.respond(json_error(404, "Fichier introuvable"));
                }
            } else {
                let _ = req.respond(json_error(400, "Chemin invalide"));
            }
        }

        _ => {
            let _ = req.respond(json_error(405, "Méthode non supportée"));
        }
    }
}

/// Rotate an image file by the given angle (90, 180, 270 degrees clockwise).
pub fn rotate_image(path: &Path, angle: u16) -> Result<()> {
    let img = image::open(path).context("Impossible d'ouvrir l'image")?;
    let rotated = match angle {
        90 => img.rotate90(),
        180 => img.rotate180(),
        270 => img.rotate270(),
        _ => anyhow::bail!("Angle invalide : {angle}"),
    };
    rotated
        .save(path)
        .context("Impossible de sauvegarder l'image tournée")?;
    Ok(())
}

/// Start the HTTP server.
pub fn run_serve(dir: &Path, port: u16) -> Result<()> {
    let dir = dir
        .canonicalize()
        .with_context(|| format!("Dossier introuvable : {}", dir.display()))?;

    let metadata = Arc::new(Mutex::new(Metadata::load(&dir)?));
    let addr = format!("0.0.0.0:{port}");
    let server =
        Server::http(&addr).map_err(|e| anyhow::anyhow!("Impossible de démarrer le serveur: {e}"))?;

    println!(
        "  {} Galerie disponible sur {}",
        console::style("✔").green().bold(),
        console::style(format!("http://localhost:{port}")).cyan().bold()
    );
    println!(
        "  {} pour arrêter",
        console::style("Ctrl+C").yellow().bold()
    );

    for req in server.incoming_requests() {
        let dir = dir.clone();
        let metadata = metadata.clone();
        std::thread::spawn(move || {
            handle_request(req, &dir, &metadata);
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    static TEST_COUNTER: AtomicU32 = AtomicU32::new(0);

    fn tmpdir() -> PathBuf {
        let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "photo_sort_serve_test_{}_{id}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn setup_photos(dir: &Path) {
        let y = dir.join("2020");
        std::fs::create_dir_all(&y).unwrap();
        std::fs::write(y.join("a.jpg"), "fake jpg data").unwrap();
        std::fs::write(y.join("b.jpg"), "fake jpg data 2").unwrap();
    }

    // --- parse_query ---

    #[test]
    fn parse_query_extracts_params() {
        let params = parse_query("/api/photo?path=2020/a.jpg&foo=bar");
        assert_eq!(params.get("path").unwrap(), "2020/a.jpg");
        assert_eq!(params.get("foo").unwrap(), "bar");
    }

    #[test]
    fn parse_query_empty_when_no_query() {
        let params = parse_query("/api/photo");
        assert!(params.is_empty());
    }

    #[test]
    fn parse_query_decodes_percent() {
        let params = parse_query("/api?name=hello%20world");
        assert_eq!(params.get("name").unwrap(), "hello world");
    }

    // --- urldecode ---

    #[test]
    fn urldecode_decodes_percent_and_plus() {
        assert_eq!(urldecode("hello+world"), "hello world");
        assert_eq!(urldecode("hello%20world"), "hello world");
        assert_eq!(urldecode("a%2Fb"), "a/b");
    }

    // --- safe_path ---

    #[test]
    fn safe_path_allows_relative() {
        let base = Path::new("/photos");
        assert!(safe_path(base, "2020/a.jpg").is_some());
    }

    #[test]
    fn safe_path_rejects_traversal() {
        let base = Path::new("/photos");
        assert!(safe_path(base, "../etc/passwd").is_none());
        assert!(safe_path(base, "2020/../../etc/passwd").is_none());
    }

    #[test]
    fn safe_path_rejects_absolute() {
        let base = Path::new("/photos");
        assert!(safe_path(base, "/etc/passwd").is_none());
    }

    // --- mime_type ---

    #[test]
    fn mime_type_for_common_formats() {
        assert_eq!(mime_type(Path::new("photo.jpg")), "image/jpeg");
        assert_eq!(mime_type(Path::new("photo.png")), "image/png");
        assert_eq!(mime_type(Path::new("page.html")), "text/html; charset=utf-8");
        assert_eq!(mime_type(Path::new("data.json")), "application/json");
        assert_eq!(mime_type(Path::new("raw.cr2")), "application/octet-stream");
    }

    // --- Integration: handle_request with real server ---
    // We test API logic by spawning a tiny_http server on a random port

    fn spawn_test_server(dir: &Path) -> (u16, Arc<Mutex<Metadata>>) {
        let metadata = Arc::new(Mutex::new(Metadata::load(dir).unwrap()));
        // port 0 = OS picks a free port
        let server = Server::http("127.0.0.1:0").unwrap();
        let port = server.server_addr().to_ip().unwrap().port();
        let dir = dir.to_path_buf();
        let meta_clone = metadata.clone();
        std::thread::spawn(move || {
            for req in server.incoming_requests() {
                handle_request(req, &dir, &meta_clone);
            }
        });
        (port, metadata)
    }

    #[test]
    fn serve_gallery_returns_html() {
        let tmp = tmpdir();
        setup_photos(&tmp);
        let (port, _) = spawn_test_server(&tmp);

        let resp = ureq_get(&format!("http://127.0.0.1:{port}/"));
        assert!(resp.contains("<!DOCTYPE html>"));
        assert!(resp.contains("photo-sort gallery"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn serve_static_file() {
        let tmp = tmpdir();
        setup_photos(&tmp);
        let (port, _) = spawn_test_server(&tmp);

        let resp = ureq_get(&format!("http://127.0.0.1:{port}/2020/a.jpg"));
        assert_eq!(resp, "fake jpg data");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn serve_404_for_missing_file() {
        let tmp = tmpdir();
        setup_photos(&tmp);
        let (port, _) = spawn_test_server(&tmp);

        let resp = ureq_get(&format!("http://127.0.0.1:{port}/nonexistent.jpg"));
        assert!(resp.contains("error"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn api_delete_photo() {
        let tmp = tmpdir();
        setup_photos(&tmp);
        let (port, _meta) = spawn_test_server(&tmp);

        assert!(tmp.join("2020/a.jpg").exists());
        let resp = ureq_delete(&format!(
            "http://127.0.0.1:{port}/api/photo?path=2020/a.jpg"
        ));
        assert!(resp.contains("ok"));
        assert!(!tmp.join("2020/a.jpg").exists());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn api_delete_rejects_traversal() {
        let tmp = tmpdir();
        setup_photos(&tmp);
        let (port, _) = spawn_test_server(&tmp);

        let resp = ureq_delete(&format!(
            "http://127.0.0.1:{port}/api/photo?path=../../../etc/passwd"
        ));
        assert!(resp.contains("error"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn api_move_photo() {
        let tmp = tmpdir();
        setup_photos(&tmp);
        let (port, _) = spawn_test_server(&tmp);

        let body = r#"{"src":"2020/a.jpg","dest_dir":"2021"}"#;
        let resp = ureq_post(
            &format!("http://127.0.0.1:{port}/api/move"),
            body,
        );
        assert!(resp.contains("ok"));
        assert!(!tmp.join("2020/a.jpg").exists());
        assert!(tmp.join("2021/a.jpg").exists());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn api_metadata_save() {
        let tmp = tmpdir();
        setup_photos(&tmp);
        let (port, _) = spawn_test_server(&tmp);

        let body = r#"{"files":{"2020/a.jpg":{"tags":["test"],"rating":5}}}"#;
        let resp = ureq_post(
            &format!("http://127.0.0.1:{port}/api/metadata"),
            body,
        );
        assert!(resp.contains("ok"));
        // Verify it was saved to disk
        let meta = Metadata::load(&tmp).unwrap();
        assert_eq!(meta.get_tags("2020/a.jpg"), &["test"]);
        assert_eq!(meta.get_rating("2020/a.jpg"), Some(5));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    // --- Helpers for HTTP requests (minimal, no deps) ---

    fn ureq_get(url: &str) -> String {
        use std::io::{BufRead, BufReader, Write};
        use std::net::TcpStream;
        let url = url.strip_prefix("http://").unwrap();
        let (host, path) = url.split_once('/').unwrap_or((url, ""));
        let path = format!("/{path}");
        let mut stream = TcpStream::connect(host).unwrap();
        write!(stream, "GET {path} HTTP/1.0\r\nHost: {host}\r\n\r\n").unwrap();
        let reader = BufReader::new(&stream);
        let mut body = false;
        let mut result = String::new();
        for line in reader.lines() {
            let line = line.unwrap();
            if body {
                result.push_str(&line);
                result.push('\n');
            } else if line.is_empty() {
                body = true;
            }
        }
        result.trim().to_string()
    }

    fn ureq_delete(url: &str) -> String {
        use std::io::{BufRead, BufReader, Write};
        use std::net::TcpStream;
        let url = url.strip_prefix("http://").unwrap();
        let (host, path) = url.split_once('/').unwrap_or((url, ""));
        let path = format!("/{path}");
        let mut stream = TcpStream::connect(host).unwrap();
        write!(stream, "DELETE {path} HTTP/1.0\r\nHost: {host}\r\n\r\n").unwrap();
        let reader = BufReader::new(&stream);
        let mut body = false;
        let mut result = String::new();
        for line in reader.lines() {
            let line = line.unwrap();
            if body {
                result.push_str(&line);
                result.push('\n');
            } else if line.is_empty() {
                body = true;
            }
        }
        result.trim().to_string()
    }

    fn ureq_post(url: &str, body_str: &str) -> String {
        use std::io::{BufRead, BufReader, Write};
        use std::net::TcpStream;
        let url = url.strip_prefix("http://").unwrap();
        let (host, path) = url.split_once('/').unwrap_or((url, ""));
        let path = format!("/{path}");
        let mut stream = TcpStream::connect(host).unwrap();
        write!(
            stream,
            "POST {path} HTTP/1.0\r\nHost: {host}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body_str}",
            body_str.len()
        )
        .unwrap();
        let reader = BufReader::new(&stream);
        let mut body = false;
        let mut result = String::new();
        for line in reader.lines() {
            let line = line.unwrap();
            if body {
                result.push_str(&line);
                result.push('\n');
            } else if line.is_empty() {
                body = true;
            }
        }
        result.trim().to_string()
    }
}
