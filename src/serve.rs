use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

use crate::gallery::{collect_photos, generate_html};
use crate::metadata::Metadata;
use crate::thumb;

/// Server state: caches the photo index and generated HTML.
pub struct ServerState {
    pub dir: PathBuf,
    metadata: Mutex<Metadata>,
    photo_index: Mutex<HashMap<String, Vec<String>>>,
    html_cache: Mutex<Option<Arc<String>>>,
    cache_gen: AtomicU64,
}

impl ServerState {
    /// Build the initial state: canonicalize dir, load metadata, collect photos,
    /// and pre-generate the HTML so the first request is instant.
    pub fn new(dir: &Path) -> Result<Arc<Self>> {
        let dir = dir
            .canonicalize()
            .with_context(|| format!("Dossier introuvable : {}", dir.display()))?;
        let metadata = Metadata::load(&dir)?;
        let photo_index = collect_photos(&dir);
        let html = generate_html(&photo_index, &metadata);
        Ok(Arc::new(Self {
            dir,
            metadata: Mutex::new(metadata),
            photo_index: Mutex::new(photo_index),
            html_cache: Mutex::new(Some(Arc::new(html))),
            cache_gen: AtomicU64::new(0),
        }))
    }

    /// Return the cached HTML, regenerating it if the cache was invalidated.
    pub fn get_cached_html(&self) -> Arc<String> {
        // Fast path: cache hit
        {
            let cache = self.html_cache.lock().unwrap();
            if let Some(ref html) = *cache {
                return Arc::clone(html);
            }
        }

        // Slow path: regenerate
        let gen_before = self.cache_gen.load(Ordering::Acquire);
        let index = self.photo_index.lock().unwrap().clone();
        let meta = self.metadata.lock().unwrap();
        let html = Arc::new(generate_html(&index, &meta));
        drop(meta);

        // Only store if no mutation happened while we were generating
        let gen_after = self.cache_gen.load(Ordering::Acquire);
        if gen_before == gen_after {
            let mut cache = self.html_cache.lock().unwrap();
            *cache = Some(Arc::clone(&html));
        }
        html
    }

    /// Bump the generation counter and clear the HTML cache.
    fn invalidate_cache(&self) {
        self.cache_gen.fetch_add(1, Ordering::Release);
        let mut cache = self.html_cache.lock().unwrap();
        *cache = None;
    }

    /// Return all relative photo paths (flat list) from the index.
    pub fn all_photo_rels(&self) -> Vec<String> {
        let index = self.photo_index.lock().unwrap();
        index.values().flat_map(|v| v.iter().cloned()).collect()
    }
}

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

/// Extract the year (first path component, 4 digits) from a relative path.
fn year_of(rel: &str) -> Option<&str> {
    let year = rel.split('/').next()?;
    if year.len() == 4 && year.chars().all(|c| c.is_ascii_digit()) {
        Some(year)
    } else {
        None
    }
}

/// Handle a single HTTP request.
pub fn handle_request(mut req: Request, state: &ServerState) {
    let url = req.url().to_string();
    let method = req.method().clone();
    let path = url.split('?').next().unwrap_or(&url);

    match (&method, path) {
        // Gallery HTML — served from cache
        (&Method::Get, "/") => {
            let html = state.get_cached_html();
            let resp = Response::from_string(html.as_str()).with_header(
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
                        let mut meta = state.metadata.lock().unwrap();
                        *meta = new_meta;
                        match meta.save(&state.dir) {
                            Ok(()) => {
                                drop(meta);
                                state.invalidate_cache();
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
                if let Some(full_path) = safe_path(&state.dir, file) {
                    if full_path.exists() {
                        match std::fs::remove_file(&full_path) {
                            Ok(()) => {
                                thumb::invalidate_thumb(&state.dir, file);
                                // Update metadata
                                {
                                    let mut meta = state.metadata.lock().unwrap();
                                    meta.files.remove(file.as_str());
                                    let _ = meta.save(&state.dir);
                                }
                                // Update photo index in-place
                                if let Some(year) = year_of(file) {
                                    let mut index = state.photo_index.lock().unwrap();
                                    if let Some(files) = index.get_mut(year) {
                                        files.retain(|f| f != file);
                                        if files.is_empty() {
                                            index.remove(year);
                                        }
                                    }
                                }
                                state.invalidate_cache();
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
                            let src_path = match safe_path(&state.dir, &mv.src) {
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
                            let dest_subdir = state.dir.join(&mv.dest_dir);
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
                                    thumb::invalidate_thumb(&state.dir, &mv.src);
                                    let new_rel = format!("{}/{}", mv.dest_dir, filename);
                                    // Update metadata
                                    {
                                        let mut meta = state.metadata.lock().unwrap();
                                        if let Some(info) = meta.files.remove(mv.src.as_str()) {
                                            meta.files.insert(new_rel.clone(), info);
                                        }
                                        let _ = meta.save(&state.dir);
                                    }
                                    // Update photo index in-place
                                    {
                                        let mut index = state.photo_index.lock().unwrap();
                                        // Remove from old year
                                        if let Some(old_year) = year_of(&mv.src) {
                                            if let Some(files) = index.get_mut(old_year) {
                                                files.retain(|f| f != &mv.src);
                                                if files.is_empty() {
                                                    index.remove(old_year);
                                                }
                                            }
                                        }
                                        // Insert into new year (sorted)
                                        if let Some(new_year) = year_of(&new_rel) {
                                            let files = index
                                                .entry(new_year.to_string())
                                                .or_default();
                                            let pos = files
                                                .binary_search(&new_rel)
                                                .unwrap_or_else(|i| i);
                                            files.insert(pos, new_rel.clone());
                                        }
                                    }
                                    state.invalidate_cache();
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

        // API: Rotate photo — no HTML invalidation (JS cache-busts the image)
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
                            let full_path = match safe_path(&state.dir, &rot.path) {
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
                                    thumb::invalidate_thumb(&state.dir, &rot.path);
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

        // Thumbnail serving
        (&Method::Get, _) if path.starts_with("/thumb/") => {
            let rel = &path[7..]; // strip "/thumb/"
            if let Some(full_path) = safe_path(&state.dir, rel) {
                if !full_path.is_file() {
                    let _ = req.respond(json_error(404, "Fichier introuvable"));
                    return;
                }
                // Try to serve thumbnail; fall back to original on error or unsupported format
                let serve_path = match thumb::get_or_create_thumb(&state.dir, rel) {
                    Ok(Some(thumb_path)) => thumb_path,
                    _ => full_path,
                };
                match std::fs::File::open(&serve_path) {
                    Ok(file) => {
                        let len = file.metadata().map(|m| m.len()).unwrap_or(0);
                        let mime = mime_type(&serve_path);
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
                let _ = req.respond(json_error(400, "Chemin invalide"));
            }
        }

        // Static file serving
        (&Method::Get, _) => {
            let rel = &path[1..]; // strip leading /
            if let Some(full_path) = safe_path(&state.dir, rel) {
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
    let state = ServerState::new(dir)?;

    // Pre-generate thumbnails in the background
    let all_rels = state.all_photo_rels();
    thumb::spawn_prewarm(state.dir.clone(), all_rels);

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
        let state = Arc::clone(&state);
        std::thread::spawn(move || {
            handle_request(req, &state);
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering as AtomicOrdering};

    static TEST_COUNTER: AtomicU32 = AtomicU32::new(0);

    fn tmpdir() -> PathBuf {
        let id = TEST_COUNTER.fetch_add(1, AtomicOrdering::SeqCst);
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

    fn spawn_test_server(dir: &Path) -> (u16, Arc<ServerState>) {
        let state = ServerState::new(dir).unwrap();
        let server = Server::http("127.0.0.1:0").unwrap();
        let port = server.server_addr().to_ip().unwrap().port();
        let state_clone = Arc::clone(&state);
        std::thread::spawn(move || {
            for req in server.incoming_requests() {
                handle_request(req, &state_clone);
            }
        });
        (port, state)
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
        let (port, _state) = spawn_test_server(&tmp);

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

    // --- Cache tests ---

    #[test]
    fn html_cache_hit() {
        let tmp = tmpdir();
        setup_photos(&tmp);
        let state = ServerState::new(&tmp).unwrap();

        let html1 = state.get_cached_html();
        let html2 = state.get_cached_html();
        // Both should point to the same Arc allocation
        assert!(Arc::ptr_eq(&html1, &html2));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn cache_invalidated_after_delete() {
        let tmp = tmpdir();
        setup_photos(&tmp);
        let (port, state) = spawn_test_server(&tmp);

        let html_before = state.get_cached_html();
        assert!(html_before.contains("a.jpg"));

        let resp = ureq_delete(&format!(
            "http://127.0.0.1:{port}/api/photo?path=2020/a.jpg"
        ));
        assert!(resp.contains("ok"));

        let html_after = state.get_cached_html();
        assert!(!Arc::ptr_eq(&html_before, &html_after));
        assert!(!html_after.contains("2020/a.jpg"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn cache_invalidated_after_move() {
        let tmp = tmpdir();
        setup_photos(&tmp);
        let (port, state) = spawn_test_server(&tmp);

        let html_before = state.get_cached_html();
        assert!(html_before.contains("2020/a.jpg"));

        let body = r#"{"src":"2020/a.jpg","dest_dir":"2021"}"#;
        let resp = ureq_post(
            &format!("http://127.0.0.1:{port}/api/move"),
            body,
        );
        assert!(resp.contains("ok"));

        let html_after = state.get_cached_html();
        assert!(!html_after.contains("2020/a.jpg"));
        assert!(html_after.contains("2021/a.jpg"));
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

    fn ureq_get_bytes(url: &str) -> Vec<u8> {
        use std::io::{Read, Write};
        use std::net::TcpStream;
        let url = url.strip_prefix("http://").unwrap();
        let (host, path) = url.split_once('/').unwrap_or((url, ""));
        let path = format!("/{path}");
        let mut stream = TcpStream::connect(host).unwrap();
        write!(stream, "GET {path} HTTP/1.0\r\nHost: {host}\r\n\r\n").unwrap();
        let mut buf = Vec::new();
        stream.read_to_end(&mut buf).unwrap();
        // Split at \r\n\r\n to get body
        let sep = b"\r\n\r\n";
        if let Some(pos) = buf.windows(4).position(|w| w == sep) {
            buf[pos + 4..].to_vec()
        } else {
            buf
        }
    }

    /// Create a real JPEG image for integration tests.
    fn create_test_jpeg(path: &Path) {
        let img = image::RgbImage::from_fn(100, 80, |x, y| {
            image::Rgb([(x % 256) as u8, (y % 256) as u8, 128])
        });
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        img.save(path).unwrap();
    }

    // --- Thumbnail endpoint ---

    #[test]
    fn thumb_endpoint_returns_jpeg() {
        let tmp = tmpdir();
        let y = tmp.join("2020");
        std::fs::create_dir_all(&y).unwrap();
        create_test_jpeg(&y.join("photo.jpg"));

        let (port, _) = spawn_test_server(&tmp);

        let bytes = ureq_get_bytes(&format!("http://127.0.0.1:{port}/thumb/2020/photo.jpg"));
        // JPEG starts with FF D8
        assert!(bytes.len() > 2);
        assert_eq!(bytes[0], 0xFF);
        assert_eq!(bytes[1], 0xD8);

        // Cache file should exist
        let cache = tmp.join(".photo_sort_thumbs/2020/photo.jpg");
        assert!(cache.exists());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn thumb_endpoint_fallback_for_heic() {
        let tmp = tmpdir();
        let y = tmp.join("2020");
        std::fs::create_dir_all(&y).unwrap();
        std::fs::write(y.join("photo.heic"), "fake heic data").unwrap();

        let (port, _) = spawn_test_server(&tmp);

        // Should fall back to serving the original file
        let resp = ureq_get(&format!("http://127.0.0.1:{port}/thumb/2020/photo.heic"));
        assert_eq!(resp, "fake heic data");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn thumb_endpoint_404_for_missing() {
        let tmp = tmpdir();
        setup_photos(&tmp);
        let (port, _) = spawn_test_server(&tmp);

        let resp = ureq_get(&format!("http://127.0.0.1:{port}/thumb/2020/nonexistent.jpg"));
        assert!(resp.contains("error"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn thumb_endpoint_rejects_traversal() {
        let tmp = tmpdir();
        setup_photos(&tmp);
        let (port, _) = spawn_test_server(&tmp);

        let resp = ureq_get(&format!(
            "http://127.0.0.1:{port}/thumb/../../../etc/passwd"
        ));
        assert!(resp.contains("error"));
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
