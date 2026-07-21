//! The curator's view of the gamedb checkout: typed manifests behind a
//! platform-agnostic editing surface, plus flags and git state.

use std::{fs, io, path::PathBuf, process::Command};

use missingno_gamedb::{
    Date, FlagFile, Game, GameBoy, GameBoyColor, GameKind, Link, LinkType, ModCategory, ModOf,
    Platform, Release, Sha1, Tree, Vcs,
};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TreeId {
    Gb,
    Gbc,
    Vcs,
}

impl TreeId {
    pub const ALL: [TreeId; 3] = [TreeId::Gb, TreeId::Gbc, TreeId::Vcs];

    pub fn dir(self) -> &'static str {
        match self {
            TreeId::Gb => "gb",
            TreeId::Gbc => "gbc",
            TreeId::Vcs => "vcs",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            TreeId::Gb => "Game Boy",
            TreeId::Gbc => "Game Boy Color",
            TreeId::Vcs => "Atari VCS",
        }
    }
}

/// One manifest, kept in its platform's schema type.
pub enum AnyGame {
    Gb(Game<GameBoy>),
    Gbc(Game<GameBoyColor>),
    Vcs(Game<Vcs>),
}

macro_rules! common {
    ($self:expr, $game:ident => $body:expr) => {
        match $self {
            AnyGame::Gb($game) => $body,
            AnyGame::Gbc($game) => $body,
            AnyGame::Vcs($game) => $body,
        }
    };
}

/// The game-level fields every platform shares, editable as text.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TextField {
    Title,
    Developer,
    Description,
    License,
}

impl AnyGame {
    pub fn title(&self) -> &str {
        common!(self, g => &g.title)
    }

    pub fn kind(&self) -> GameKind {
        common!(self, g => g.kind)
    }

    pub fn curations(&self) -> &[missingno_gamedb::Curation] {
        common!(self, g => &g.curated)
    }

    /// Add or refresh one curator's endorsement.
    pub fn stamp_curation(&mut self, by: &str, recommended: bool) {
        let stamp = missingno_gamedb::Curation {
            by: by.to_owned(),
            date: Db::today(),
            recommended,
        };
        common!(self, g => {
            match g.curated.iter_mut().find(|c| c.by == stamp.by) {
                Some(existing) => *existing = stamp.clone(),
                None => g.curated.push(stamp.clone()),
            }
        });
    }

    /// An automated change un-vouches every curator.
    pub fn clear_curations(&mut self) {
        common!(self, g => g.curated.clear());
    }

    pub fn text_field(&self, field: TextField) -> String {
        common!(self, g => match field {
            TextField::Title => g.title.clone(),
            TextField::Developer => g.developer.clone().unwrap_or_default(),
            TextField::Description => g.description.clone().unwrap_or_default(),
            TextField::License => g.license.clone().unwrap_or_default(),
        })
    }

    pub fn set_text_field(&mut self, field: TextField, value: String) {
        let optional = (!value.is_empty()).then_some(value.clone());
        common!(self, g => match field {
            TextField::Title => g.title = value.clone(),
            TextField::Developer => g.developer = optional.clone(),
            TextField::Description => g.description = optional.clone(),
            TextField::License => g.license = optional.clone(),
        });
    }

    /// One display line per release.
    pub fn release_lines(&self) -> Vec<String> {
        fn line<P: Platform>(r: &missingno_gamedb::Release<P>, extra: &str) -> String {
            let mut parts = Vec::new();
            if let Some(title) = &r.title {
                parts.push(title.clone());
            }
            if !r.regions.is_empty() {
                parts.push(format!("{:?}", r.regions));
            }
            if let Some(label) = &r.label {
                parts.push(label.clone());
            }
            if r.status != Default::default() {
                parts.push(format!("{:?}", r.status));
            }
            if let Some(date) = &r.date {
                parts.push(date.as_str().to_owned());
            }
            if !extra.is_empty() {
                parts.push(extra.to_owned());
            }
            parts.push(format!(
                "{} artifact(s), {} source(s)",
                r.artifacts.len(),
                r.sources.len()
            ));
            parts.join(" · ")
        }
        match self {
            AnyGame::Gb(g) => g
                .releases
                .iter()
                .map(|r| {
                    line(
                        r,
                        &format!("sgb {:?} / cgb {:?}", r.hardware.sgb, r.hardware.cgb),
                    )
                })
                .collect(),
            AnyGame::Gbc(g) => g.releases.iter().map(|r| line(r, "")).collect(),
            AnyGame::Vcs(g) => g
                .releases
                .iter()
                .map(|r| {
                    let hw = [
                        r.hardware.tv_format.map(|t| format!("{t:?}")),
                        r.hardware.cart_type.clone(),
                    ]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join(" ");
                    line(r, &hw)
                })
                .collect(),
        }
    }

    /// First directly-downloadable source URL across releases.
    pub fn download_url(&self) -> Option<String> {
        const GBDEV: &str = "https://raw.githubusercontent.com/gbdev/database/master/entries";
        let sources = common!(self, g => g
            .releases
            .iter()
            .flat_map(|r| r.sources.clone())
            .collect::<Vec<_>>());
        sources.iter().find_map(|s| match s {
            missingno_gamedb::Source::HomebrewHub { slug, filename } => {
                Some(format!("{GBDEV}/{slug}/{filename}"))
            }
            missingno_gamedb::Source::Download { url } => Some(url.clone()),
            _ => None,
        })
    }

    pub fn artifact_sha1s(&self) -> Vec<String> {
        common!(self, g => g
            .releases
            .iter()
            .flat_map(|r| &r.artifacts)
            .map(|a| a.sha1.as_str().to_owned())
            .collect())
    }

    /// Record a newly verified dump on the first sourced (else first) release.
    /// Returns false when the hash was already present.
    pub fn stage_artifact(&mut self, sha1: &str, size: u64) -> bool {
        if self.artifact_sha1s().iter().any(|s| s == sha1) {
            return false;
        }
        let Ok(sha1) = sha1.parse::<missingno_gamedb::Sha1>() else {
            return false;
        };
        common!(self, g => {
            let at = g
                .releases
                .iter()
                .position(|r| !r.sources.is_empty())
                .unwrap_or(0);
            if let Some(release) = g.releases.get_mut(at) {
                release.artifacts.push(missingno_gamedb::Artifact {
                    sha1,
                    size: Some(size),
                });
                true
            } else {
                false
            }
        })
    }

    pub fn covers(&self) -> Vec<String> {
        common!(self, g => g.covers.clone())
    }

    pub fn set_covers(&mut self, covers: Vec<String>) {
        common!(self, g => g.covers = covers.clone());
    }

    /// Add a cover URL if absent; returns whether anything changed.
    pub fn add_cover(&mut self, url: &str) -> bool {
        common!(self, g => if g.covers.iter().any(|c| c == url) {
            false
        } else {
            g.covers.push(url.to_owned());
            true
        })
    }

    /// Add or replace a link, keyed by name — re-staging the same source is
    /// idempotent rather than duplicating.
    pub fn upsert_link(&mut self, name: &str, url: &str, link_type: LinkType) {
        common!(self, g => {
            if let Some(link) = g.links.iter_mut().find(|l| l.name == name) {
                link.url = url.to_owned();
                link.link_type = link_type;
            } else {
                g.links.push(Link {
                    name: name.to_owned(),
                    url: url.to_owned(),
                    link_type,
                });
            }
        });
    }

    /// Convenience alias for the one link every commercial game tends to have.
    pub fn set_wikipedia(&mut self, url: &str) {
        self.upsert_link("Wikipedia", url, LinkType::Wiki);
    }

    pub fn links(&self) -> Vec<(String, String)> {
        common!(self, g => g
            .links
            .iter()
            .map(|l| (l.name.clone(), l.url.clone()))
            .collect())
    }

    pub fn tags(&self) -> Vec<String> {
        common!(self, g => g.tags.clone())
    }

    pub fn release_artifact_sha1s(&self, index: usize) -> Vec<String> {
        common!(self, g => g
            .releases
            .get(index)
            .map(|r| r.artifacts.iter().map(|a| a.sha1.as_str().to_owned()).collect())
            .unwrap_or_default())
    }

    pub fn release_publisher(&self, index: usize) -> String {
        common!(self, g => g
            .releases
            .get(index)
            .and_then(|r| r.publisher.clone())
            .unwrap_or_default())
    }

    pub fn set_release_publisher(&mut self, index: usize, value: String) {
        let publisher = (!value.is_empty()).then_some(value.clone());
        common!(self, g => {
            if let Some(release) = g.releases.get_mut(index) {
                release.publisher = publisher.clone();
            }
        });
    }

    pub fn release_titles(&self) -> Vec<String> {
        common!(self, g => g
            .releases
            .iter()
            .filter_map(|r| r.title.clone())
            .collect())
    }

    /// Board hint for the session factory (VCS only — carts have no header,
    /// so the db's word must reach the core).
    pub fn cart_hint(&self) -> Option<String> {
        match self {
            AnyGame::Vcs(g) => g.releases.iter().find_map(|r| r.hardware.cart_type.clone()),
            _ => None,
        }
    }

    /// Stage what a Game Boy header states, filling only unknown fields.
    /// Returns (staged, conflicts-with-db) descriptions.
    pub fn stage_gb_header(
        &mut self,
        header: &crate::verify::GbHeader,
    ) -> (Vec<String>, Vec<String>) {
        use missingno_gamedb::Enhancement;
        let mut staged = Vec::new();
        let mut conflicts = Vec::new();
        match self {
            AnyGame::Gb(g) => {
                if g.releases.is_empty() {
                    return (staged, conflicts);
                }
                if header.cgb_flag == 0xC0 {
                    conflicts
                        .push("header says CGB-only, but this entry is in the gb tree".to_owned());
                }
                let release = &mut g.releases[0];
                let header_sgb = if header.sgb {
                    Enhancement::Enhanced
                } else {
                    Enhancement::NotEnhanced
                };
                let header_cgb = if header.cgb_flag & 0x80 != 0 {
                    Enhancement::Enhanced
                } else {
                    Enhancement::NotEnhanced
                };
                match release.hardware.sgb {
                    Enhancement::Unknown => {
                        release.hardware.sgb = header_sgb;
                        staged.push(format!("sgb: {header_sgb:?}"));
                    }
                    current if current != header_sgb => {
                        conflicts.push(format!("sgb: db {current:?} vs header {header_sgb:?}"))
                    }
                    _ => {}
                }
                match release.hardware.cgb {
                    Enhancement::Unknown => {
                        release.hardware.cgb = header_cgb;
                        staged.push(format!("cgb: {header_cgb:?}"));
                    }
                    current if current != header_cgb => {
                        conflicts.push(format!("cgb: db {current:?} vs header {header_cgb:?}"))
                    }
                    _ => {}
                }
                match &release.hardware.mapper {
                    None => {
                        release.hardware.mapper = Some(header.mapper.clone());
                        staged.push(format!("mapper: {}", header.mapper));
                    }
                    Some(current) if *current != header.mapper => {
                        conflicts.push(format!("mapper: db {current} vs header {}", header.mapper))
                    }
                    _ => {}
                }
            }
            AnyGame::Gbc(g) => {
                if g.releases.is_empty() {
                    return (staged, conflicts);
                }
                if header.cgb_flag & 0x80 == 0 {
                    conflicts.push(
                        "header has no CGB flag, but this entry is in the gbc tree".to_owned(),
                    );
                }
                let release = &mut g.releases[0];
                match &release.hardware.mapper {
                    None => {
                        release.hardware.mapper = Some(header.mapper.clone());
                        staged.push(format!("mapper: {}", header.mapper));
                    }
                    Some(current) if *current != header.mapper => {
                        conflicts.push(format!("mapper: db {current} vs header {}", header.mapper))
                    }
                    _ => {}
                }
            }
            AnyGame::Vcs(_) => {}
        }
        (staged, conflicts)
    }

    /// Agent override: set the first release's mapper (GB/GBC) — for carts
    /// whose headers lie.
    pub fn set_mapper(&mut self, value: &str) -> bool {
        match self {
            AnyGame::Gb(g) => g.releases.first_mut().map(|r| {
                r.hardware.mapper = Some(value.to_owned());
            }),
            AnyGame::Gbc(g) => g.releases.first_mut().map(|r| {
                r.hardware.mapper = Some(value.to_owned());
            }),
            AnyGame::Vcs(_) => None,
        }
        .is_some()
    }

    /// Agent override: set the first release's board (VCS — no headers).
    pub fn set_cart_type(&mut self, value: &str) -> bool {
        match self {
            AnyGame::Vcs(g) => g.releases.first_mut().map(|r| {
                r.hardware.cart_type = Some(value.to_owned());
            }),
            _ => None,
        }
        .is_some()
    }

    /// Broadcast-standard hint for the session factory (VCS only).
    pub fn tv_hint(&self) -> Option<String> {
        match self {
            AnyGame::Vcs(g) => g
                .releases
                .iter()
                .find_map(|r| r.hardware.tv_format)
                .map(|tv| format!("{tv:?}").to_lowercase()),
            _ => None,
        }
    }

    pub fn to_ron_string(&self) -> Result<String, String> {
        common!(self, g => g.to_ron_string().map_err(|e| e.to_string()))
    }
}

/// A JSON string → LinkType, rejecting unknowns with the valid set named.
pub fn parse_link_type(value: &str) -> Result<LinkType, String> {
    Ok(match value {
        "Wiki" => LinkType::Wiki,
        "Manual" => LinkType::Manual,
        "Source" => LinkType::Source,
        "Speedrun" => LinkType::Speedrun,
        "UnusedContent" => LinkType::UnusedContent,
        "TechnicalReference" => LinkType::TechnicalReference,
        "Guide" => LinkType::Guide,
        "Community" => LinkType::Community,
        other => {
            return Err(format!(
                "unknown link_type {other:?}; expected Wiki, Manual, Source, Speedrun,                  UnusedContent, TechnicalReference, Guide, or Community"
            ));
        }
    })
}

pub fn parse_mod_category(value: &str) -> Result<ModCategory, String> {
    Ok(match value {
        "Translation" => ModCategory::Translation,
        "QualityOfLife" => ModCategory::QualityOfLife,
        "ContentChange" => ModCategory::ContentChange,
        "TotalConversion" => ModCategory::TotalConversion,
        other => {
            return Err(format!(
                "unknown category {other:?}; expected Translation, QualityOfLife,                  ContentChange, or TotalConversion"
            ));
        }
    })
}

fn slugify(title: &str) -> String {
    let mut slug = String::new();
    let mut gap = false;
    for c in title.chars() {
        if c.is_ascii_alphanumeric() {
            if gap && !slug.is_empty() {
                slug.push('-');
            }
            slug.push(c.to_ascii_lowercase());
            gap = false;
        } else if c != '\'' && c != '\u{2019}' {
            gap = true;
        }
    }
    slug
}

/// Remove `sha1` from whichever release holds it and build the derived-work
/// entry that inherits that release's status and hardware.
fn split_hack_from<P: Platform>(
    source: &mut Game<P>,
    sha1: &str,
    title: String,
    category: ModCategory,
    base: Sha1,
) -> Option<Game<P>> {
    for release in &mut source.releases {
        let Some(at) = release
            .artifacts
            .iter()
            .position(|a| a.sha1.as_str() == sha1)
        else {
            continue;
        };
        let artifact = release.artifacts.remove(at);
        return Some(Game {
            title,
            kind: GameKind::Game,
            developer: None,
            description: None,
            license: None,
            tags: Vec::new(),
            links: Vec::new(),
            covers: Vec::new(),
            screenshots: Vec::new(),
            mod_of: Some(ModOf {
                base_sha1: base,
                category,
                patch: None,
            }),
            curated: Vec::new(),
            releases: vec![Release {
                title: None,
                label: None,
                regions: Vec::new(),
                date: None,
                publisher: None,
                status: release.status,
                hardware: release.hardware.clone(),
                sources: Vec::new(),
                artifacts: vec![artifact],
            }],
        });
    }
    None
}

pub struct EntryHandle {
    pub tree: TreeId,
    pub slug: String,
    pub game: AnyGame,
    pub dirty: bool,
}

impl EntryHandle {
    pub fn key(&self) -> String {
        format!("{}/{}", self.tree.dir(), self.slug)
    }
}

pub struct Db {
    pub repo_root: PathBuf,
    pub entries: Vec<EntryHandle>,
    pub flags: FlagFile,
    /// Files written since the last commit.
    pub uncommitted: usize,
}

impl Db {
    pub fn load(repo_root: PathBuf) -> io::Result<Self> {
        let data_root = repo_root.join("data");
        let mut entries = Vec::new();
        fn load_tree<P: Platform>(
            data_root: &std::path::Path,
            tree: TreeId,
            wrap: impl Fn(Game<P>) -> AnyGame,
            out: &mut Vec<EntryHandle>,
        ) -> io::Result<()> {
            let (loaded, issues) = Tree::<P>::load(data_root)?;
            if let Some(first) = issues.first() {
                return Err(io::Error::other(format!(
                    "{} manifests failed to load; first: {}: {}",
                    issues.len(),
                    first.path.display(),
                    first.message
                )));
            }
            for entry in loaded.games {
                out.push(EntryHandle {
                    tree,
                    slug: entry.slug.as_str().to_owned(),
                    game: wrap(entry.game),
                    dirty: false,
                });
            }
            Ok(())
        }
        load_tree::<GameBoy>(&data_root, TreeId::Gb, AnyGame::Gb, &mut entries)?;
        load_tree::<GameBoyColor>(&data_root, TreeId::Gbc, AnyGame::Gbc, &mut entries)?;
        load_tree::<Vcs>(&data_root, TreeId::Vcs, AnyGame::Vcs, &mut entries)?;
        let flags = FlagFile::load(&repo_root)?;
        Ok(Self {
            repo_root,
            entries,
            flags,
            uncommitted: 0,
        })
    }

    pub fn backlog_count(&self, tree: TreeId) -> usize {
        self.entries
            .iter()
            .filter(|e| e.tree == tree && e.game.curations().is_empty())
            .count()
    }

    /// Write a dirty entry's manifest back in canonical form.
    pub fn write_entry(&mut self, index: usize) -> io::Result<()> {
        let entry = &mut self.entries[index];
        let text = entry.game.to_ron_string().map_err(io::Error::other)?;
        let dir = self
            .repo_root
            .join("data")
            .join(entry.tree.dir())
            .join(&entry.slug);
        fs::write(dir.join("manifest.ron"), text)?;
        entry.dirty = false;
        self.uncommitted += 1;
        Ok(())
    }

    pub fn save_flags(&mut self) -> io::Result<()> {
        self.flags.save(&self.repo_root)?;
        self.uncommitted += 1;
        Ok(())
    }

    /// A dump that turned out to be a hack: pull it out of `source` and give
    /// it its own derived-work entry pointing at its base. Returns the new key.
    pub fn mark_hack(
        &mut self,
        source: usize,
        sha1: &str,
        title: Option<String>,
        category: ModCategory,
        base_override: Option<String>,
    ) -> Result<String, String> {
        let tree = self.entries[source].tree;
        let source_title = self.entries[source].game.title().to_owned();
        if !self.entries[source]
            .game
            .artifact_sha1s()
            .iter()
            .any(|s| s == sha1)
        {
            return Err(format!("{sha1} is not an artifact of this entry"));
        }
        let base: Sha1 = match base_override {
            Some(base) => base.parse()?,
            None => self.entries[source]
                .game
                .artifact_sha1s()
                .into_iter()
                .find(|s| s != sha1)
                .ok_or("no other artifact to use as the base — pass base_sha1")?
                .parse()?,
        };
        let title = title.unwrap_or_else(|| format!("Hack of {source_title}"));

        let mut slug = slugify(&title);
        let taken: std::collections::HashSet<String> = self
            .entries
            .iter()
            .filter(|e| e.tree == tree)
            .map(|e| e.slug.clone())
            .collect();
        let mut n = 1;
        while taken.contains(&slug) {
            n += 1;
            slug = format!("{}-{n}", slugify(&title));
        }

        let hack = match &mut self.entries[source].game {
            AnyGame::Gb(g) => split_hack_from(g, sha1, title, category, base).map(AnyGame::Gb),
            AnyGame::Gbc(g) => split_hack_from(g, sha1, title, category, base).map(AnyGame::Gbc),
            AnyGame::Vcs(g) => split_hack_from(g, sha1, title, category, base).map(AnyGame::Vcs),
        }
        .ok_or("artifact vanished mid-operation")?;

        self.entries.push(EntryHandle {
            tree,
            slug: slug.clone(),
            game: hack,
            dirty: true,
        });
        let new_index = self.entries.len() - 1;
        self.write_entry(source).map_err(|e| e.to_string())?;
        self.write_entry(new_index).map_err(|e| e.to_string())?;
        Ok(format!("{}/{slug}", tree.dir()))
    }

    pub fn commit(&mut self, message: &str) -> Result<String, String> {
        let run = |args: &[&str]| {
            let output = Command::new("git")
                .arg("-C")
                .arg(&self.repo_root)
                .args(args)
                .output()
                .map_err(|e| e.to_string())?;
            if output.status.success() {
                Ok(String::from_utf8_lossy(&output.stdout).into_owned())
            } else {
                Err(String::from_utf8_lossy(&output.stderr).into_owned())
            }
        };
        run(&["add", "data", "curation"])?;
        run(&["commit", "-m", message])?;
        self.uncommitted = 0;
        run(&["log", "--oneline", "-1"])
    }

    pub fn today() -> Date {
        jiff::Zoned::now()
            .date()
            .to_string()
            .parse()
            .expect("jiff civil date is YYYY-MM-DD")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Load the real checkout when present: every manifest parses into the
    // curator's editing surface and the flag file round-trips.
    #[test]
    fn real_gamedb_loads() {
        let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../missingno-gamedb");
        if !repo.join("data/gb").is_dir() {
            return;
        }
        let db = Db::load(repo).expect("gamedb loads");
        assert!(db.entries.len() > 8000, "{}", db.entries.len());
        assert!(db.flags.open().count() > 2000);
        assert!(db.backlog_count(TreeId::Vcs) > 4000);
    }
}

#[cfg(test)]
mod link_tests {
    use super::*;
    use missingno_gamedb::{Game, GameBoy};

    #[test]
    fn upsert_link_is_idempotent_and_updates() {
        let game = Game::<GameBoy>::from_ron(
            r#"(title: "T", releases: [(sources: [Download(url: "x")])])"#,
        )
        .unwrap();
        let mut any = AnyGame::Gb(game);
        any.upsert_link("AtariAge", "https://atariage.com/a", LinkType::Community);
        any.upsert_link("AtariAge", "https://atariage.com/a", LinkType::Community);
        assert_eq!(any.links().len(), 1);
        any.upsert_link(
            "AtariAge",
            "https://atariage.com/b",
            LinkType::TechnicalReference,
        );
        assert_eq!(
            any.links(),
            vec![("AtariAge".to_owned(), "https://atariage.com/b".to_owned())]
        );
        any.set_wikipedia("https://en.wikipedia.org/wiki/T");
        any.set_wikipedia("https://en.wikipedia.org/wiki/T");
        assert_eq!(any.links().len(), 2);
    }

    #[test]
    fn link_type_parse_rejects_unknowns_usefully() {
        assert!(parse_link_type("Guide").is_ok());
        let err = parse_link_type("Blog").unwrap_err();
        assert!(err.contains("Blog") && err.contains("Community"));
    }
}
