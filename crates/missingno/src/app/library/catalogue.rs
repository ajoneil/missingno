//! Bundled game catalogue — loaded from a tar.zst archive compiled into the binary.
//!
//! Provides identification (SHA1 → game info) and search (title, tags, source
//! type) across the console trees the archive ships (Game Boy / Game Boy Color
//! from No-Intro + gbdev homebrew, Atari VCS from vcs_cart_db).

use std::collections::HashMap;

use serde::Deserialize;

/// The compressed gamedb archive, embedded at compile time.
static GAMEDB_ARCHIVE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/gamedb.tar.zst"));

// ── Public types ──────────────────────────────────────────────────────

/// A game manifest from the catalogue.
#[derive(Debug, Clone, Deserialize)]
pub struct GameManifest {
    pub title: String,
    /// Release date — "YYYY-MM-DD" for homebrew, or absent for commercial.
    #[serde(default)]
    pub date: Option<String>,
    #[serde(default, rename = "region")]
    pub _region: Option<String>,
    #[serde(default)]
    pub developer: Option<String>,
    #[serde(default)]
    pub publisher: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    /// Broadcast standard (VCS): carts have no region header, so the DB is
    /// authoritative and the core only heuristically probes without it.
    #[serde(default)]
    pub tv_format: Option<crate::app::system::TvStandard>,
    /// Cartridge board code (VCS), e.g. "F8", "F6SC" — resolves the bank
    /// scheme the size heuristic can't tell apart (F8 vs F8SC, 8 KB E0 etc.).
    #[serde(default)]
    pub cart_type: Option<String>,
    #[serde(default)]
    pub hashes: Vec<String>,
    #[serde(default)]
    pub source: Option<GameSource>,
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub screenshots: Vec<String>,
    #[serde(default)]
    pub links: Vec<GameLink>,
}

#[derive(Debug, Clone, Deserialize)]
pub enum GameSource {
    HomebrewHub { slug: String, filename: String },
    Url(String),
}

#[derive(Debug, Clone, Deserialize)]
pub struct GameLink {
    pub name: String,
    pub url: String,
    #[serde(rename = "link_type")]
    pub _link_type: LinkType,
}

#[derive(Debug, Clone, Deserialize)]
pub enum LinkType {
    Wiki,
    Manual,
    Source,
    Speedrun,
    UnusedContent,
    TechnicalReference,
    Guide,
    Community,
}

/// An entry in the catalogue with its slug.
#[derive(Debug, Clone)]
pub struct CatalogueEntry {
    pub slug: String,
    pub manifest: GameManifest,
}

impl CatalogueEntry {
    /// Whether this is a downloadable homebrew game.
    pub fn is_homebrew(&self) -> bool {
        self.manifest.source.is_some()
    }

    /// Cover image URL (for homebrew from gbdev). Uses "cover.png" if listed
    /// in screenshots, otherwise falls back to the first screenshot.
    pub fn download_cover_url(&self) -> Option<String> {
        let slug = match &self.manifest.source {
            Some(GameSource::HomebrewHub { slug, .. }) => slug,
            _ => return None,
        };
        let filename = if self.manifest.screenshots.iter().any(|s| s == "cover.png") {
            "cover.png"
        } else {
            self.manifest.screenshots.first().map(|s| s.as_str())?
        };
        Some(format!(
            "https://raw.githubusercontent.com/gbdev/database/master/entries/{slug}/{filename}"
        ))
    }

    /// Download URL for homebrew games.
    pub fn download_url(&self) -> Option<String> {
        match &self.manifest.source {
            Some(GameSource::HomebrewHub { slug, filename }) => Some(format!(
                "https://raw.githubusercontent.com/gbdev/database/master/entries/{slug}/{filename}"
            )),
            Some(GameSource::Url(url)) => Some(url.clone()),
            None => None,
        }
    }
}

// ── Catalogue ─────────────────────────────────────────────────────────

/// The loaded game catalogue. Built once at startup from the embedded archive.
pub struct Catalogue {
    /// All entries, sorted by title.
    entries: Vec<CatalogueEntry>,
    /// SHA1 hash → index into entries.
    hash_index: HashMap<String, usize>,
}

impl Catalogue {
    /// Load the catalogue from the embedded archive. Call once at startup.
    pub fn load() -> Self {
        let mut entries = Vec::new();

        // Decompress
        let tar_data = match zstd::decode_all(GAMEDB_ARCHIVE) {
            Ok(data) => data,
            Err(_) => {
                return Self {
                    entries: Vec::new(),
                    hash_index: HashMap::new(),
                };
            }
        };

        // Parse tar
        let mut archive = tar::Archive::new(tar_data.as_slice());
        let tar_entries = match archive.entries() {
            Ok(e) => e,
            Err(_) => {
                return Self {
                    entries: Vec::new(),
                    hash_index: HashMap::new(),
                };
            }
        };

        for entry in tar_entries.flatten() {
            let path = match entry.path() {
                Ok(p) => p.to_path_buf(),
                Err(_) => continue,
            };

            // We only care about manifest.ron files
            if path
                .file_name()
                .map(|f| f != "manifest.ron")
                .unwrap_or(true)
            {
                continue;
            }

            let slug = match path.parent().and_then(|p| p.file_name()) {
                Some(s) => s.to_string_lossy().to_string(),
                None => continue,
            };

            // Read the file content
            let content = {
                use std::io::Read;
                let mut s = String::new();
                let mut entry = entry;
                if entry.read_to_string(&mut s).is_err() {
                    continue;
                }
                s
            };

            // Deserialize
            if let Ok(manifest) = ron::from_str::<GameManifest>(&content) {
                entries.push(CatalogueEntry { slug, manifest });
            }
        }

        // Sort by title
        entries.sort_by(|a, b| {
            a.manifest
                .title
                .to_lowercase()
                .cmp(&b.manifest.title.to_lowercase())
        });

        // Build hash index
        let mut hash_index = HashMap::new();
        for (i, entry) in entries.iter().enumerate() {
            for hash in &entry.manifest.hashes {
                hash_index.insert(hash.clone(), i);
            }
        }

        Self {
            entries,
            hash_index,
        }
    }

    /// Look up a game by slug.
    pub fn lookup_slug(&self, slug: &str) -> Option<&CatalogueEntry> {
        self.entries.iter().find(|e| e.slug == slug)
    }

    /// Look up a game by ROM SHA1 hash.
    pub fn lookup_hash(&self, sha1: &str) -> Option<&CatalogueEntry> {
        let sha1_lower = sha1.to_lowercase();
        self.hash_index.get(&sha1_lower).map(|&i| &self.entries[i])
    }

    /// Get all homebrew entries, sorted by year (newest first).
    pub fn homebrew(&self) -> Vec<&CatalogueEntry> {
        let mut results: Vec<_> = self.entries.iter().filter(|e| e.is_homebrew()).collect();
        results.sort_by(|a, b| {
            b.manifest
                .date
                .as_deref()
                .unwrap_or("")
                .cmp(a.manifest.date.as_deref().unwrap_or(""))
        });
        results
    }

    /// Search homebrew by title substring. Results sorted by year (newest first).
    pub fn search_homebrew(&self, query: &str) -> Vec<&CatalogueEntry> {
        let query_lower = query.to_lowercase();
        let mut results: Vec<_> = self
            .entries
            .iter()
            .filter(|e| {
                if !e.is_homebrew() {
                    return false;
                }
                let m = &e.manifest;
                m.title.to_lowercase().contains(&query_lower)
                    || m.developer
                        .as_ref()
                        .is_some_and(|d| d.to_lowercase().contains(&query_lower))
                    || m.publisher
                        .as_ref()
                        .is_some_and(|p| p.to_lowercase().contains(&query_lower))
                    || m.description
                        .as_ref()
                        .is_some_and(|d| d.to_lowercase().contains(&query_lower))
                    || m.tags
                        .iter()
                        .any(|t| t.to_lowercase().contains(&query_lower))
            })
            .collect();
        results.sort_by(|a, b| {
            b.manifest
                .date
                .as_deref()
                .unwrap_or("")
                .cmp(a.manifest.date.as_deref().unwrap_or(""))
        });
        results
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The embedded archive stores manifests two levels deep as
    // {console}/{slug}/manifest.ron; load() must still index them and carry the
    // VCS-only fields through to lookup.
    #[test]
    fn embedded_catalogue_loads_vcs_fields() {
        let catalogue = Catalogue::load();
        if catalogue.entries.is_empty() {
            return; // submodule not checked out
        }
        // "1 Adventure 2 Many" — a VCS entry with NTSC / 4K in the db.
        let entry = catalogue
            .lookup_hash("f64aaa03dcdfafde7ddda70c7d0c0e7d2f8f4f70")
            .expect("known VCS sha1 resolves");
        assert_eq!(
            entry.manifest.tv_format,
            Some(crate::app::system::TvStandard::Ntsc)
        );
        assert_eq!(entry.manifest.cart_type.as_deref(), Some("4K"));
    }

    // A broadcast standard belongs to the cartridge, not the game: two dumps
    // can share a title and differ in region, and each must resolve to its own.
    #[test]
    fn same_title_cartridges_keep_their_own_region() {
        let catalogue = Catalogue::load();
        if catalogue.entries.is_empty() {
            return; // submodule not checked out
        }
        // Both are "Pitfall II - Lost Caverns"; only the part number and the
        // region tell them apart.
        let usa = catalogue
            .lookup_hash("920cfbd517764ad3fa6a7425c031bd72dc7d927c")
            .expect("USA Pitfall II resolves");
        let pal = catalogue
            .lookup_hash("3ee18a1be7155900c2a01a104563657254d3a9a9")
            .expect("PAL Pitfall II resolves");
        assert_eq!(usa.manifest.title, pal.manifest.title);
        assert_eq!(
            usa.manifest.tv_format,
            Some(crate::app::system::TvStandard::Ntsc)
        );
        assert_eq!(
            pal.manifest.tv_format,
            Some(crate::app::system::TvStandard::Pal)
        );
    }

    // Catalogue::load() silently drops manifests that fail to deserialize,
    // so parse the gamedb source tree directly to surface any bad files.
    #[test]
    fn all_gamedb_manifests_parse() {
        let gamedb =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../missingno-gamedb");
        if !gamedb.join("gb").is_dir() {
            return;
        }

        let mut checked = 0;
        let mut failures = Vec::new();
        for console in ["gb", "gbc", "vcs"] {
            let Ok(entries) = std::fs::read_dir(gamedb.join(console)) else {
                continue;
            };
            for dir in entries.flatten() {
                let manifest = dir.path().join("manifest.ron");
                if !manifest.is_file() {
                    continue;
                }
                let content = std::fs::read_to_string(&manifest).unwrap();
                if let Err(e) = ron::from_str::<GameManifest>(&content) {
                    failures.push(format!("{}: {e}", manifest.display()));
                }
                checked += 1;
            }
        }

        assert!(checked > 0);
        assert!(
            failures.is_empty(),
            "{} manifests failed to parse:\n{}",
            failures.len(),
            failures.join("\n")
        );
    }
}
