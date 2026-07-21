//! Bundled game catalogue — loaded from a tar.zst archive compiled into the binary.
//!
//! Manifests are `missingno-gamedb` schema types (game → releases → artifacts),
//! flattened here into a platform-tagged view the UI can read without generics.
//! Provides identification (SHA1 → game + release) and search (title, release
//! titles, tags, source type) across the console trees the archive ships.

use std::collections::HashMap;

use missingno_gamedb::{
    Artifact, Game, GameBoy, GameBoyColor, GameKind, Link, Platform as DbPlatform, ReleaseStatus,
    Source, TvFormat, Vcs,
};

use crate::app::system::TvStandard;

/// The compressed gamedb archive, embedded at compile time.
static GAMEDB_ARCHIVE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/gamedb.tar.zst"));

const GBDEV_ENTRIES: &str = "https://raw.githubusercontent.com/gbdev/database/master/entries";

// ── Public types ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CataloguePlatform {
    GameBoy,
    GameBoyColor,
    Vcs,
}

/// A catalogue game, flattened from the schema types.
#[derive(Debug, Clone)]
pub struct CatalogueEntry {
    #[expect(dead_code)]
    pub platform: CataloguePlatform,
    pub slug: String,
    pub title: String,
    #[expect(dead_code)]
    pub kind: GameKind,
    pub developer: Option<String>,
    pub description: Option<String>,
    pub license: Option<String>,
    pub tags: Vec<String>,
    pub links: Vec<Link>,
    pub covers: Vec<String>,
    pub screenshots: Vec<String>,
    pub releases: Vec<CatalogueRelease>,
}

#[derive(Debug, Clone)]
pub struct CatalogueRelease {
    /// Title this release was published under, when it differs from the game's.
    pub title: Option<String>,
    #[expect(dead_code)]
    pub label: Option<String>,
    pub date: Option<String>,
    pub publisher: Option<String>,
    #[expect(dead_code)]
    pub status: ReleaseStatus,
    /// Broadcast standard (VCS): carts have no region header, so the DB is
    /// authoritative and the core only heuristically probes without it.
    pub tv_format: Option<TvStandard>,
    /// Cartridge board code (VCS), e.g. "F8", "F6SC" — resolves the bank
    /// scheme the size heuristic can't tell apart.
    pub cart_type: Option<String>,
    pub sources: Vec<Source>,
    pub artifacts: Vec<Artifact>,
}

impl CatalogueEntry {
    /// Whether this game can be obtained from a listed source.
    pub fn is_homebrew(&self) -> bool {
        self.releases.iter().any(|r| !r.sources.is_empty())
    }

    /// Release date of the first release (homebrew games have exactly one).
    pub fn primary_date(&self) -> Option<&str> {
        self.releases.first().and_then(|r| r.date.as_deref())
    }

    /// The Homebrew Hub source, if any release has one.
    pub fn homebrew_source(&self) -> Option<&Source> {
        self.releases
            .iter()
            .flat_map(|r| &r.sources)
            .find(|s| matches!(s, Source::HomebrewHub { .. }))
    }

    /// Cover image URL: explicit cover, else first screenshot.
    pub fn download_cover_url(&self) -> Option<String> {
        self.covers
            .first()
            .or_else(|| self.screenshots.first())
            .cloned()
    }

    /// Direct ROM download URL, in source preference order.
    pub fn download_url(&self) -> Option<String> {
        self.releases
            .iter()
            .flat_map(|r| &r.sources)
            .find_map(|source| match source {
                Source::HomebrewHub { slug, filename } => {
                    Some(format!("{GBDEV_ENTRIES}/{slug}/{filename}"))
                }
                Source::Download { url } => Some(url.clone()),
                Source::Itch { .. } | Source::SteamBundled { .. } => None,
            })
    }
}

// ── Flattening ────────────────────────────────────────────────────────

fn tv_standard(format: TvFormat) -> TvStandard {
    match format {
        TvFormat::Ntsc => TvStandard::Ntsc,
        TvFormat::Pal => TvStandard::Pal,
        // PAL-M is System M — 525 lines at 59.94 Hz — so the machine runs on
        // NTSC timing; only the colour encoding is PAL's, and on the VCS that
        // came from a board outside the TIA.
        TvFormat::PalM => TvStandard::Ntsc,
        TvFormat::Secam => TvStandard::Secam,
    }
}

fn entry_from<P: DbPlatform>(
    platform: CataloguePlatform,
    slug: String,
    game: Game<P>,
    hardware: impl Fn(&P::ReleaseHardware) -> (Option<TvStandard>, Option<String>),
) -> CatalogueEntry {
    CatalogueEntry {
        platform,
        slug,
        title: game.title,
        kind: game.kind,
        developer: game.developer,
        description: game.description,
        license: game.license,
        tags: game.tags,
        links: game.links,
        covers: game.covers,
        screenshots: game.screenshots,
        releases: game
            .releases
            .into_iter()
            .map(|release| {
                let (tv_format, cart_type) = hardware(&release.hardware);
                CatalogueRelease {
                    title: release.title,
                    label: release.label,
                    date: release.date.map(|d| d.as_str().to_owned()),
                    publisher: release.publisher,
                    status: release.status,
                    tv_format,
                    cart_type,
                    sources: release.sources,
                    artifacts: release.artifacts,
                }
            })
            .collect(),
    }
}

fn parse_entry(console: &str, slug: String, text: &str) -> Option<CatalogueEntry> {
    match console {
        "gb" => Game::<GameBoy>::from_ron(text)
            .ok()
            .map(|g| entry_from(CataloguePlatform::GameBoy, slug, g, |_| (None, None))),
        "gbc" => Game::<GameBoyColor>::from_ron(text)
            .ok()
            .map(|g| entry_from(CataloguePlatform::GameBoyColor, slug, g, |_| (None, None))),
        "vcs" => Game::<Vcs>::from_ron(text).ok().map(|g| {
            entry_from(CataloguePlatform::Vcs, slug, g, |hw| {
                (hw.tv_format.map(tv_standard), hw.cart_type.clone())
            })
        }),
        _ => None,
    }
}

// ── Catalogue ─────────────────────────────────────────────────────────

/// The loaded game catalogue. Built once at startup from the embedded archive.
pub struct Catalogue {
    /// All entries, sorted by title.
    entries: Vec<CatalogueEntry>,
    /// SHA1 hash → (entry, release) indices.
    hash_index: HashMap<String, (usize, usize)>,
}

impl Catalogue {
    /// Load the catalogue from the embedded archive. Call once at startup.
    pub fn load() -> Self {
        let mut entries = Vec::new();

        let empty = Self {
            entries: Vec::new(),
            hash_index: HashMap::new(),
        };
        let Ok(tar_data) = zstd::decode_all(GAMEDB_ARCHIVE) else {
            return empty;
        };
        let mut archive = tar::Archive::new(tar_data.as_slice());
        let Ok(tar_entries) = archive.entries() else {
            return empty;
        };

        for entry in tar_entries.flatten() {
            let Ok(path) = entry.path().map(|p| p.to_path_buf()) else {
                continue;
            };
            if path
                .file_name()
                .map(|f| f != "manifest.ron")
                .unwrap_or(true)
            {
                continue;
            }
            let (Some(console), Some(slug)) = (
                path.iter().next().map(|c| c.to_string_lossy().to_string()),
                path.parent()
                    .and_then(|p| p.file_name())
                    .map(|s| s.to_string_lossy().to_string()),
            ) else {
                continue;
            };
            let content = {
                use std::io::Read;
                let mut s = String::new();
                let mut entry = entry;
                if entry.read_to_string(&mut s).is_err() {
                    continue;
                }
                s
            };
            if let Some(parsed) = parse_entry(&console, slug, &content) {
                entries.push(parsed);
            }
        }

        entries.sort_by_key(|e| e.title.to_lowercase());

        let mut hash_index = HashMap::new();
        for (i, entry) in entries.iter().enumerate() {
            for (r, release) in entry.releases.iter().enumerate() {
                for artifact in &release.artifacts {
                    hash_index.insert(artifact.sha1.as_str().to_owned(), (i, r));
                }
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

    /// Look up a ROM by SHA1 hash: the game and the release the dump belongs to.
    pub fn lookup_hash(&self, sha1: &str) -> Option<(&CatalogueEntry, &CatalogueRelease)> {
        let sha1_lower = sha1.to_lowercase();
        self.hash_index.get(&sha1_lower).map(|&(i, r)| {
            let entry = &self.entries[i];
            (entry, &entry.releases[r])
        })
    }

    /// Get all homebrew entries, sorted by year (newest first).
    pub fn homebrew(&self) -> Vec<&CatalogueEntry> {
        let mut results: Vec<_> = self.entries.iter().filter(|e| e.is_homebrew()).collect();
        results.sort_by(|a, b| {
            b.primary_date()
                .unwrap_or("")
                .cmp(a.primary_date().unwrap_or(""))
        });
        results
    }

    /// Search homebrew by title substring. Results sorted by year (newest first).
    pub fn search_homebrew(&self, query: &str) -> Vec<&CatalogueEntry> {
        let query_lower = query.to_lowercase();
        let matches = |text: &str| text.to_lowercase().contains(&query_lower);
        let mut results: Vec<_> = self
            .entries
            .iter()
            .filter(|e| {
                e.is_homebrew()
                    && (matches(&e.title)
                        || e.releases
                            .iter()
                            .any(|r| r.title.as_deref().is_some_and(matches))
                        || e.developer.as_deref().is_some_and(matches)
                        || e.releases
                            .iter()
                            .any(|r| r.publisher.as_deref().is_some_and(matches))
                        || e.description.as_deref().is_some_and(matches)
                        || e.tags.iter().any(|t| matches(t)))
            })
            .collect();
        results.sort_by(|a, b| {
            b.primary_date()
                .unwrap_or("")
                .cmp(a.primary_date().unwrap_or(""))
        });
        results
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The embedded archive stores manifests as {console}/{slug}/manifest.ron;
    // load() must index every release's artifacts and carry the VCS hardware
    // facts through to hash lookup.
    #[test]
    fn embedded_catalogue_resolves_vcs_hardware_per_release() {
        let catalogue = Catalogue::load();
        if catalogue.entries.is_empty() {
            return; // submodule not checked out
        }
        // Pitfall II is one game whose NTSC and PAL cartridges are separate
        // releases; each hash must resolve to its own release.
        let (usa_game, usa) = catalogue
            .lookup_hash("920cfbd517764ad3fa6a7425c031bd72dc7d927c")
            .expect("USA Pitfall II resolves");
        let (pal_game, pal) = catalogue
            .lookup_hash("3ee18a1be7155900c2a01a104563657254d3a9a9")
            .expect("PAL Pitfall II resolves");
        assert_eq!(usa_game.title, pal_game.title);
        assert_eq!(usa.tv_format, Some(TvStandard::Ntsc));
        assert_eq!(pal.tv_format, Some(TvStandard::Pal));
        assert_eq!(usa.cart_type.as_deref(), Some("DPC"));
    }

    // Catalogue::load() silently drops manifests that fail to deserialize, so
    // parse the gamedb source tree directly to surface any bad files.
    #[test]
    fn all_gamedb_manifests_parse() {
        let data =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../missingno-gamedb/data");
        if !data.join("gb").is_dir() {
            return;
        }
        let (db, issues) = missingno_gamedb::Database::load(&data).unwrap();
        assert!(
            issues.is_empty(),
            "{} manifests failed to load; first: {:?}",
            issues.len(),
            issues.first()
        );
        assert!(!db.gb.games.is_empty());
        assert!(!db.gbc.games.is_empty());
        assert!(!db.vcs.games.is_empty());
    }
}
