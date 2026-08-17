use std::{fs, path::PathBuf};

use crate::app::library::{self, catalogue::Catalogue, hasheous, homebrew_hub};
use crate::app::system;

pub fn scan_directories(directories: &[PathBuf], catalogue: &Catalogue) -> Vec<library::GameEntry> {
    let mut new_entries = Vec::new();

    for dir in directories {
        let entries = match fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(_) => continue,
        };

        for dir_entry in entries.flatten() {
            let path = dir_entry.path();
            if !is_rom_file(&path) {
                continue;
            }

            let rom = match fs::read(&path) {
                Ok(rom) => rom,
                Err(_) => continue,
            };

            // Skip media no family claims — an unlaunchable library entry
            // helps nobody.
            let Some(family) = system::family_for(&path, &rom) else {
                continue;
            };

            let sha1 = hasheous::rom_sha1(&rom);

            // Check if already in library; older entries may predate
            // platform classification, so stamp it while the ROM is at hand.
            // What cartridge this is comes from the catalogue and is read there
            // at launch, so a rescan revises nothing about the media — and the
            // user's own launch overrides are theirs alone.
            if let Some((game_dir, mut existing)) = library::find_by_sha1(&sha1) {
                revise_on_rescan(&mut existing, family.platform, path);
                library::save_entry(&game_dir, &existing);
                continue;
            }

            let header_title = (family.title_from_rom)(&rom);

            // Try catalogue first for a good title, fall back to the header
            // title or the file stem.
            let mut entry = if let Some((game, release, _)) = catalogue.lookup_hash(&sha1) {
                let mut e = library::GameEntry::new(sha1, game.title.clone(), path.clone());
                e.platform = Some(family.platform);
                e.publisher = release.publisher.clone().or(game.developer.clone());
                e.year = release.date.clone();
                e.description = game.description.clone();
                e
            } else {
                let title = header_title
                    .clone()
                    .unwrap_or_else(|| crate::app::file_stem_title(&path));
                let mut e = library::GameEntry::new(sha1, title, path.clone());
                e.platform = Some(family.platform);
                e
            };
            entry.header_title = header_title;

            let game_dir = match library::game_dir_for(&entry.title, &entry.sha1) {
                Some(dir) => dir,
                None => continue,
            };

            // Import .sav from next to ROM if available
            let legacy_sav = path.with_extension("sav");
            if legacy_sav.exists() {
                library::activity::import_legacy_sav(&game_dir, &legacy_sav);
            }

            library::save_entry(&game_dir, &entry);
            new_entries.push(entry);
        }
    }

    new_entries
}

/// Result of enriching a single game.
#[derive(Debug, Clone)]
pub struct EnrichResult {
    /// SHA1 of the game that was enriched, if any.
    pub sha1: Option<String>,
    /// Whether there may be more games to enrich.
    pub has_more: bool,
    /// Whether visible data changed (title, cover, metadata).
    pub data_changed: bool,
}

/// What a rescan revises on an entry already in the library: only what the
/// media itself settles. What cartridge this is comes from the catalogue and is
/// read there at launch, and the user's own launch overrides are theirs alone.
fn revise_on_rescan(entry: &mut library::GameEntry, platform: system::Platform, path: PathBuf) {
    entry.platform.get_or_insert(platform);
    entry.add_rom_path(path);
}

/// What enriching one library entry over the network would do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Enrichment {
    /// A curated catalogue game with no cover yet: fetch the one it links.
    CatalogueCover,
    /// Ask Hasheous about a game the catalogue has not curated.
    HasheousLookup,
}

/// What the catalogue and the library already hold for one entry, as the
/// enrichment decision reads them.
pub(crate) struct EnrichmentState {
    /// A human has reviewed the catalogue's entry for this dump.
    pub curated: bool,
    /// The catalogue links a cover image for it.
    pub has_cover_url: bool,
    /// A cover is already saved beside the entry.
    pub has_cover: bool,
    /// Something has already been fetched for this entry.
    pub attempted: bool,
    /// The Hasheous metadata setting is on.
    pub hasheous_allowed: bool,
}

/// The one decision point for reaching the network about a library entry. A
/// curated catalogue game is already the better source for everything Hasheous
/// supplies, so Hasheous is never asked about one — its only fetch is the cover
/// the catalogue links.
pub(crate) fn enrichment_for(state: &EnrichmentState) -> Option<Enrichment> {
    if state.attempted {
        return None;
    }
    if state.curated {
        return (state.has_cover_url && !state.has_cover).then_some(Enrichment::CatalogueCover);
    }
    state.hasheous_allowed.then_some(Enrichment::HasheousLookup)
}

/// Enrich the next library entry that has something left to fetch.
pub fn enrich_next(catalogue: &Catalogue, hasheous_allowed: bool) -> EnrichResult {
    // Rate limit: sleep 1s before each request
    std::thread::sleep(std::time::Duration::from_secs(1));

    let next = library::list_all()
        .into_iter()
        .find_map(|(game_dir, entry)| {
            let state = EnrichmentState {
                curated: catalogue.curated(&entry.sha1),
                has_cover_url: catalogue.cover_url(&entry.sha1).is_some(),
                has_cover: library::load_cover(&game_dir).is_some(),
                attempted: entry.enrichment_attempted,
                hasheous_allowed,
            };
            enrichment_for(&state).map(|enrichment| (game_dir, entry, enrichment))
        });

    let Some((game_dir, mut entry, enrichment)) = next else {
        return EnrichResult {
            sha1: None,
            has_more: false,
            data_changed: false,
        };
    };

    let sha1 = entry.sha1.clone();

    if enrichment == Enrichment::CatalogueCover {
        // The catalogue's own cover is a curated entry's only network access.
        let bytes = catalogue.cover_url(&sha1).and_then(|url| {
            homebrew_hub::HomebrewHubClient::new()
                .download_image(&url)
                .ok()
        });
        if let Some(bytes) = &bytes {
            library::save_cover(&game_dir, bytes);
        }
        entry.enrichment_attempted = true;
        library::save_entry(&game_dir, &entry);
        return EnrichResult {
            sha1: Some(sha1),
            has_more: true,
            data_changed: bytes.is_some(),
        };
    }

    let mut info = match hasheous::lookup(&entry.sha1) {
        Ok(Some(info)) => info,
        Ok(None) => {
            entry.enrichment_attempted = true;
            library::save_entry(&game_dir, &entry);
            return EnrichResult {
                sha1: Some(sha1),
                has_more: true,
                data_changed: false,
            };
        }
        Err(_) => {
            return EnrichResult {
                sha1: None,
                has_more: false,
                data_changed: false,
            };
        }
    };

    let cover_art = info.cover_art.take();
    entry.apply_metadata(info);
    library::save_entry(&game_dir, &entry);

    if let Some(bytes) = &cover_art {
        library::save_cover(&game_dir, bytes);
    }

    EnrichResult {
        sha1: Some(sha1),
        has_more: true,
        data_changed: true,
    }
}

fn is_rom_file(path: &std::path::Path) -> bool {
    path.is_file()
        && path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|ext| {
                system::FAMILIES
                    .iter()
                    .any(|family| family.extensions.contains(&ext))
            })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> EnrichmentState {
        EnrichmentState {
            curated: false,
            has_cover_url: true,
            has_cover: false,
            attempted: false,
            hasheous_allowed: true,
        }
    }

    #[test]
    fn a_curated_game_is_never_looked_up() {
        let curated = EnrichmentState {
            curated: true,
            ..state()
        };
        assert_eq!(enrichment_for(&curated), Some(Enrichment::CatalogueCover));

        let with_cover = EnrichmentState {
            curated: true,
            has_cover: true,
            ..state()
        };
        assert_eq!(enrichment_for(&with_cover), None);

        let no_cover_linked = EnrichmentState {
            curated: true,
            has_cover_url: false,
            ..state()
        };
        assert_eq!(enrichment_for(&no_cover_linked), None);
    }

    #[test]
    fn a_rescan_leaves_the_users_own_launch_options_alone() {
        let mut entry =
            library::GameEntry::new("abc".into(), "T".into(), PathBuf::from("/roms/t.a26"));
        entry.overrides.set_choice("board", "F6");
        revise_on_rescan(
            &mut entry,
            system::Platform::AtariVcs,
            PathBuf::from("/roms/copy.a26"),
        );
        assert_eq!(entry.overrides.choice("board"), Some("F6"));
        assert_eq!(entry.platform, Some(system::Platform::AtariVcs));
        assert_eq!(entry.rom_paths.len(), 2);
    }

    #[test]
    fn an_uncurated_game_still_goes_to_hasheous() {
        assert_eq!(enrichment_for(&state()), Some(Enrichment::HasheousLookup));
    }

    #[test]
    fn nothing_is_fetched_twice() {
        let attempted = EnrichmentState {
            attempted: true,
            ..state()
        };
        assert_eq!(enrichment_for(&attempted), None);
        let attempted_curated = EnrichmentState {
            curated: true,
            attempted: true,
            ..state()
        };
        assert_eq!(enrichment_for(&attempted_curated), None);
    }

    #[test]
    fn hasheous_off_leaves_an_uncurated_game_alone() {
        let off = EnrichmentState {
            hasheous_allowed: false,
            ..state()
        };
        assert_eq!(enrichment_for(&off), None);
    }
}
