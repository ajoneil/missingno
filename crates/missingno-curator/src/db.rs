//! The curator's view of the gamedb checkout: typed manifests behind a
//! platform-agnostic editing surface, plus flags and git state.

use std::{fs, io, path::PathBuf, process::Command};

use missingno_gamedb::{
    Date, FlagFile, Game, GameBoy, GameBoyColor, GameKind, Link, LinkType, Mod, ModCategory, ModOf,
    ModRelease, Platform, Release, Sha1, Tree, Vcs, Verification, VerificationMethod,
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
                    label: None,
                    size: Some(size),
                    verified: Vec::new(),
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

    pub fn release_artifacts(&self, index: usize) -> Vec<(String, String)> {
        common!(self, g => g
            .releases
            .get(index)
            .map(|r| r
                .artifacts
                .iter()
                .map(|a| (
                    a.sha1.as_str().to_owned(),
                    a.label.clone().unwrap_or_default(),
                ))
                .collect())
            .unwrap_or_default())
    }

    pub fn set_artifact_label(&mut self, sha1: &str, label: &str) -> bool {
        let value = (!label.is_empty()).then(|| label.to_owned());
        common!(self, g => {
            for release in &mut g.releases {
                if let Some(artifact) =
                    release.artifacts.iter_mut().find(|a| a.sha1.as_str() == sha1)
                {
                    artifact.label = value.clone();
                    return true;
                }
            }
            for game_mod in &mut g.mods {
                for release in &mut game_mod.releases {
                    if let Some(artifact) =
                        release.artifacts.iter_mut().find(|a| a.sha1.as_str() == sha1)
                    {
                        artifact.label = value.clone();
                        return true;
                    }
                }
            }
            false
        })
    }

    /// One display line per attached mod, with its links.
    pub fn mod_lines(&self) -> Vec<(String, Vec<(String, String)>)> {
        common!(self, g => g
            .mods
            .iter()
            .map(|m| {
                let curations = if m.curated.is_empty() {
                    " · unreviewed".to_owned()
                } else {
                    format!(
                        " · curated by {}",
                        m.curated
                            .iter()
                            .map(|c| format!(
                                "{}{}",
                                c.by,
                                if c.recommended { " ★" } else { "" }
                            ))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                };
                (
                    format!(
                        "{} · {:?}{} · {} version(s){curations}",
                        m.name,
                        m.category,
                        m.author
                            .as_ref()
                            .map(|a| format!(" · by {a}"))
                            .unwrap_or_default(),
                        m.releases.len()
                    ),
                    m.links.iter().map(|l| (l.name.clone(), l.url.clone())).collect(),
                )
            })
            .collect())
    }

    /// Record that a signature database recognised a dump — additive evidence
    /// about an immutable hash; curations are untouched. Replaces an earlier
    /// answer from the same database (a refresh proves it still holds).
    pub fn record_signature(&mut self, sha1: &str, database: &str, entry: &str) -> bool {
        let evidence = Verification {
            method: VerificationMethod::Signature {
                database: database.to_owned(),
                entry: entry.to_owned(),
            },
            date: Db::today(),
        };
        common!(self, g => {
            for release in &mut g.releases {
                for artifact in &mut release.artifacts {
                    if artifact.sha1.as_str() != sha1 {
                        continue;
                    }
                    let existing = artifact.verified.iter_mut().find(|v| {
                        matches!(&v.method, VerificationMethod::Signature { database: d, .. }
                            if d == database)
                    });
                    match existing {
                        Some(slot) if *slot == evidence => return false,
                        Some(slot) => *slot = evidence.clone(),
                        None => artifact.verified.push(evidence.clone()),
                    }
                    return true;
                }
            }
            false
        })
    }

    /// The developer saw this exact dump run. Never inferred — only the
    /// explicit button writes it. Mod dumps count: people play hacks.
    pub fn record_playtest(&mut self, sha1: &str, by: &str) -> bool {
        let evidence = Verification {
            method: VerificationMethod::Playtest { by: by.to_owned() },
            date: Db::today(),
        };
        let upsert = |artifact: &mut missingno_gamedb::Artifact| {
            let existing = artifact
                .verified
                .iter_mut()
                .find(|v| matches!(&v.method, VerificationMethod::Playtest { by: b } if b == by));
            match existing {
                Some(slot) => *slot = evidence.clone(),
                None => artifact.verified.push(evidence.clone()),
            }
        };
        common!(self, g => {
            for release in &mut g.releases {
                if let Some(artifact) =
                    release.artifacts.iter_mut().find(|a| a.sha1.as_str() == sha1)
                {
                    upsert(artifact);
                    return true;
                }
            }
            for game_mod in &mut g.mods {
                for release in &mut game_mod.releases {
                    if let Some(artifact) =
                        release.artifacts.iter_mut().find(|a| a.sha1.as_str() == sha1)
                    {
                        upsert(artifact);
                        return true;
                    }
                }
            }
            false
        })
    }

    /// Short display marks for a dump's recorded verifications.
    pub fn verification_marks(&self, sha1: &str) -> Vec<String> {
        let marks = |artifact: &missingno_gamedb::Artifact| {
            artifact
                .verified
                .iter()
                .map(|v| match &v.method {
                    VerificationMethod::Signature { database, .. } => format!("✓{database}"),
                    VerificationMethod::Playtest { by } => format!("▶{by}"),
                })
                .collect::<Vec<_>>()
        };
        common!(self, g => {
            for release in &g.releases {
                if let Some(a) = release.artifacts.iter().find(|a| a.sha1.as_str() == sha1) {
                    return marks(a);
                }
            }
            for game_mod in &g.mods {
                for release in &game_mod.releases {
                    if let Some(a) =
                        release.artifacts.iter().find(|a| a.sha1.as_str() == sha1)
                    {
                        return marks(a);
                    }
                }
            }
            Vec::new()
        })
    }

    /// Run an edit against the named attached mod (Mod is platform-shared).
    pub fn edit_mod<R>(&mut self, name: &str, edit: impl FnOnce(&mut Mod) -> R) -> Option<R> {
        common!(self, g => g.mods.iter_mut().find(|m| m.name == name).map(edit))
    }

    pub fn mod_names(&self) -> Vec<String> {
        common!(self, g => g.mods.iter().map(|m| m.name.clone()).collect())
    }

    /// Endorse one attached mod, independently of the game.
    pub fn stamp_mod_curation(&mut self, index: usize, by: &str, recommended: bool) -> bool {
        let stamp = missingno_gamedb::Curation {
            by: by.to_owned(),
            date: Db::today(),
            recommended,
        };
        common!(self, g => {
            let Some(m) = g.mods.get_mut(index) else { return false };
            match m.curated.iter_mut().find(|c| c.by == stamp.by) {
                Some(existing) => *existing = stamp.clone(),
                None => m.curated.push(stamp.clone()),
            }
            true
        })
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

    /// TV/board hints for booting one specific dump: the release that owns it
    /// speaks first; a mod's dump answers through its base's release; only
    /// then fall back to the entry's first stated values.
    pub fn hints_for(&self, sha1: &str) -> (Option<String>, Option<String>) {
        let AnyGame::Vcs(g) = self else {
            return (None, None);
        };
        let release_hints = |r: &Release<Vcs>| {
            (
                r.hardware
                    .tv_format
                    .map(|tv| format!("{tv:?}").to_lowercase()),
                r.hardware.cart_type.clone(),
            )
        };
        for release in &g.releases {
            if release.artifacts.iter().any(|a| a.sha1.as_str() == sha1) {
                return release_hints(release);
            }
        }
        for game_mod in &g.mods {
            for mod_release in &game_mod.releases {
                if mod_release
                    .artifacts
                    .iter()
                    .any(|a| a.sha1.as_str() == sha1)
                    && let Some(base) = &mod_release.base_sha1
                    && let Some(release) = g
                        .releases
                        .iter()
                        .find(|r| r.artifacts.iter().any(|a| a.sha1 == *base))
                {
                    return release_hints(release);
                }
            }
        }
        (self.tv_hint(), self.cart_hint())
    }

    /// Every dump attached to the game's mods, flattened.
    pub fn mod_artifacts(&self, index: usize) -> Vec<(String, String)> {
        common!(self, g => g
            .mods
            .get(index)
            .map(|m| m
                .releases
                .iter()
                .flat_map(|r| &r.artifacts)
                .map(|a| (
                    a.sha1.as_str().to_owned(),
                    a.label.clone().unwrap_or_default(),
                ))
                .collect())
            .unwrap_or_default())
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
    homepage: Option<String>,
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
            links: homepage
                .map(|url| {
                    vec![Link {
                        name: "Homepage".to_owned(),
                        url,
                        link_type: LinkType::Community,
                    }]
                })
                .unwrap_or_default(),
            covers: Vec::new(),
            screenshots: Vec::new(),
            mod_of: Some(ModOf {
                base_sha1: base,
                category,
                patch: None,
            }),
            mods: Vec::new(),
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

/// Move `sha1` out of its release into a mod attached to the same game.
fn attach_mod<P: Platform>(
    source: &mut Game<P>,
    sha1: &str,
    name: String,
    category: ModCategory,
    homepage: Option<String>,
    base_sha1: Option<Sha1>,
) -> bool {
    for release in &mut source.releases {
        let Some(at) = release
            .artifacts
            .iter()
            .position(|a| a.sha1.as_str() == sha1)
        else {
            continue;
        };
        let artifact = release.artifacts.remove(at);
        source.mods.push(Mod {
            name,
            category,
            author: None,
            curated: Vec::new(),
            links: homepage
                .map(|url| {
                    vec![Link {
                        name: "Homepage".to_owned(),
                        url,
                        link_type: LinkType::Community,
                    }]
                })
                .unwrap_or_default(),
            releases: vec![ModRelease {
                label: None,
                date: None,
                base_sha1,
                patch: None,
                sources: Vec::new(),
                artifacts: vec![artifact],
            }],
        });
        return true;
    }
    false
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
        fs::create_dir_all(&dir)?;
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

    /// A dump that turned out to be a hack. Modifications of the game — QoL,
    /// content changes, translations (a translated game is still the same
    /// game, exactly as official localizations are releases of it) — attach
    /// as mods; only total conversions get their own entry.
    pub fn mark_hack(
        &mut self,
        source: usize,
        sha1: &str,
        title: Option<String>,
        category: ModCategory,
        base_override: Option<String>,
        homepage: Option<String>,
    ) -> Result<String, String> {
        if !matches!(category, ModCategory::TotalConversion) {
            let source_title = self.entries[source].game.title().to_owned();
            let name = title.unwrap_or_else(|| format!("Unnamed hack of {source_title}"));
            let base = self.resolve_base(source, sha1, base_override)?;
            let attached = match &mut self.entries[source].game {
                AnyGame::Gb(g) => attach_mod(g, sha1, name.clone(), category, homepage, base),
                AnyGame::Gbc(g) => attach_mod(g, sha1, name.clone(), category, homepage, base),
                AnyGame::Vcs(g) => attach_mod(g, sha1, name.clone(), category, homepage, base),
            };
            if !attached {
                return Err(format!("{sha1} is not an artifact of this entry"));
            }
            // Re-filing a dump changes what the entry claims: un-vouch it.
            self.entries[source].game.clear_curations();
            self.entries[source].dirty = true;
            self.write_entry(source).map_err(|e| e.to_string())?;
            return Ok(format!(
                "{} (as attached mod {name:?})",
                self.entries[source].key()
            ));
        }
        self.split_out_conversion(source, sha1, title, category, base_override, homepage)
    }

    /// The dump a mod derives from: an explicit base must be one of the
    /// entry's artifacts; without one, a single remaining candidate is used,
    /// none is honestly None, and several is a refusal — never a guess.
    fn resolve_base(
        &self,
        source: usize,
        hack_sha1: &str,
        base_override: Option<String>,
    ) -> Result<Option<Sha1>, String> {
        let candidates: Vec<String> = self.entries[source]
            .game
            .artifact_sha1s()
            .into_iter()
            .filter(|s| s != hack_sha1)
            .collect();
        match base_override {
            Some(base) => {
                let base = base.to_ascii_lowercase();
                if base == hack_sha1 {
                    return Err("base_sha1 is the hack itself".to_owned());
                }
                if !candidates.contains(&base) {
                    return Err(format!(
                        "base_sha1 {base} is not an artifact of this entry; artifacts: {}",
                        candidates.join(", ")
                    ));
                }
                Ok(Some(base.parse()?))
            }
            None => match candidates.as_slice() {
                [] => Ok(None),
                [only] => Ok(Some(only.parse()?)),
                many => Err(format!(
                    "several dumps could be the base — pass base_sha1; candidates: {}",
                    many.join(", ")
                )),
            },
        }
    }

    fn split_out_conversion(
        &mut self,
        source: usize,
        sha1: &str,
        title: Option<String>,
        category: ModCategory,
        base_override: Option<String>,
        homepage: Option<String>,
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
        let base: Sha1 = self
            .resolve_base(source, sha1, base_override)?
            .ok_or("a total conversion needs a base artifact — pass base_sha1")?;
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
            AnyGame::Gb(g) => {
                split_hack_from(g, sha1, title, category, base, homepage).map(AnyGame::Gb)
            }
            AnyGame::Gbc(g) => {
                split_hack_from(g, sha1, title, category, base, homepage).map(AnyGame::Gbc)
            }
            AnyGame::Vcs(g) => {
                split_hack_from(g, sha1, title, category, base, homepage).map(AnyGame::Vcs)
            }
        }
        .ok_or("artifact vanished mid-operation")?;

        self.entries[source].game.clear_curations();
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

#[cfg(test)]
mod mark_hack_tests {
    use super::*;
    use missingno_gamedb::ModCategory;

    fn db_with_three_dumps() -> (tempfile::TempDir, Db) {
        let dir = tempfile::tempdir().unwrap();
        let game_dir = dir.path().join("data/vcs/adventure");
        std::fs::create_dir_all(&game_dir).unwrap();
        std::fs::write(
            game_dir.join("manifest.ron"),
            r#"(
    title: "Adventure",
    releases: [
        (
            artifacts: [
                (sha1: "e07e48d463d30321239a8acc00c490f27f1f7422"),
                (sha1: "4ffe36c574a30188db7f9548d5e9ac36c9df5a09"),
                (sha1: "7362b7ee00e4e0d777100dc8b70ba6b4a5e6ee6e"),
            ],
        ),
    ],
)
"#,
        )
        .unwrap();
        let db = Db::load(dir.path().to_path_buf()).unwrap();
        (dir, db)
    }

    const REAL: &str = "e07e48d463d30321239a8acc00c490f27f1f7422";
    const HACK_A: &str = "4ffe36c574a30188db7f9548d5e9ac36c9df5a09";
    const HACK_B: &str = "7362b7ee00e4e0d777100dc8b70ba6b4a5e6ee6e";

    fn mod_bases(db: &Db) -> Vec<Option<String>> {
        match &db.entries[0].game {
            AnyGame::Vcs(g) => g
                .mods
                .iter()
                .map(|m| {
                    m.releases[0]
                        .base_sha1
                        .as_ref()
                        .map(|s| s.as_str().to_owned())
                })
                .collect(),
            _ => unreachable!(),
        }
    }

    // The reproduced bug: an explicit base must be recorded verbatim, in
    // whatever order the hacks are marked.
    #[test]
    fn explicit_base_is_recorded_regardless_of_order() {
        for order in [[HACK_A, HACK_B], [HACK_B, HACK_A]] {
            let (_dir, mut db) = db_with_three_dumps();
            for hack in order {
                db.mark_hack(
                    0,
                    hack,
                    Some(format!("hack {hack}")),
                    ModCategory::ContentChange,
                    Some(REAL.to_owned()),
                    None,
                )
                .unwrap();
            }
            assert_eq!(
                mod_bases(&db),
                vec![Some(REAL.to_owned()), Some(REAL.to_owned())]
            );
        }
    }

    #[test]
    fn ambiguous_base_refuses_rather_than_guessing() {
        let (_dir, mut db) = db_with_three_dumps();
        let err = db
            .mark_hack(0, HACK_A, None, ModCategory::ContentChange, None, None)
            .unwrap_err();
        assert!(err.contains("pass base_sha1"), "{err}");
        assert!(
            mod_bases(&db).is_empty(),
            "nothing may be written on refusal"
        );
    }

    #[test]
    fn single_candidate_is_used_and_lone_dump_gets_none() {
        let (_dir, mut db) = db_with_three_dumps();
        db.mark_hack(
            0,
            HACK_A,
            None,
            ModCategory::ContentChange,
            Some(REAL.to_owned()),
            None,
        )
        .unwrap();
        // Two dumps left (REAL, HACK_B): marking HACK_B has one candidate.
        db.mark_hack(0, HACK_B, None, ModCategory::ContentChange, None, None)
            .unwrap();
        assert_eq!(mod_bases(&db)[1], Some(REAL.to_owned()));
    }

    #[test]
    fn bogus_base_is_rejected_not_stored() {
        let (_dir, mut db) = db_with_three_dumps();
        let err = db
            .mark_hack(
                0,
                HACK_A,
                None,
                ModCategory::ContentChange,
                Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_owned()),
                None,
            )
            .unwrap_err();
        assert!(err.contains("not an artifact"), "{err}");
        assert!(mod_bases(&db).is_empty());
    }
}

#[cfg(test)]
mod verification_tests {
    use super::*;
    use missingno_gamedb::VerificationMethod;

    #[test]
    fn signature_evidence_is_idempotent_and_preserves_curations() {
        let game = Game::<GameBoy>::from_ron(
            r#"(
    title: "T",
    curated: [(by: "andrew", date: "2026-07-21", recommended: true)],
    releases: [(artifacts: [(sha1: "0123456789abcdef0123456789abcdef01234567")])],
)"#,
        )
        .unwrap();
        let mut any = AnyGame::Gb(game);
        assert!(any.record_signature(
            "0123456789abcdef0123456789abcdef01234567",
            "Hasheous",
            "T (1983)(Someone)(NTSC)"
        ));
        // Same evidence again: unchanged, no duplicate.
        assert!(!any.record_signature(
            "0123456789abcdef0123456789abcdef01234567",
            "Hasheous",
            "T (1983)(Someone)(NTSC)"
        ));
        // A different answer from the same database replaces, not stacks.
        assert!(any.record_signature(
            "0123456789abcdef0123456789abcdef01234567",
            "Hasheous",
            "T (1983)(Someone)(PAL)"
        ));
        let AnyGame::Gb(g) = &any else { unreachable!() };
        let artifact = &g.releases[0].artifacts[0];
        assert_eq!(artifact.verified.len(), 1);
        assert!(matches!(
            &artifact.verified[0].method,
            VerificationMethod::Signature { entry, .. } if entry.ends_with("(PAL)")
        ));
        assert_eq!(g.curated.len(), 1, "verification never clears curations");
    }
}
