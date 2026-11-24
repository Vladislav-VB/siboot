//! The IPAs of your own siboot can install: those kept in ~/.siboot/library
//! (picked on the page from this iPhone's files, or downloaded from a link),
//! and those in the folders mounted in Minis (Settings › Mount External
//! Folders), which show up under /var/minis/mounts.
//!
//! A file picked on the page arrives at siboot over this iPhone's loopback
//! (web.rs) and is kept under its own name, made safe for a file name, so the
//! menu offers it again on the runs after: a free account's apps last 7 days,
//! and renewing one is installing it again.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::fail::{Fail, Result};

/// Where Minis shows the folders mounted in it. `SIBOOT_MOUNTS` stands in for
/// it in tests.
const MOUNTS: &str = "/var/minis/mounts";
/// How deep under a mounted folder to look, and how many of each kind of
/// file to offer: the menu is a list of buttons on a phone.
const DEPTH: usize = 2;
pub const MOST: usize = 8;
/// A name is cut to this many characters.
const LONGEST_NAME: usize = 80;

/// An IPA siboot could install.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub path: PathBuf,
    /// Its file name.
    pub name: String,
    pub size: u64,
    modified: SystemTime,
}

impl Entry {
    /// "Delta.ipa · 31.4 MB", or with the folder it's in for a mounted one.
    pub fn label(&self, place: Option<&str>) -> String {
        let size = format!("{:.1} MB", self.size as f64 / 1_048_576.0);
        match place {
            Some(place) => format!("{} · {size} · {place}", self.name),
            None => format!("{} · {size}", self.name),
        }
    }
}

/// ~/.siboot/library, made if it isn't there.
pub fn dir(home: &Path) -> Result<PathBuf> {
    let dir = home.join("library");
    std::fs::create_dir_all(&dir)
        .map_err(|e| Fail::new(format!("Couldn't create {}.", dir.display()), e.to_string()))?;
    Ok(dir)
}

/// The kept IPAs, the newest first.
pub fn kept(home: &Path) -> Vec<Entry> {
    let mut found = Vec::new();
    scan(&home.join("library"), 0, &mut found);
    newest_first(found)
}

/// The IPAs in the folders mounted in Minis, the newest first.
pub fn mounted() -> Vec<Entry> {
    mounted_in(&mounts_root())
}

fn mounts_root() -> PathBuf {
    std::env::var_os("SIBOOT_MOUNTS").map_or_else(|| PathBuf::from(MOUNTS), PathBuf::from)
}

fn mounted_in(root: &Path) -> Vec<Entry> {
    let mut found = Vec::new();
    scan(root, DEPTH, &mut found);
    newest_first(found)
}

/// The mounted folder `entry` is in: its first folder under the mounts.
pub fn place(entry: &Entry) -> Option<String> {
    let root = mounts_root();
    let relative = entry.path.strip_prefix(&root).ok()?;
    let first = relative.components().next()?;
    Some(first.as_os_str().to_string_lossy().into_owned())
}

fn newest_first(mut found: Vec<Entry>) -> Vec<Entry> {
    found.sort_by(|a, b| b.modified.cmp(&a.modified).then_with(|| a.name.cmp(&b.name)));
    found.truncate(MOST);
    found
}

/// Every `.ipa` in `dir`, and in its folders `depth` deep. Hidden files and
/// folders, and anything unreadable, are passed over.
fn scan(dir: &Path, depth: usize, found: &mut Vec<Entry>) {
    let Ok(read) = std::fs::read_dir(dir) else { return };
    for entry in read.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let path = entry.path();
        let Ok(meta) = std::fs::metadata(&path) else { continue };
        if meta.is_dir() {
            if depth > 0 {
                scan(&path, depth - 1, found);
            }
        } else if meta.is_file() && name.to_ascii_lowercase().ends_with(".ipa") {
            found.push(Entry {
                path,
                name,
                size: meta.len(),
                modified: meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            });
        }
    }
}

/// `raw` as a file name that can't climb out of the library or hide: letters,
/// digits and a few marks kept, anything else made `_`, ending in `.ipa`.
pub fn safe_name(raw: &str) -> String {
    // Only the last part of a path a browser might send.
    let raw = raw.rsplit(['/', '\\']).next().unwrap_or("");
    let mut name: String = raw
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || " -_.+()[]".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect();
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".ipa") {
        name.truncate(name.len() - 4);
    }
    let mut name: String = name.trim().trim_matches('.').trim().chars().take(LONGEST_NAME).collect();
    if name.is_empty() {
        name = "app".into();
    }
    format!("{name}.ipa")
}

/// The file name a download link gives, made safe, or None when the link
/// names no file.
pub fn name_from_url(url: &str) -> Option<String> {
    let parsed = reqwest::Url::parse(url).ok()?;
    let last = parsed.path_segments()?.rev().find(|s| !s.is_empty())?;
    let decoded = percent_decode(last);
    Some(safe_name(&decoded))
}

/// `%20` and the like, as a URL's path writes them.
pub fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("siboot-library-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn names_are_kept_to_the_library() {
        assert_eq!(safe_name("Delta.ipa"), "Delta.ipa");
        assert_eq!(safe_name("Delta 1.6 (b2).IPA"), "Delta 1.6 (b2).ipa");
        assert_eq!(safe_name("../../etc/passwd"), "passwd.ipa");
        assert_eq!(safe_name("C:\\Users\\me\\App.ipa"), "App.ipa");
        assert_eq!(safe_name(".hidden.ipa"), "hidden.ipa");
        assert_eq!(safe_name(""), "app.ipa");
        assert_eq!(safe_name("..."), "app.ipa");
        assert_eq!(safe_name("a\u{0}b;rm -rf$.ipa"), "a_b_rm -rf_.ipa");
        assert_eq!(safe_name("Приложение.ipa"), "Приложение.ipa");
        assert_eq!(safe_name(&"x".repeat(200)).len(), LONGEST_NAME + 4);
    }

    #[test]
    fn a_link_names_its_file() {
        assert_eq!(
            name_from_url("https://example.com/files/My%20App.ipa?dl=1").as_deref(),
            Some("My App.ipa")
        );
        assert_eq!(
            name_from_url("https://example.com/download/").as_deref(),
            Some("download.ipa")
        );
        assert_eq!(name_from_url("https://example.com").as_deref(), None);
        assert_eq!(name_from_url("not a url"), None);
        assert_eq!(percent_decode("a%2"), "a%2");
        assert_eq!(percent_decode("%41%zz"), "A%zz");
    }

    #[test]
    fn ipas_are_found_newest_first_and_not_too_deep() {
        let root = folder("mounts");
        std::fs::create_dir_all(root.join("Downloads/apps/deeper")).unwrap();
        std::fs::create_dir_all(root.join(".Trash")).unwrap();
        std::fs::write(root.join("Downloads/Old.ipa"), b"PK\x03\x04old").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(root.join("Downloads/apps/New.IPA"), b"PK\x03\x04new!").unwrap();
        std::fs::write(root.join("Downloads/apps/deeper/Deep.ipa"), b"PK").unwrap();
        std::fs::write(root.join("Downloads/notes.txt"), b"x").unwrap();
        std::fs::write(root.join(".Trash/Gone.ipa"), b"PK").unwrap();
        let found = mounted_in(&root);
        let names: Vec<&str> = found.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["New.IPA", "Old.ipa"]);
        assert_eq!(found[0].size, 8);
        assert_eq!(found[1].label(Some("Downloads")), "Old.ipa · 0.0 MB · Downloads");

        let home = folder("home");
        assert!(kept(&home).is_empty());
        let library = dir(&home).unwrap();
        for i in 0..MOST + 2 {
            std::fs::write(library.join(format!("App{i}.ipa")), b"PK").unwrap();
        }
        assert_eq!(kept(&home).len(), MOST);
    }
}
