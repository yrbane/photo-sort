use anyhow::Result;
use console::style;
use std::fs;
use std::path::Path;

use crate::gallery::collect_photos;
use crate::metadata::Metadata;

/// Collect files matching the given tag and/or minimum rating filters.
pub fn filter_files(
    metadata: &Metadata,
    all_files: &[String],
    tag: Option<&str>,
    min_rating: Option<u8>,
) -> Vec<String> {
    all_files
        .iter()
        .filter(|f| {
            if let Some(t) = tag {
                if !metadata.get_tags(f).contains(&t.to_string()) {
                    return false;
                }
            }
            if let Some(min) = min_rating {
                if !metadata.get_rating(f).is_some_and(|r| r >= min) {
                    return false;
                }
            }
            true
        })
        .cloned()
        .collect()
}

pub fn run_export(
    dir: &Path,
    dest: &Path,
    tag: Option<&str>,
    min_rating: Option<u8>,
) -> Result<()> {
    if tag.is_none() && min_rating.is_none() {
        anyhow::bail!("Spécifiez au moins --tag ou --rating pour filtrer l'export");
    }

    let metadata = Metadata::load(dir)?;
    let photos = collect_photos(dir);
    let all_files: Vec<String> = photos.values().flatten().cloned().collect();

    let matched = filter_files(&metadata, &all_files, tag, min_rating);

    if matched.is_empty() {
        println!("  {} Aucun fichier ne correspond aux filtres.", style("!").yellow().bold());
        return Ok(());
    }

    fs::create_dir_all(dest)?;

    let mut copied = 0usize;
    for file in &matched {
        let src_path = dir.join(file);
        let filename = src_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        let mut dest_path = dest.join(&filename);
        // Handle collision
        if dest_path.exists() {
            let stem = dest_path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let ext = dest_path
                .extension()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let mut counter = 1u32;
            loop {
                dest_path = dest.join(format!("{stem}_{counter}.{ext}"));
                if !dest_path.exists() {
                    break;
                }
                counter += 1;
            }
        }

        fs::copy(&src_path, &dest_path)?;
        copied += 1;
    }

    println!(
        "  {} {} fichiers exportés vers {}",
        style("✔").green().bold(),
        style(copied).green().bold(),
        style(dest.display()).white().bold()
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    static TEST_COUNTER: AtomicU32 = AtomicU32::new(0);

    fn tmpdir() -> std::path::PathBuf {
        let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "photo_sort_export_test_{}_{id}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn setup_source(dir: &std::path::Path) {
        let y2020 = dir.join("2020");
        let y2021 = dir.join("2021");
        std::fs::create_dir_all(&y2020).unwrap();
        std::fs::create_dir_all(&y2021).unwrap();
        std::fs::write(y2020.join("a.jpg"), "photo a").unwrap();
        std::fs::write(y2020.join("b.jpg"), "photo b").unwrap();
        std::fs::write(y2021.join("c.jpg"), "photo c").unwrap();
    }

    // --- filter_files ---

    #[test]
    fn filter_by_tag() {
        let mut meta = Metadata::default();
        meta.add_tag("2020/a.jpg", "vacances");
        meta.add_tag("2020/b.jpg", "noel");

        let all = vec![
            "2020/a.jpg".to_string(),
            "2020/b.jpg".to_string(),
            "2021/c.jpg".to_string(),
        ];

        let result = filter_files(&meta, &all, Some("vacances"), None);
        assert_eq!(result, vec!["2020/a.jpg"]);
    }

    #[test]
    fn filter_by_rating() {
        let mut meta = Metadata::default();
        meta.set_rating("2020/a.jpg", Some(5));
        meta.set_rating("2020/b.jpg", Some(2));
        meta.set_rating("2021/c.jpg", Some(4));

        let all = vec![
            "2020/a.jpg".to_string(),
            "2020/b.jpg".to_string(),
            "2021/c.jpg".to_string(),
        ];

        let result = filter_files(&meta, &all, None, Some(4));
        assert_eq!(result, vec!["2020/a.jpg", "2021/c.jpg"]);
    }

    #[test]
    fn filter_by_tag_and_rating() {
        let mut meta = Metadata::default();
        meta.add_tag("2020/a.jpg", "vacances");
        meta.set_rating("2020/a.jpg", Some(5));
        meta.add_tag("2020/b.jpg", "vacances");
        meta.set_rating("2020/b.jpg", Some(2));
        meta.add_tag("2021/c.jpg", "noel");
        meta.set_rating("2021/c.jpg", Some(5));

        let all = vec![
            "2020/a.jpg".to_string(),
            "2020/b.jpg".to_string(),
            "2021/c.jpg".to_string(),
        ];

        let result = filter_files(&meta, &all, Some("vacances"), Some(4));
        assert_eq!(result, vec!["2020/a.jpg"]);
    }

    #[test]
    fn filter_no_match_returns_empty() {
        let meta = Metadata::default();
        let all = vec!["2020/a.jpg".to_string()];

        let result = filter_files(&meta, &all, Some("inexistant"), None);
        assert!(result.is_empty());
    }

    #[test]
    fn filter_no_filters_returns_all() {
        let meta = Metadata::default();
        let all = vec!["2020/a.jpg".to_string(), "2020/b.jpg".to_string()];

        let result = filter_files(&meta, &all, None, None);
        assert_eq!(result.len(), 2);
    }

    // --- run_export ---

    #[test]
    fn export_copies_matching_files() {
        let src = tmpdir();
        let dest = tmpdir();
        setup_source(&src);

        let mut meta = Metadata::default();
        meta.add_tag("2020/a.jpg", "vacances");
        meta.add_tag("2021/c.jpg", "vacances");
        meta.save(&src).unwrap();

        run_export(&src, &dest, Some("vacances"), None).unwrap();

        assert!(dest.join("a.jpg").exists());
        assert!(dest.join("c.jpg").exists());
        assert!(!dest.join("b.jpg").exists());

        let _ = std::fs::remove_dir_all(&src);
        let _ = std::fs::remove_dir_all(&dest);
    }

    #[test]
    fn export_handles_filename_collision() {
        let src = tmpdir();
        let dest = tmpdir();
        setup_source(&src);

        let mut meta = Metadata::default();
        meta.add_tag("2020/a.jpg", "x");
        meta.save(&src).unwrap();

        // Pre-create a.jpg in dest
        std::fs::write(dest.join("a.jpg"), "existing").unwrap();

        run_export(&src, &dest, Some("x"), None).unwrap();

        assert!(dest.join("a.jpg").exists());
        assert!(dest.join("a_1.jpg").exists());

        let _ = std::fs::remove_dir_all(&src);
        let _ = std::fs::remove_dir_all(&dest);
    }

    #[test]
    fn export_no_filter_errors() {
        let src = tmpdir();
        let dest = tmpdir();
        assert!(run_export(&src, &dest, None, None).is_err());
        let _ = std::fs::remove_dir_all(&src);
        let _ = std::fs::remove_dir_all(&dest);
    }

    #[test]
    fn export_by_rating() {
        let src = tmpdir();
        let dest = tmpdir();
        setup_source(&src);

        let mut meta = Metadata::default();
        meta.set_rating("2020/a.jpg", Some(5));
        meta.set_rating("2020/b.jpg", Some(1));
        meta.save(&src).unwrap();

        run_export(&src, &dest, None, Some(3)).unwrap();

        assert!(dest.join("a.jpg").exists());
        assert!(!dest.join("b.jpg").exists());
        assert!(!dest.join("c.jpg").exists());

        let _ = std::fs::remove_dir_all(&src);
        let _ = std::fs::remove_dir_all(&dest);
    }
}
