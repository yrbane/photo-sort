use anyhow::{Context, Result};
use chrono::NaiveDateTime;
use console::style;
use indicatif::{HumanBytes, ProgressBar, ProgressStyle};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{BufReader, Read as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use walkdir::WalkDir;

pub const PHOTO_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "heic", "heif", "cr2", "cr3", "nef", "arw", "dng", "orf", "rw2", "raf",
    "tiff", "tif",
];

#[derive(Serialize, Deserialize, Clone)]
pub struct ProcessedEntry {
    pub source: String,
    pub dest: String,
    pub size: u64,
    pub hash: String,
    pub date_source: String,
}

#[derive(Serialize, Deserialize, Default)]
pub struct Progress {
    pub processed: Vec<ProcessedEntry>,
}

#[derive(Debug)]
pub enum DateSource {
    Exif,
    Dirname,
    Filesystem,
}

impl DateSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            DateSource::Exif => "exif",
            DateSource::Dirname => "dirname",
            DateSource::Filesystem => "filesystem",
        }
    }
}

pub fn is_photo(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| PHOTO_EXTENSIONS.contains(&ext.to_lowercase().as_str()))
}

pub fn hash_file(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = blake3::Hasher::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

pub fn date_from_exif(path: &Path) -> Option<NaiveDateTime> {
    let file = fs::File::open(path).ok()?;
    let mut reader = BufReader::new(file);
    let exif = exif::Reader::new().read_from_container(&mut reader).ok()?;

    for tag in [
        exif::Tag::DateTimeOriginal,
        exif::Tag::DateTimeDigitized,
        exif::Tag::DateTime,
    ] {
        if let Some(field) = exif.get_field(tag, exif::In::PRIMARY) {
            let val = field.display_value().to_string();
            if let Ok(dt) = NaiveDateTime::parse_from_str(&val, "%Y-%m-%d %H:%M:%S") {
                return Some(dt);
            }
        }
    }
    None
}

pub fn date_from_dirname(path: &Path) -> Option<NaiveDateTime> {
    let re = Regex::new(r"(19|20)\d{2}").unwrap();
    let path_str = path.to_string_lossy();
    let year: u32 = re
        .find_iter(&path_str)
        .last()?
        .as_str()
        .parse()
        .ok()?;
    NaiveDateTime::parse_from_str(&format!("{year}-01-01 00:00:00"), "%Y-%m-%d %H:%M:%S").ok()
}

pub fn date_from_filesystem(path: &Path) -> Option<NaiveDateTime> {
    let meta = fs::metadata(path).ok()?;
    let system_time = meta.created().or_else(|_| meta.modified()).ok()?;
    let dt: chrono::DateTime<chrono::Local> = system_time.into();
    Some(dt.naive_local())
}

pub fn detect_date(path: &Path) -> (NaiveDateTime, DateSource) {
    if let Some(dt) = date_from_exif(path) {
        return (dt, DateSource::Exif);
    }
    if let Some(dt) = date_from_dirname(path) {
        return (dt, DateSource::Dirname);
    }
    if let Some(dt) = date_from_filesystem(path) {
        return (dt, DateSource::Filesystem);
    }
    (
        NaiveDateTime::parse_from_str("1970-01-01 00:00:00", "%Y-%m-%d %H:%M:%S").unwrap(),
        DateSource::Filesystem,
    )
}

pub fn build_dest_path(output_dir: &Path, dt: &NaiveDateTime, ext: &str) -> PathBuf {
    let year = dt.format("%Y").to_string();
    let base_name = dt.format("%Y-%m-%d_%H-%M-%S").to_string();
    let year_dir = output_dir.join(&year);

    let candidate = year_dir.join(format!("{base_name}.{ext}"));
    if !candidate.exists() {
        return candidate;
    }

    let mut counter = 1u32;
    loop {
        let candidate = year_dir.join(format!("{base_name}_{counter}.{ext}"));
        if !candidate.exists() {
            return candidate;
        }
        counter += 1;
    }
}

pub fn load_progress(path: &Path) -> Result<Progress> {
    if path.exists() {
        let data =
            fs::read_to_string(path).context("Impossible de lire le fichier de progression")?;
        let progress: Progress =
            serde_json::from_str(&data).context("Fichier de progression invalide")?;
        Ok(progress)
    } else {
        Ok(Progress::default())
    }
}

pub fn save_progress(path: &Path, progress: &Progress) -> Result<()> {
    let json = serde_json::to_string_pretty(progress)?;
    fs::write(path, json).context("Impossible de sauvegarder la progression")?;
    Ok(())
}

fn append_origin(year_dir: &Path, new_name: &str, original_path: &str) -> Result<()> {
    use std::io::Write;
    let origins_path = year_dir.join(".photo_sort_origins");
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&origins_path)?;
    writeln!(file, "{new_name} <- {original_path}")?;
    Ok(())
}

pub fn run_sort(source: &Path, output_dir: &Path) -> Result<()> {
    fs::create_dir_all(output_dir)?;

    let progress_path = output_dir.join(".photo_sort_progress.json");
    let mut progress = load_progress(&progress_path)?;

    let mut processed_index: HashMap<String, u64> = HashMap::new();
    let mut known_hashes: HashSet<String> = HashSet::new();
    for entry in &progress.processed {
        processed_index.insert(entry.source.clone(), entry.size);
        known_hashes.insert(entry.hash.clone());
    }

    let interrupted = Arc::new(AtomicBool::new(false));
    let interrupted_clone = Arc::clone(&interrupted);
    ctrlc::set_handler(move || {
        interrupted_clone.store(true, Ordering::SeqCst);
    })?;

    let scan_spinner = ProgressBar::new_spinner();
    scan_spinner.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.cyan} {msg}")
            .unwrap(),
    );
    scan_spinner.set_message("Analyse du dossier source…");

    let mut photos: Vec<PathBuf> = Vec::new();
    let mut source_dirs: HashSet<PathBuf> = HashSet::new();
    let mut total_size: u64 = 0;

    for entry in WalkDir::new(source)
        .into_iter()
        .filter_entry(|e| !e.file_type().is_dir() || e.file_name() != ".thumbnails")
        .filter_map(|e| e.ok())
    {
        if entry.file_type().is_dir() {
            source_dirs.insert(entry.into_path());
        } else if entry.file_type().is_file() && is_photo(entry.path()) {
            total_size += entry.metadata().map(|m| m.len()).unwrap_or(0);
            photos.push(entry.into_path());
        }
        scan_spinner.tick();
    }

    scan_spinner.finish_and_clear();

    let total = photos.len();
    let dir_count = source_dirs.len();

    println!(
        "\n{}  {}\n",
        style("photo-sort").bold().cyan(),
        style("·").dim(),
    );
    println!(
        "  {}  {}",
        style("Source").dim(),
        style(source.display()).white().bold()
    );
    println!(
        "  {}  {}",
        style("Sortie").dim(),
        style(output_dir.display()).white().bold()
    );
    println!(
        "  {} {}  {}  {} {}",
        style("Dossiers").dim(),
        style(dir_count).yellow().bold(),
        style("·").dim(),
        style("Photos").dim(),
        style(total).green().bold(),
    );
    println!(
        "  {}  {}",
        style("Taille").dim(),
        style(HumanBytes(total_size)).white()
    );
    if !processed_index.is_empty() {
        println!(
            "  {}  {} fichiers déjà traités",
            style("Reprise").dim(),
            style(processed_index.len()).cyan().bold()
        );
    }
    println!();

    if total == 0 {
        println!("  {} Aucune photo trouvée.", style("!").yellow().bold());
        return Ok(());
    }

    let pb = ProgressBar::new(total as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template(
                "  {bar:40.green/dark_gray} {pos}/{len}  {percent}%  \
                 {msg}\n  \
                 {elapsed_precise} écoulé  ·  ETA {eta_precise}  ·  {per_sec}",
            )
            .unwrap()
            .progress_chars("━╸─"),
    );

    let mut copied = 0usize;
    let mut skipped = 0usize;
    let mut duplicates = 0usize;
    let mut by_method: HashMap<&str, usize> = HashMap::new();
    let mut years_created: HashSet<String> = HashSet::new();

    for photo_path in &photos {
        if interrupted.load(Ordering::SeqCst) {
            pb.abandon_with_message(
                style("Interruption — progression sauvegardée")
                    .yellow()
                    .to_string(),
            );
            save_progress(&progress_path, &progress)?;
            std::process::exit(0);
        }

        let abs_source = photo_path
            .canonicalize()
            .unwrap_or_else(|_| photo_path.clone());
        let source_str = abs_source.to_string_lossy().to_string();

        let file_size = fs::metadata(&abs_source).map(|m| m.len()).unwrap_or(0);

        if let Some(&prev_size) = processed_index.get(&source_str) {
            if prev_size == file_size {
                skipped += 1;
                pb.set_message(format!(
                    "{} {}",
                    style("skip").dim(),
                    style(
                        abs_source
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                    )
                    .dim()
                ));
                pb.inc(1);
                continue;
            }
        }

        let filename = abs_source
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        let file_hash = hash_file(&abs_source).unwrap_or_default();
        if known_hashes.contains(&file_hash) {
            duplicates += 1;
            pb.set_message(format!(
                "{} {}",
                style("dupe").magenta(),
                style(&filename).dim()
            ));
            pb.inc(1);
            continue;
        }

        let (dt, date_source) = detect_date(&abs_source);

        let ext = abs_source
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("jpg")
            .to_lowercase();

        let dest_path = build_dest_path(output_dir, &dt, &ext);

        if let Some(parent) = dest_path.parent() {
            fs::create_dir_all(parent)?;
        }

        pb.set_message(format!(
            "{} {}",
            style(date_source.as_str()).cyan(),
            style(&filename).white()
        ));

        fs::copy(&abs_source, &dest_path).with_context(|| {
            format!(
                "Erreur de copie : {} → {}",
                abs_source.display(),
                dest_path.display()
            )
        })?;

        copied += 1;
        *by_method.entry(date_source.as_str()).or_insert(0) += 1;

        let year = dt.format("%Y").to_string();
        years_created.insert(year);

        let dest_relative = dest_path
            .strip_prefix(output_dir)
            .unwrap_or(&dest_path)
            .to_string_lossy()
            .to_string();

        let dest_filename = dest_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        if let Some(year_dir) = dest_path.parent() {
            if let Err(e) = append_origin(year_dir, &dest_filename, &source_str) {
                pb.suspend(|| {
                    eprintln!("  {} origins : {e}", style("!").yellow().bold());
                });
            }
        }

        let entry = ProcessedEntry {
            source: source_str.clone(),
            dest: dest_relative,
            size: file_size,
            hash: file_hash.clone(),
            date_source: date_source.as_str().to_string(),
        };

        progress.processed.push(entry);
        processed_index.insert(source_str, file_size);
        known_hashes.insert(file_hash);

        save_progress(&progress_path, &progress)?;
        pb.inc(1);
    }

    pb.finish_and_clear();

    println!();
    println!("  {} Terminé !", style("✔").green().bold());
    println!();
    println!(
        "  {}  {}",
        style("Copiées").dim(),
        style(copied).green().bold()
    );
    if skipped > 0 {
        println!(
            "  {}  {} (déjà traitées)",
            style("Ignorées").dim(),
            style(skipped).yellow().bold()
        );
    }
    if duplicates > 0 {
        println!(
            "  {}  {} (même contenu)",
            style("Doublons").dim(),
            style(duplicates).magenta().bold()
        );
    }

    if !by_method.is_empty() {
        let parts: Vec<String> = ["exif", "dirname", "filesystem"]
            .iter()
            .filter_map(|m| by_method.get(m).map(|c| format!("{m} {c}")))
            .collect();
        println!(
            "  {}  {}",
            style("Méthode").dim(),
            style(parts.join("  ·  ")).white()
        );
    }

    if !years_created.is_empty() {
        let mut years: Vec<&String> = years_created.iter().collect();
        years.sort();
        println!(
            "  {}  {}",
            style("Années").dim(),
            style(
                years
                    .iter()
                    .map(|y| y.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
            .cyan()
        );
    }
    println!();

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU32;

    static TEST_COUNTER: AtomicU32 = AtomicU32::new(0);

    pub fn tmpdir() -> PathBuf {
        let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "photo_sort_test_{}_{id}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn parse_dt(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").unwrap()
    }

    #[test]
    fn is_photo_recognizes_supported_extensions() {
        for ext in PHOTO_EXTENSIONS {
            assert!(
                is_photo(Path::new(&format!("photo.{ext}"))),
                "devrait reconnaître .{ext}"
            );
        }
    }

    #[test]
    fn is_photo_case_insensitive() {
        assert!(is_photo(Path::new("photo.JPG")));
        assert!(is_photo(Path::new("photo.Cr2")));
        assert!(is_photo(Path::new("photo.HEIC")));
    }

    #[test]
    fn is_photo_rejects_non_photo() {
        assert!(!is_photo(Path::new("document.pdf")));
        assert!(!is_photo(Path::new("video.mp4")));
        assert!(!is_photo(Path::new("readme.txt")));
        assert!(!is_photo(Path::new("no_extension")));
    }

    #[test]
    fn dirname_extracts_year() {
        let dt = date_from_dirname(Path::new("/photos/vacances 2008/DCIM/IMG_001.jpg"));
        assert_eq!(dt.unwrap(), parse_dt("2008-01-01 00:00:00"));
    }

    #[test]
    fn dirname_takes_last_year_match() {
        let dt = date_from_dirname(Path::new("/photos/2005/sous-dossier 2010/photo.jpg"));
        assert_eq!(dt.unwrap(), parse_dt("2010-01-01 00:00:00"));
    }

    #[test]
    fn dirname_no_year_returns_none() {
        assert!(date_from_dirname(Path::new("/photos/vacances/DCIM/IMG.jpg")).is_none());
    }

    #[test]
    fn dirname_rejects_out_of_range_years() {
        assert!(date_from_dirname(Path::new("/photos/1899/photo.jpg")).is_none());
        assert!(date_from_dirname(Path::new("/photos/2100/photo.jpg")).is_none());
    }

    #[test]
    fn dirname_boundary_years() {
        assert_eq!(
            date_from_dirname(Path::new("/1900/photo.jpg")).unwrap(),
            parse_dt("1900-01-01 00:00:00")
        );
        assert_eq!(
            date_from_dirname(Path::new("/2099/photo.jpg")).unwrap(),
            parse_dt("2099-01-01 00:00:00")
        );
    }

    #[test]
    fn dest_path_basic_format() {
        let tmp = tmpdir();
        let result = build_dest_path(&tmp, &parse_dt("2008-07-15 14:30:22"), "jpg");
        assert_eq!(result, tmp.join("2008/2008-07-15_14-30-22.jpg"));
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn dest_path_collision_increments() {
        let tmp = tmpdir();
        let year_dir = tmp.join("2020");
        fs::create_dir_all(&year_dir).unwrap();

        let date = parse_dt("2020-03-10 09:00:00");

        fs::write(year_dir.join("2020-03-10_09-00-00.jpg"), "a").unwrap();
        let result = build_dest_path(&tmp, &date, "jpg");
        assert_eq!(result, tmp.join("2020/2020-03-10_09-00-00_1.jpg"));

        fs::write(year_dir.join("2020-03-10_09-00-00_1.jpg"), "b").unwrap();
        let result = build_dest_path(&tmp, &date, "jpg");
        assert_eq!(result, tmp.join("2020/2020-03-10_09-00-00_2.jpg"));

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn progress_load_missing_file_returns_empty() {
        let progress = load_progress(Path::new("/nonexistent/path.json")).unwrap();
        assert!(progress.processed.is_empty());
    }

    #[test]
    fn progress_roundtrip() {
        let tmp = tmpdir();
        let path = tmp.join("progress.json");

        let progress = Progress {
            processed: vec![
                ProcessedEntry {
                    source: "/photos/img.jpg".to_string(),
                    dest: "2020/2020-01-01_00-00-00.jpg".to_string(),
                    size: 12345,
                    hash: "abc123".to_string(),
                    date_source: "exif".to_string(),
                },
                ProcessedEntry {
                    source: "/photos/img2.cr2".to_string(),
                    dest: "2019/2019-06-15_10-30-00.cr2".to_string(),
                    size: 67890,
                    hash: "def456".to_string(),
                    date_source: "dirname".to_string(),
                },
            ],
        };

        save_progress(&path, &progress).unwrap();
        let loaded = load_progress(&path).unwrap();

        assert_eq!(loaded.processed.len(), 2);
        assert_eq!(loaded.processed[0].source, "/photos/img.jpg");
        assert_eq!(loaded.processed[0].size, 12345);
        assert_eq!(loaded.processed[1].date_source, "dirname");

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn progress_load_invalid_json_errors() {
        let tmp = tmpdir();
        let path = tmp.join("bad.json");
        fs::write(&path, "not json at all").unwrap();
        assert!(load_progress(&path).is_err());
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn append_origin_creates_and_appends() {
        let tmp = tmpdir();
        append_origin(&tmp, "2020-01-01_00-00-00.jpg", "/photos/a.jpg").unwrap();
        append_origin(&tmp, "2020-01-01_00-00-00_1.jpg", "/photos/b.jpg").unwrap();

        let content = fs::read_to_string(tmp.join(".photo_sort_origins")).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], "2020-01-01_00-00-00.jpg <- /photos/a.jpg");
        assert_eq!(lines[1], "2020-01-01_00-00-00_1.jpg <- /photos/b.jpg");
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn filesystem_date_from_real_file() {
        let tmp = tmpdir();
        let file = tmp.join("test.jpg");
        fs::write(&file, "fake photo").unwrap();

        let dt = date_from_filesystem(&file);
        assert!(dt.is_some());
        let now = chrono::Local::now().naive_local();
        let diff = now - dt.unwrap();
        assert!(diff.num_seconds().abs() < 5);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn filesystem_date_nonexistent_returns_none() {
        assert!(date_from_filesystem(Path::new("/nonexistent/file.jpg")).is_none());
    }

    #[test]
    fn detect_date_uses_dirname_for_non_exif_file() {
        let tmp = tmpdir();
        let subdir = tmp.join("vacances 2015");
        fs::create_dir_all(&subdir).unwrap();
        let file = subdir.join("photo.jpg");
        fs::write(&file, "not a real jpeg").unwrap();

        let (date, source) = detect_date(&file);
        assert_eq!(date, parse_dt("2015-01-01 00:00:00"));
        assert_eq!(source.as_str(), "dirname");
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn detect_date_falls_back_to_filesystem() {
        let tmp = tmpdir();
        let file = tmp.join("photo.jpg");
        fs::write(&file, "not a real jpeg").unwrap();

        let (_, source) = detect_date(&file);
        assert_eq!(source.as_str(), "filesystem");
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn date_source_as_str() {
        assert_eq!(DateSource::Exif.as_str(), "exif");
        assert_eq!(DateSource::Dirname.as_str(), "dirname");
        assert_eq!(DateSource::Filesystem.as_str(), "filesystem");
    }

    #[test]
    fn exif_returns_none_for_non_image() {
        let tmp = tmpdir();
        let file = tmp.join("fake.jpg");
        fs::write(&file, "this is not a jpeg").unwrap();
        assert!(date_from_exif(&file).is_none());
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn exif_returns_none_for_missing_file() {
        assert!(date_from_exif(Path::new("/nonexistent/photo.jpg")).is_none());
    }

    #[test]
    fn hash_file_deterministic() {
        let tmp = tmpdir();
        let file = tmp.join("test.bin");
        fs::write(&file, "hello world").unwrap();

        let h1 = hash_file(&file).unwrap();
        let h2 = hash_file(&file).unwrap();
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 64);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn hash_file_differs_for_different_content() {
        let tmp = tmpdir();
        let f1 = tmp.join("a.bin");
        let f2 = tmp.join("b.bin");
        fs::write(&f1, "content A").unwrap();
        fs::write(&f2, "content B").unwrap();
        assert_ne!(hash_file(&f1).unwrap(), hash_file(&f2).unwrap());
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn hash_file_same_for_identical_content() {
        let tmp = tmpdir();
        let f1 = tmp.join("a.bin");
        let f2 = tmp.join("b.bin");
        fs::write(&f1, "identical").unwrap();
        fs::write(&f2, "identical").unwrap();
        assert_eq!(hash_file(&f1).unwrap(), hash_file(&f2).unwrap());
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn hash_file_missing_returns_error() {
        assert!(hash_file(Path::new("/nonexistent/file.bin")).is_err());
    }

    #[test]
    fn progress_roundtrip_includes_hash() {
        let tmp = tmpdir();
        let path = tmp.join("progress.json");

        let progress = Progress {
            processed: vec![ProcessedEntry {
                source: "/photos/img.jpg".to_string(),
                dest: "2020/2020-01-01_00-00-00.jpg".to_string(),
                size: 100,
                hash: "aabbcc".to_string(),
                date_source: "exif".to_string(),
            }],
        };

        save_progress(&path, &progress).unwrap();
        let loaded = load_progress(&path).unwrap();
        assert_eq!(loaded.processed[0].hash, "aabbcc");
        let _ = fs::remove_dir_all(&tmp);
    }
}
