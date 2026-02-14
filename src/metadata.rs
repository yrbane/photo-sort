use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

const METADATA_FILE: &str = ".photo_sort_metadata.json";

#[derive(Serialize, Deserialize, Clone, Default, Debug, PartialEq)]
pub struct FileInfo {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rating: Option<u8>,
}

#[derive(Serialize, Deserialize, Default, Debug)]
pub struct Metadata {
    pub files: HashMap<String, FileInfo>,
}

impl Metadata {
    pub fn load(dir: &Path) -> Result<Self> {
        let path = dir.join(METADATA_FILE);
        if path.exists() {
            let data =
                std::fs::read_to_string(&path).context("Impossible de lire le fichier metadata")?;
            let meta: Metadata =
                serde_json::from_str(&data).context("Fichier metadata invalide")?;
            Ok(meta)
        } else {
            Ok(Metadata::default())
        }
    }

    pub fn save(&self, dir: &Path) -> Result<()> {
        let path = dir.join(METADATA_FILE);
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, json).context("Impossible de sauvegarder les metadata")?;
        Ok(())
    }

    pub fn add_tag(&mut self, file: &str, tag: &str) {
        let info = self.files.entry(file.to_string()).or_default();
        if !info.tags.contains(&tag.to_string()) {
            info.tags.push(tag.to_string());
        }
    }

    pub fn remove_tag(&mut self, file: &str, tag: &str) {
        if let Some(info) = self.files.get_mut(file) {
            info.tags.retain(|t| t != tag);
        }
    }

    pub fn set_rating(&mut self, file: &str, rating: Option<u8>) {
        let info = self.files.entry(file.to_string()).or_default();
        info.rating = rating;
    }

    pub fn get_tags(&self, file: &str) -> &[String] {
        self.files.get(file).map(|i| i.tags.as_slice()).unwrap_or(&[])
    }

    pub fn get_rating(&self, file: &str) -> Option<u8> {
        self.files.get(file).and_then(|i| i.rating)
    }

    #[allow(dead_code)]
    pub fn files_with_tag(&self, tag: &str) -> Vec<String> {
        self.files
            .iter()
            .filter(|(_, info)| info.tags.contains(&tag.to_string()))
            .map(|(path, _)| path.clone())
            .collect()
    }

    #[allow(dead_code)]
    pub fn files_with_min_rating(&self, min: u8) -> Vec<String> {
        self.files
            .iter()
            .filter(|(_, info)| info.rating.is_some_and(|r| r >= min))
            .map(|(path, _)| path.clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    static TEST_COUNTER: AtomicU32 = AtomicU32::new(0);

    fn tmpdir() -> std::path::PathBuf {
        let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "photo_sort_meta_test_{}_{id}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    // --- Tags ---

    #[test]
    fn add_tag_to_new_file() {
        let mut meta = Metadata::default();
        meta.add_tag("2020/photo.jpg", "vacances");
        assert_eq!(meta.get_tags("2020/photo.jpg"), &["vacances"]);
    }

    #[test]
    fn add_multiple_tags() {
        let mut meta = Metadata::default();
        meta.add_tag("2020/photo.jpg", "vacances");
        meta.add_tag("2020/photo.jpg", "plage");
        assert_eq!(meta.get_tags("2020/photo.jpg"), &["vacances", "plage"]);
    }

    #[test]
    fn add_duplicate_tag_is_noop() {
        let mut meta = Metadata::default();
        meta.add_tag("2020/photo.jpg", "vacances");
        meta.add_tag("2020/photo.jpg", "vacances");
        assert_eq!(meta.get_tags("2020/photo.jpg"), &["vacances"]);
    }

    #[test]
    fn remove_tag() {
        let mut meta = Metadata::default();
        meta.add_tag("2020/photo.jpg", "vacances");
        meta.add_tag("2020/photo.jpg", "plage");
        meta.remove_tag("2020/photo.jpg", "vacances");
        assert_eq!(meta.get_tags("2020/photo.jpg"), &["plage"]);
    }

    #[test]
    fn remove_nonexistent_tag_is_noop() {
        let mut meta = Metadata::default();
        meta.add_tag("2020/photo.jpg", "vacances");
        meta.remove_tag("2020/photo.jpg", "inexistant");
        assert_eq!(meta.get_tags("2020/photo.jpg"), &["vacances"]);
    }

    #[test]
    fn remove_tag_on_unknown_file_is_noop() {
        let mut meta = Metadata::default();
        meta.remove_tag("unknown.jpg", "tag");
        assert!(meta.get_tags("unknown.jpg").is_empty());
    }

    #[test]
    fn get_tags_unknown_file_returns_empty() {
        let meta = Metadata::default();
        assert!(meta.get_tags("nonexistent.jpg").is_empty());
    }

    // --- Ratings ---

    #[test]
    fn set_rating() {
        let mut meta = Metadata::default();
        meta.set_rating("2020/photo.jpg", Some(4));
        assert_eq!(meta.get_rating("2020/photo.jpg"), Some(4));
    }

    #[test]
    fn update_rating() {
        let mut meta = Metadata::default();
        meta.set_rating("2020/photo.jpg", Some(3));
        meta.set_rating("2020/photo.jpg", Some(5));
        assert_eq!(meta.get_rating("2020/photo.jpg"), Some(5));
    }

    #[test]
    fn clear_rating() {
        let mut meta = Metadata::default();
        meta.set_rating("2020/photo.jpg", Some(4));
        meta.set_rating("2020/photo.jpg", None);
        assert_eq!(meta.get_rating("2020/photo.jpg"), None);
    }

    #[test]
    fn get_rating_unknown_file_returns_none() {
        let meta = Metadata::default();
        assert_eq!(meta.get_rating("nonexistent.jpg"), None);
    }

    // --- Filters ---

    #[test]
    fn files_with_tag_returns_matching() {
        let mut meta = Metadata::default();
        meta.add_tag("a.jpg", "vacances");
        meta.add_tag("b.jpg", "vacances");
        meta.add_tag("c.jpg", "noel");

        let mut files = meta.files_with_tag("vacances");
        files.sort();
        assert_eq!(files, vec!["a.jpg", "b.jpg"]);
    }

    #[test]
    fn files_with_tag_returns_empty_for_unknown_tag() {
        let meta = Metadata::default();
        assert!(meta.files_with_tag("inexistant").is_empty());
    }

    #[test]
    fn files_with_min_rating_returns_matching() {
        let mut meta = Metadata::default();
        meta.set_rating("a.jpg", Some(3));
        meta.set_rating("b.jpg", Some(5));
        meta.set_rating("c.jpg", Some(1));
        meta.set_rating("d.jpg", None);

        let mut files = meta.files_with_min_rating(3);
        files.sort();
        assert_eq!(files, vec!["a.jpg", "b.jpg"]);
    }

    #[test]
    fn files_with_min_rating_excludes_unrated() {
        let mut meta = Metadata::default();
        meta.set_rating("a.jpg", None);
        assert!(meta.files_with_min_rating(1).is_empty());
    }

    // --- Persistence ---

    #[test]
    fn save_and_load_roundtrip() {
        let tmp = tmpdir();
        let mut meta = Metadata::default();
        meta.add_tag("2020/photo.jpg", "vacances");
        meta.add_tag("2020/photo.jpg", "plage");
        meta.set_rating("2020/photo.jpg", Some(4));
        meta.add_tag("2019/autre.jpg", "noel");

        meta.save(&tmp).unwrap();
        let loaded = Metadata::load(&tmp).unwrap();

        assert_eq!(loaded.get_tags("2020/photo.jpg"), &["vacances", "plage"]);
        assert_eq!(loaded.get_rating("2020/photo.jpg"), Some(4));
        assert_eq!(loaded.get_tags("2019/autre.jpg"), &["noel"]);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn load_missing_file_returns_empty() {
        let meta = Metadata::load(Path::new("/nonexistent")).unwrap();
        assert!(meta.files.is_empty());
    }

    #[test]
    fn load_invalid_json_errors() {
        let tmp = tmpdir();
        std::fs::write(tmp.join(".photo_sort_metadata.json"), "bad json").unwrap();
        assert!(Metadata::load(&tmp).is_err());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn tags_and_rating_on_same_file() {
        let mut meta = Metadata::default();
        meta.add_tag("photo.jpg", "famille");
        meta.set_rating("photo.jpg", Some(5));

        assert_eq!(meta.get_tags("photo.jpg"), &["famille"]);
        assert_eq!(meta.get_rating("photo.jpg"), Some(5));
    }
}
