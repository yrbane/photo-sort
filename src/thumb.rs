use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// Name of the thumbnail cache directory (created inside the photo base dir).
pub const THUMB_DIR: &str = ".photo_sort_thumbs";

/// Maximum width (in pixels) for generated thumbnails.
const THUMB_MAX_SIZE: u32 = 300;

/// JPEG quality for thumbnails (0–100).
const THUMB_QUALITY: u8 = 80;

/// Extensions that the `image` crate can decode (subset of PHOTO_EXTENSIONS).
const SUPPORTED_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "tiff", "tif"];

/// Return the cache path for a given relative photo path.
/// E.g. `thumb_cache_path("/photos", "2020/a.jpg")` → `/photos/.photo_sort_thumbs/2020/a.jpg`
/// The cached file always gets a `.jpg` extension.
pub fn thumb_cache_path(base: &Path, rel: &str) -> PathBuf {
    let mut p = base.join(THUMB_DIR).join(rel);
    p.set_extension("jpg");
    p
}

/// Return `true` if the cached thumbnail is still fresh (newer than the source).
pub fn thumb_is_fresh(source: &Path, cached: &Path) -> bool {
    let Ok(src_meta) = source.metadata() else {
        return false;
    };
    let Ok(cache_meta) = cached.metadata() else {
        return false;
    };
    let Ok(src_mtime) = src_meta.modified() else {
        return false;
    };
    let Ok(cache_mtime) = cache_meta.modified() else {
        return false;
    };
    cache_mtime >= src_mtime
}

/// Return `true` if we can generate a thumbnail for this file extension.
pub fn can_generate_thumb(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| SUPPORTED_EXTENSIONS.contains(&ext.to_lowercase().as_str()))
}

/// Generate a JPEG thumbnail from `source` and write it to `dest`.
pub fn generate_thumb(source: &Path, dest: &Path) -> Result<()> {
    let img = image::open(source)
        .with_context(|| format!("Cannot open image: {}", source.display()))?;

    let thumb = img.thumbnail(THUMB_MAX_SIZE, THUMB_MAX_SIZE);

    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Cannot create thumb dir: {}", parent.display()))?;
    }

    let mut out = std::fs::File::create(dest)
        .with_context(|| format!("Cannot create thumb file: {}", dest.display()))?;

    let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, THUMB_QUALITY);
    thumb
        .write_with_encoder(encoder)
        .context("Failed to encode thumbnail")?;

    Ok(())
}

/// Return the path to a cached (or freshly generated) thumbnail.
/// Returns `None` if the format is not supported for thumbnail generation.
pub fn get_or_create_thumb(base: &Path, rel: &str) -> Result<Option<PathBuf>> {
    let source = base.join(rel);
    if !can_generate_thumb(&source) {
        return Ok(None);
    }

    let cached = thumb_cache_path(base, rel);
    if cached.exists() && thumb_is_fresh(&source, &cached) {
        return Ok(Some(cached));
    }

    generate_thumb(&source, &cached)?;
    Ok(Some(cached))
}

/// Delete the cached thumbnail for a given relative path (if it exists).
pub fn invalidate_thumb(base: &Path, rel: &str) {
    let cached = thumb_cache_path(base, rel);
    let _ = std::fs::remove_file(cached);
}

/// Spawn a background thread that pre-generates thumbnails for all given photos.
/// Photos that already have a fresh thumbnail are skipped.
pub fn spawn_prewarm(base: PathBuf, rels: Vec<String>) {
    std::thread::spawn(move || {
        prewarm_thumbnails(&base, &rels);
    });
}

/// Pre-generate thumbnails in parallel using a scoped thread pool.
fn prewarm_thumbnails(base: &Path, rels: &[String]) {
    // Filter to only photos that need a thumbnail generated
    let to_generate: Vec<&String> = rels
        .iter()
        .filter(|rel| {
            let source = base.join(rel.as_str());
            if !can_generate_thumb(&source) {
                return false;
            }
            let cached = thumb_cache_path(base, rel);
            !thumb_is_fresh(&source, &cached)
        })
        .collect();

    if to_generate.is_empty() {
        return;
    }

    let n_workers = std::thread::available_parallelism()
        .map(|n| n.get().min(8))
        .unwrap_or(4);

    let chunks: Vec<&[&String]> = to_generate.chunks(
        (to_generate.len() + n_workers - 1) / n_workers
    ).collect();

    std::thread::scope(|s| {
        for chunk in chunks {
            s.spawn(move || {
                for rel in chunk {
                    let _ = get_or_create_thumb(base, rel);
                }
            });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    static TEST_COUNTER: AtomicU32 = AtomicU32::new(0);

    fn tmpdir() -> PathBuf {
        let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "photo_sort_thumb_test_{}_{id}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Create a small real JPEG file for testing.
    fn create_test_jpeg(path: &Path) {
        let img = image::RgbImage::from_fn(100, 80, |x, y| {
            image::Rgb([(x % 256) as u8, (y % 256) as u8, 128])
        });
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        img.save(path).unwrap();
    }

    /// Create a small real PNG file for testing.
    fn create_test_png(path: &Path) {
        let img = image::RgbImage::from_fn(80, 60, |x, y| {
            image::Rgb([128, (x % 256) as u8, (y % 256) as u8])
        });
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        img.save(path).unwrap();
    }

    // --- thumb_cache_path ---

    #[test]
    fn cache_path_under_thumb_dir() {
        let p = thumb_cache_path(Path::new("/photos"), "2020/a.jpg");
        assert_eq!(p, PathBuf::from("/photos/.photo_sort_thumbs/2020/a.jpg"));
    }

    #[test]
    fn cache_path_converts_extension_to_jpg() {
        let p = thumb_cache_path(Path::new("/photos"), "2020/img.png");
        assert_eq!(p, PathBuf::from("/photos/.photo_sort_thumbs/2020/img.jpg"));
    }

    #[test]
    fn cache_path_preserves_nested_dirs() {
        let p = thumb_cache_path(Path::new("/base"), "2020/sub/deep/photo.tiff");
        assert_eq!(
            p,
            PathBuf::from("/base/.photo_sort_thumbs/2020/sub/deep/photo.jpg")
        );
    }

    // --- can_generate_thumb ---

    #[test]
    fn can_generate_for_supported_formats() {
        assert!(can_generate_thumb(Path::new("photo.jpg")));
        assert!(can_generate_thumb(Path::new("photo.JPEG")));
        assert!(can_generate_thumb(Path::new("photo.png")));
        assert!(can_generate_thumb(Path::new("photo.tiff")));
        assert!(can_generate_thumb(Path::new("photo.tif")));
    }

    #[test]
    fn cannot_generate_for_unsupported_formats() {
        assert!(!can_generate_thumb(Path::new("photo.heic")));
        assert!(!can_generate_thumb(Path::new("photo.cr2")));
        assert!(!can_generate_thumb(Path::new("photo.nef")));
        assert!(!can_generate_thumb(Path::new("photo.arw")));
        assert!(!can_generate_thumb(Path::new("photo.dng")));
    }

    #[test]
    fn cannot_generate_for_no_extension() {
        assert!(!can_generate_thumb(Path::new("photo")));
    }

    // --- thumb_is_fresh ---

    #[test]
    fn fresh_when_cache_newer() {
        let tmp = tmpdir();
        let src = tmp.join("src.jpg");
        let cache = tmp.join("cache.jpg");
        create_test_jpeg(&src);
        // Sleep a tiny bit so mtime differs
        std::thread::sleep(std::time::Duration::from_millis(50));
        std::fs::write(&cache, "cached").unwrap();

        assert!(thumb_is_fresh(&src, &cache));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn stale_when_cache_missing() {
        let tmp = tmpdir();
        let src = tmp.join("src.jpg");
        create_test_jpeg(&src);

        assert!(!thumb_is_fresh(&src, &tmp.join("nonexistent.jpg")));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn stale_when_source_missing() {
        let tmp = tmpdir();
        let cache = tmp.join("cache.jpg");
        std::fs::write(&cache, "cached").unwrap();

        assert!(!thumb_is_fresh(&tmp.join("nonexistent.jpg"), &cache));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    // --- generate_thumb ---

    #[test]
    fn generate_thumb_creates_smaller_jpeg() {
        let tmp = tmpdir();
        let src = tmp.join("2020/photo.jpg");
        let dest = tmp.join("thumb/photo.jpg");
        create_test_jpeg(&src);

        generate_thumb(&src, &dest).unwrap();

        assert!(dest.exists());
        // Thumbnail should be smaller than source
        let src_size = std::fs::metadata(&src).unwrap().len();
        let dest_size = std::fs::metadata(&dest).unwrap().len();
        assert!(dest_size < src_size || dest_size > 0);

        // Verify dimensions
        let thumb_img = image::open(&dest).unwrap();
        assert!(thumb_img.width() <= 300);
        assert!(thumb_img.height() <= 300);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn generate_thumb_from_png() {
        let tmp = tmpdir();
        let src = tmp.join("img.png");
        let dest = tmp.join("thumb.jpg");
        create_test_png(&src);

        generate_thumb(&src, &dest).unwrap();
        assert!(dest.exists());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn generate_thumb_creates_parent_dirs() {
        let tmp = tmpdir();
        let src = tmp.join("photo.jpg");
        let dest = tmp.join("deep/nested/dir/thumb.jpg");
        create_test_jpeg(&src);

        generate_thumb(&src, &dest).unwrap();
        assert!(dest.exists());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    // --- get_or_create_thumb ---

    #[test]
    fn get_or_create_generates_and_caches() {
        let tmp = tmpdir();
        let src = tmp.join("2020/photo.jpg");
        create_test_jpeg(&src);

        let result = get_or_create_thumb(&tmp, "2020/photo.jpg").unwrap();
        assert!(result.is_some());
        let cached = result.unwrap();
        assert!(cached.exists());
        assert_eq!(cached, thumb_cache_path(&tmp, "2020/photo.jpg"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn get_or_create_returns_cached() {
        let tmp = tmpdir();
        let src = tmp.join("2020/photo.jpg");
        create_test_jpeg(&src);

        // First call generates
        let r1 = get_or_create_thumb(&tmp, "2020/photo.jpg").unwrap().unwrap();
        let mtime1 = std::fs::metadata(&r1).unwrap().modified().unwrap();

        // Small delay
        std::thread::sleep(std::time::Duration::from_millis(50));

        // Second call should use cache (same mtime)
        let r2 = get_or_create_thumb(&tmp, "2020/photo.jpg").unwrap().unwrap();
        let mtime2 = std::fs::metadata(&r2).unwrap().modified().unwrap();
        assert_eq!(mtime1, mtime2);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn get_or_create_returns_none_for_heic() {
        let tmp = tmpdir();
        let src = tmp.join("2020/photo.heic");
        std::fs::create_dir_all(src.parent().unwrap()).unwrap();
        std::fs::write(&src, "fake heic").unwrap();

        let result = get_or_create_thumb(&tmp, "2020/photo.heic").unwrap();
        assert!(result.is_none());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    // --- invalidate_thumb ---

    #[test]
    fn invalidate_removes_cached_file() {
        let tmp = tmpdir();
        let src = tmp.join("2020/photo.jpg");
        create_test_jpeg(&src);

        // Generate thumb
        get_or_create_thumb(&tmp, "2020/photo.jpg").unwrap();
        let cached = thumb_cache_path(&tmp, "2020/photo.jpg");
        assert!(cached.exists());

        // Invalidate
        invalidate_thumb(&tmp, "2020/photo.jpg");
        assert!(!cached.exists());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn invalidate_noop_when_no_cache() {
        let tmp = tmpdir();
        // Should not panic
        invalidate_thumb(&tmp, "2020/nonexistent.jpg");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    // --- prewarm ---

    #[test]
    fn prewarm_generates_missing_thumbs() {
        let tmp = tmpdir();
        create_test_jpeg(&tmp.join("2020/a.jpg"));
        create_test_jpeg(&tmp.join("2020/b.jpg"));

        let rels = vec!["2020/a.jpg".to_string(), "2020/b.jpg".to_string()];
        prewarm_thumbnails(&tmp, &rels);

        assert!(thumb_cache_path(&tmp, "2020/a.jpg").exists());
        assert!(thumb_cache_path(&tmp, "2020/b.jpg").exists());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn prewarm_skips_already_cached() {
        let tmp = tmpdir();
        create_test_jpeg(&tmp.join("2020/a.jpg"));

        // Pre-generate one thumb
        get_or_create_thumb(&tmp, "2020/a.jpg").unwrap();
        let cached = thumb_cache_path(&tmp, "2020/a.jpg");
        let mtime_before = std::fs::metadata(&cached).unwrap().modified().unwrap();

        std::thread::sleep(std::time::Duration::from_millis(50));

        let rels = vec!["2020/a.jpg".to_string()];
        prewarm_thumbnails(&tmp, &rels);

        // mtime should be unchanged (was skipped)
        let mtime_after = std::fs::metadata(&cached).unwrap().modified().unwrap();
        assert_eq!(mtime_before, mtime_after);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn prewarm_skips_unsupported_formats() {
        let tmp = tmpdir();
        std::fs::create_dir_all(tmp.join("2020")).unwrap();
        std::fs::write(tmp.join("2020/photo.heic"), "fake heic").unwrap();

        let rels = vec!["2020/photo.heic".to_string()];
        prewarm_thumbnails(&tmp, &rels);

        assert!(!thumb_cache_path(&tmp, "2020/photo.heic").exists());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn spawn_prewarm_runs_in_background() {
        let tmp = tmpdir();
        create_test_jpeg(&tmp.join("2020/a.jpg"));

        let rels = vec!["2020/a.jpg".to_string()];
        spawn_prewarm(tmp.clone(), rels);

        // Wait for the background thread to finish
        for _ in 0..100 {
            if thumb_cache_path(&tmp, "2020/a.jpg").exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(thumb_cache_path(&tmp, "2020/a.jpg").exists());
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
