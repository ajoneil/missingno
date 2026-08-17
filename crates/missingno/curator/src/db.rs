//! The curator's view of the gamedb checkout: typed manifests behind a
//! platform-agnostic editing surface, plus flags and git state.

use std::{fs, io, path::PathBuf};

use missingno_gamedb::{
    Controller, Defect, FlagFile, Game, GameBoy, GameBoyColor, GameKind, GbCartType, Language,
    Link, LinkType, Mod, ModCategory, ModOf, ModRelease, Platform, Region, Release, ReleaseStatus,
    Sg1000, Sg1000CartType, Sha1, Slug, Tree, TvFormat, Vcs, VcsCartType,
};

use crate::vocabulary;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TreeId {
    Gb,
    Gbc,
    Sg1000,
    Vcs,
}

impl TreeId {
    pub fn dir(self) -> &'static str {
        match self {
            TreeId::Gb => "gb",
            TreeId::Gbc => "gbc",
            TreeId::Sg1000 => "sg1000",
            TreeId::Vcs => "vcs",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            TreeId::Gb => "Game Boy",
            TreeId::Gbc => "Game Boy Color",
            TreeId::Sg1000 => "SG-1000",
            TreeId::Vcs => "Atari VCS",
        }
    }
}

/// One manifest, kept in its platform's schema type.
pub enum AnyGame {
    Gb(Game<GameBoy>),
    Gbc(Game<GameBoyColor>),
    Sg1000(Game<Sg1000>),
    Vcs(Game<Vcs>),
}

macro_rules! common {
    ($self:expr, $game:ident => $body:expr) => {
        match $self {
            AnyGame::Gb($game) => $body,
            AnyGame::Gbc($game) => $body,
            AnyGame::Sg1000($game) => $body,
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
}

/// A release's header, split so the shipped title and box label can be styled
/// apart from the remaining facts.
pub struct ReleaseLine {
    pub title: Option<String>,
    pub label: Option<String>,
    pub detail: String,
}

/// The VCS facts that vary from release to release, as one display string.
fn vcs_hardware(hardware: &missingno_gamedb::VcsHardware) -> String {
    [
        hardware.tv_format.map(|t| format!("{t:?}")),
        hardware.cart_type.map(|c| c.code().to_owned()),
        (!hardware.controllers.is_empty()).then(|| {
            hardware
                .controllers
                .iter()
                .map(|c| format!("{c:?}"))
                .collect::<Vec<_>>()
                .join("/")
        }),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" ")
}

impl AnyGame {
    pub fn title(&self) -> &str {
        common!(self, g => &g.title)
    }

    pub fn kind(&self) -> GameKind {
        common!(self, g => g.kind)
    }

    pub fn set_kind(&mut self, kind: GameKind) {
        common!(self, g => g.kind = kind)
    }

    pub fn curated(&self) -> bool {
        common!(self, g => g.curated)
    }

    pub fn adult(&self) -> bool {
        common!(self, g => g.adult)
    }

    pub fn set_adult(&mut self, adult: bool) {
        common!(self, g => g.adult = adult);
    }

    pub fn recommended_by(&self) -> &[String] {
        common!(self, g => &g.recommended_by)
    }

    /// Mark reviewed; a recommendation adds the curator's identifier once.
    pub fn stamp_curation(&mut self, by: &str, recommended: bool) {
        common!(self, g => {
            g.curated = true;
            if recommended && !g.recommended_by.iter().any(|id| id == by) {
                g.recommended_by.push(by.to_owned());
            }
        });
    }

    pub fn text_field(&self, field: TextField) -> String {
        common!(self, g => match field {
            TextField::Title => g.title.clone(),
            TextField::Developer => g.developer.clone().unwrap_or_default(),
            TextField::Description => g.description.clone().unwrap_or_default(),
        })
    }

    pub fn set_text_field(&mut self, field: TextField, value: String) {
        let optional = (!value.is_empty()).then_some(value.clone());
        common!(self, g => match field {
            TextField::Title => g.title = value,
            TextField::Developer => g.developer = optional,
            TextField::Description => g.description = optional,
        });
    }

    /// One display line per release, split so the renderer can style the
    /// shipped title and box label differently from the remaining facts.
    pub fn release_lines(&self) -> Vec<ReleaseLine> {
        fn line<P: Platform>(r: &missingno_gamedb::Release<P>, extra: &str) -> ReleaseLine {
            let mut parts = Vec::new();
            if !r.regions.is_empty() {
                parts.push(
                    r.regions
                        .iter()
                        .map(|region| format!("{region:?}"))
                        .collect::<Vec<_>>()
                        .join("/"),
                );
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
            ReleaseLine {
                title: r.title.clone(),
                label: r.label.clone(),
                detail: parts.join(" · "),
            }
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
            AnyGame::Sg1000(g) => g
                .releases
                .iter()
                .map(|r| {
                    line(
                        r,
                        r.hardware.cart_type.map(Sg1000CartType::code).unwrap_or(""),
                    )
                })
                .collect(),
            AnyGame::Vcs(g) => g
                .releases
                .iter()
                .map(|r| line(r, &vcs_hardware(&r.hardware)))
                .collect(),
        }
    }

    /// First directly-downloadable URL: a game-level `Download` link.
    pub fn download_url(&self) -> Option<String> {
        common!(self, g => g
            .links
            .iter()
            .find(|l| l.link_type == missingno_gamedb::LinkType::Download)
            .map(|l| l.url.clone()))
    }

    pub fn artifact_sha1s(&self) -> Vec<String> {
        common!(self, g => g
            .releases
            .iter()
            .flat_map(|r| &r.artifacts)
            .map(|a| a.sha1.as_str().to_owned())
            .collect())
    }

    /// The dumps held by the entry's mods, which a further derivation can be
    /// based on: a Supercharger conversion of a hack patches the hack's dump.
    pub fn mod_artifact_sha1s(&self) -> Vec<String> {
        common!(self, g => g
            .mods
            .iter()
            .flat_map(|m| &m.releases)
            .flat_map(|r| &r.artifacts)
            .map(|a| a.sha1.as_str().to_owned())
            .collect())
    }

    /// Take another entry's releases and mods into this one — the two
    /// described one game (an unlicensed reissue filed as its own entry).
    /// Dumps already held here are dropped rather than duplicated.
    pub fn absorb(&mut self, other: AnyGame) -> Result<(usize, usize), String> {
        let held = self.artifact_sha1s();
        match (self, other) {
            (AnyGame::Gb(into), AnyGame::Gb(from)) => Ok(absorb_into(into, from, &held)),
            (AnyGame::Gbc(into), AnyGame::Gbc(from)) => Ok(absorb_into(into, from, &held)),
            (AnyGame::Sg1000(into), AnyGame::Sg1000(from)) => Ok(absorb_into(into, from, &held)),
            (AnyGame::Vcs(into), AnyGame::Vcs(from)) => Ok(absorb_into(into, from, &held)),
            _ => Err("the two entries are on different platforms".to_owned()),
        }
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
            if let Some(release) = g.releases.get_mut(0) {
                release.artifacts.push(missingno_gamedb::Artifact {
                    sha1,
                    label: None,
                    defect: None,
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
        common!(self, g => g.covers = covers);
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
    /// Drop links by name — the `wikipedia` field mints its own "Wikipedia"
    /// link, so a hand-added one for the same article is a duplicate.
    /// Returns whether anything went.
    pub fn remove_links(&mut self, names: &[String]) -> bool {
        common!(self, g => {
            let before = g.links.len();
            g.links.retain(|l| !names.iter().any(|n| n == &l.name));
            g.links.len() != before
        })
    }

    pub fn upsert_link(
        &mut self,
        name: &str,
        url: &str,
        link_type: LinkType,
        languages: Vec<Language>,
    ) {
        common!(self, g => {
            if let Some(link) = g.links.iter_mut().find(|l| l.name == name) {
                link.url = url.to_owned();
                link.link_type = link_type;
                link.languages = languages;
            } else {
                g.links.push(Link {
                    name: name.to_owned(),
                    url: url.to_owned(),
                    link_type,
                    languages,
                });
            }
        });
    }

    /// Convenience alias for the one link every commercial game tends to have.
    pub fn set_wikipedia(&mut self, url: &str) {
        self.upsert_link("Wikipedia", url, LinkType::Wiki, Vec::new());
    }

    /// Each link as (name, url, languages) — languages joined for display, empty
    /// when the link is English (the default).
    pub fn links(&self) -> Vec<(String, String, String)> {
        common!(self, g => g
            .links
            .iter()
            .map(|l| (
                l.name.clone(),
                l.url.clone(),
                l.languages.iter().map(|lang| lang.label()).collect::<Vec<_>>().join(", "),
            ))
            .collect())
    }

    pub fn tags(&self) -> Vec<String> {
        common!(self, g => g.tags.clone())
    }

    pub fn release_artifacts(&self, index: usize) -> Vec<(String, String, Option<Defect>)> {
        common!(self, g => g
            .releases
            .get(index)
            .map(|r| r
                .artifacts
                .iter()
                .map(|a| (
                    a.sha1.as_str().to_owned(),
                    a.label.clone().unwrap_or_default(),
                    a.defect,
                ))
                .collect())
            .unwrap_or_default())
    }

    /// Edit the dump with this hash, wherever it hangs — a release's or a mod
    /// release's. False when the entry doesn't hold it.
    fn with_artifact(
        &mut self,
        sha1: &str,
        edit: impl FnOnce(&mut missingno_gamedb::Artifact),
    ) -> bool {
        common!(self, g => {
            let found = g
                .releases
                .iter_mut()
                .flat_map(|r| r.artifacts.iter_mut())
                .chain(
                    g.mods
                        .iter_mut()
                        .flat_map(|m| m.releases.iter_mut())
                        .flat_map(|r| r.artifacts.iter_mut()),
                )
                .find(|a| a.sha1.as_str() == sha1);
            match found {
                Some(artifact) => {
                    edit(artifact);
                    true
                }
                None => false,
            }
        })
    }

    pub fn set_artifact_label(&mut self, sha1: &str, label: &str) -> bool {
        let value = (!label.is_empty()).then(|| label.to_owned());
        self.with_artifact(sha1, |artifact| artifact.label = value)
    }

    /// Set (or clear, with `None`) a dump's quality defect.
    pub fn set_artifact_defect(&mut self, sha1: &str, defect: Option<Defect>) -> bool {
        self.with_artifact(sha1, |artifact| artifact.defect = defect)
    }

    /// One display line per attached mod, with its links.
    pub fn mod_lines(&self) -> Vec<(String, Vec<(String, String)>)> {
        common!(self, g => g
            .mods
            .iter()
            .map(|m| {
                let curations = if !m.curated {
                    " · unreviewed".to_owned()
                } else if m.recommended_by.is_empty() {
                    " · curated".to_owned()
                } else {
                    format!(" · curated · ★ {}", m.recommended_by.join(", "))
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

    /// One line per version of the mod at `index`: its label, date, and the
    /// hardware it states where a conversion moved off the game's own.
    pub fn mod_version_lines(&self, index: usize) -> Vec<String> {
        fn describe(
            label: &Option<String>,
            date: &Option<missingno_gamedb::ReleaseDate>,
            hw: &str,
        ) -> String {
            let mut parts = Vec::new();
            if let Some(label) = label {
                parts.push(label.clone());
            }
            if let Some(date) = date {
                parts.push(date.as_str().to_owned());
            }
            if !hw.is_empty() {
                parts.push(hw.to_owned());
            }
            if parts.is_empty() {
                "(unlabelled)".to_owned()
            } else {
                parts.join(" · ")
            }
        }
        fn versions<P: Platform>(
            game: &Game<P>,
            index: usize,
            hardware: impl Fn(&P::ReleaseHardware) -> String,
        ) -> Vec<String> {
            game.mods
                .get(index)
                .map(|m| {
                    m.releases
                        .iter()
                        .map(|r| describe(&r.label, &r.date, &hardware(&r.hardware)))
                        .collect()
                })
                .unwrap_or_default()
        }
        match self {
            AnyGame::Vcs(g) => versions(g, index, vcs_hardware),
            AnyGame::Sg1000(g) => versions(g, index, |hw| {
                hw.cart_type
                    .map(Sg1000CartType::code)
                    .unwrap_or("")
                    .to_owned()
            }),
            AnyGame::Gb(g) => versions(g, index, |_| String::new()),
            AnyGame::Gbc(g) => versions(g, index, |_| String::new()),
        }
    }

    /// Apply edits to the named attached mod, reporting which fields landed.
    /// `None` when no mod goes by that name.
    pub fn update_mod(&mut self, name: &str, edits: ModEdits) -> Option<Vec<&'static str>> {
        common!(self, g => {
            let m = g.mods.iter_mut().find(|m| m.name == name)?;
            let touches_release = edits.touches_release();
            let mut applied = Vec::new();
            if let Some(rename) = edits.name {
                m.name = rename;
                applied.push("name");
            }
            if let Some(category) = edits.category {
                m.category = category;
                applied.push("category");
            }
            if let Some(author) = edits.author {
                m.author = (!author.is_empty()).then_some(author);
                applied.push("author");
            }
            if let Some(url) = edits.url {
                m.links.retain(|l| l.name != "Homepage");
                if !url.is_empty() {
                    m.links.push(Link {
                        name: "Homepage".to_owned(),
                        url,
                        link_type: LinkType::Community,
                        languages: Vec::new(),
                    });
                }
                applied.push("url");
            }
            match m.releases.get_mut(edits.release_index) {
                Some(release) => {
                    if let Some(base) = edits.base_sha1 {
                        release.base_sha1 = base;
                        applied.push("base_sha1");
                    }
                    if let Some(label) = edits.label {
                        release.label = (!label.is_empty()).then_some(label);
                        applied.push("label");
                    }
                    if let Some(date) = edits.date {
                        release.date = Some(date);
                        applied.push("date");
                    }
                }
                None if touches_release => {
                    applied.push("(release fields skipped: no such release_index)");
                }
                None => {}
            }
            Some(applied)
        })
    }

    /// A build that runs on a different standard than the game it patches — an
    /// NTSC conversion of a PAL cart. VCS only.
    pub fn set_mod_tv_format(&mut self, name: &str, index: usize, format: TvFormat) -> bool {
        self.mod_release(name, index, |r| r.hardware.tv_format = Some(format))
    }

    /// A conversion that swaps the controller it plays on — a joystick build of
    /// a keypad game. VCS only.
    pub fn set_mod_controllers(
        &mut self,
        name: &str,
        index: usize,
        wanted: Vec<Controller>,
    ) -> bool {
        self.mod_release(name, index, |r| r.hardware.controllers = wanted.clone())
    }

    fn mod_release(
        &mut self,
        name: &str,
        index: usize,
        edit: impl FnOnce(&mut ModRelease<missingno_gamedb::Vcs>),
    ) -> bool {
        match self {
            AnyGame::Vcs(g) => g
                .mods
                .iter_mut()
                .find(|m| m.name == name)
                .and_then(|m| m.releases.get_mut(index))
                .map(edit),
            _ => None,
        }
        .is_some()
    }

    pub fn mod_names(&self) -> Vec<String> {
        common!(self, g => g.mods.iter().map(|m| m.name.clone()).collect())
    }

    pub fn release_publisher(&self, index: usize) -> String {
        common!(self, g => g
            .releases
            .get(index)
            .and_then(|r| r.publisher.clone())
            .unwrap_or_default())
    }

    pub fn update_release(&mut self, index: usize, edits: ReleaseEdits) -> bool {
        common!(self, g => {
            let Some(release) = g.releases.get_mut(index) else {
                return false;
            };
            if let Some(regions) = edits.regions {
                release.regions = regions;
            }
            if let Some(languages) = edits.languages {
                release.languages = languages;
            }
            if let Some(status) = edits.status {
                release.status = status;
            }
            if let Some(title) = edits.title {
                release.title = (!title.is_empty()).then_some(title);
            }
            if let Some(label) = edits.label {
                release.label = (!label.is_empty()).then_some(label);
            }
            if let Some(date) = edits.date {
                release.date = date;
            }
            if let Some(publisher) = edits.publisher {
                release.publisher = (!publisher.is_empty()).then_some(publisher);
            }
            true
        })
    }

    pub fn set_release_publisher(&mut self, index: usize, value: String) {
        let publisher = (!value.is_empty()).then_some(value);
        common!(self, g => {
            if let Some(release) = g.releases.get_mut(index) {
                release.publisher = publisher;
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

    /// Board hint for the session factory (VCS and SG-1000 — their carts have
    /// no header, so the db's word must reach the core).
    pub fn cart_hint(&self) -> Option<String> {
        fn first_board<P: Platform>(
            game: &Game<P>,
            code: impl Fn(&P::ReleaseHardware) -> Option<&'static str>,
        ) -> Option<String> {
            game.releases
                .iter()
                .find_map(|r| code(&r.hardware))
                .map(str::to_owned)
        }
        match self {
            AnyGame::Vcs(g) => first_board(g, |hw| hw.cart_type.map(VcsCartType::code)),
            AnyGame::Sg1000(g) => first_board(g, |hw| hw.cart_type.map(Sg1000CartType::code)),
            _ => None,
        }
    }

    /// TV/board hints for booting one specific dump: the release that owns it
    /// speaks first; a mod's dump answers through its base's release; only
    /// then fall back to the entry's first stated values.
    pub fn hints_for(&self, sha1: &str) -> (Option<String>, Option<String>) {
        let stated = match self {
            AnyGame::Vcs(g) => release_holding(g, sha1).map(|r| {
                (
                    r.hardware
                        .tv_format
                        .map(|tv| format!("{tv:?}").to_lowercase()),
                    r.hardware.cart_type.map(|c| c.code().to_owned()),
                )
            }),
            AnyGame::Sg1000(g) => release_holding(g, sha1)
                .map(|r| (None, r.hardware.cart_type.map(|c| c.code().to_owned()))),
            _ => None,
        };
        stated.unwrap_or_else(|| (self.tv_hint(), self.cart_hint()))
    }

    /// The quality problem catalogued against one dump, wherever it hangs: a
    /// release's artifact or a mod's.
    pub fn defect_for(&self, sha1: &str) -> Option<Defect> {
        common!(self, g => g
            .releases
            .iter()
            .flat_map(|r| &r.artifacts)
            .chain(
                g.mods
                    .iter()
                    .flat_map(|m| &m.releases)
                    .flat_map(|r| &r.artifacts),
            )
            .find(|a| a.sha1.as_str() == sha1)
            .and_then(|a| a.defect))
    }

    /// The controllers the release holding this dump states — what the play
    /// pane puts in the jacks before the game boots.
    pub fn controllers_for(&self, sha1: &str) -> Vec<missingno_gamedb::platform::Controller> {
        let AnyGame::Vcs(g) = self else {
            return Vec::new();
        };
        let stated = |r: &Release<Vcs>| r.hardware.controllers.clone();
        for release in &g.releases {
            if release.artifacts.iter().any(|a| a.sha1.as_str() == sha1) {
                return stated(release);
            }
        }
        g.releases.first().map(stated).unwrap_or_default()
    }

    /// Every dump attached to the game's mods, flattened.
    pub fn mod_artifacts(&self, index: usize) -> Vec<(String, String, Option<Defect>)> {
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
                    a.defect,
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
                stage_mapper(
                    &mut release.hardware.mapper,
                    header.mapper,
                    &mut staged,
                    &mut conflicts,
                );
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
                stage_mapper(
                    &mut release.hardware.mapper,
                    header.mapper,
                    &mut staged,
                    &mut conflicts,
                );
            }
            AnyGame::Sg1000(_) | AnyGame::Vcs(_) => {}
        }
        (staged, conflicts)
    }

    /// Agent override: set the first release's board (GB/GBC) — for carts
    /// whose headers lie. An empty code hands the field back to the header.
    pub fn set_mapper(&mut self, code: &str) -> Result<(), String> {
        match self {
            AnyGame::Gb(g) => {
                let board = gb_board(code)?;
                first_release(g)?.hardware.mapper = board;
                Ok(())
            }
            AnyGame::Gbc(g) => {
                let board = gb_board(code)?;
                first_release(g)?.hardware.mapper = board;
                Ok(())
            }
            _ => Err("mapper applies to Game Boy and Game Boy Color entries only".to_owned()),
        }
    }

    /// Agent override: set the first release's board (VCS and SG-1000 — no
    /// headers). An empty code clears it back to auto-detect.
    pub fn set_cart_type(&mut self, code: &str) -> Result<(), String> {
        match self {
            AnyGame::Vcs(g) => {
                let board = vcs_board(code)?;
                first_release(g)?.hardware.cart_type = board;
                Ok(())
            }
            AnyGame::Sg1000(g) => {
                let board = sg1000_board(code)?;
                first_release(g)?.hardware.cart_type = board;
                Ok(())
            }
            _ => Err("cart_type applies to Atari VCS and SG-1000 entries only".to_owned()),
        }
    }

    /// Re-file a dump onto a mod that is already attached: another version of
    /// it, or another dump of a version it already has. A hack's second build
    /// and a bad dump of a hack are both the mod's, not the game's, and
    /// neither is a mod of its own.
    pub fn attach_dump_to_mod(
        &mut self,
        mod_name: &str,
        sha1: &str,
        as_version: bool,
        label: Option<String>,
    ) -> Result<String, String> {
        common!(self, g => {
            if !g.mods.iter().any(|m| m.name == mod_name) {
                let known: Vec<&str> = g.mods.iter().map(|m| m.name.as_str()).collect();
                return Err(format!(
                    "no mod named {mod_name:?}; attached mods: {}",
                    known.join(", ")
                ));
            }
            let mut taken = None;
            for index in 0..g.releases.len() {
                let release = &mut g.releases[index];
                if let Some(at) = release.artifacts.iter().position(|a| a.sha1.as_str() == sha1) {
                    let artifact = release.artifacts.remove(at);
                    if release.artifacts.is_empty() {
                        g.releases.remove(index);
                    }
                    taken = Some(artifact);
                    break;
                }
            }
            // Also reachable from another mod: a second build filed as a mod of
            // its own is the case this exists to undo.
            if taken.is_none() {
                'mods: for m in 0..g.mods.len() {
                    if g.mods[m].name == mod_name {
                        continue;
                    }
                    for r in 0..g.mods[m].releases.len() {
                        let release = &mut g.mods[m].releases[r];
                        if let Some(at) =
                            release.artifacts.iter().position(|a| a.sha1.as_str() == sha1)
                        {
                            let artifact = release.artifacts.remove(at);
                            if release.artifacts.is_empty() {
                                g.mods[m].releases.remove(r);
                            }
                            if g.mods[m].releases.is_empty() {
                                g.mods.remove(m);
                            }
                            taken = Some(artifact);
                            break 'mods;
                        }
                    }
                }
            }
            let Some(mut artifact) = taken else {
                return Err(format!("{sha1} is not a dump of this entry"));
            };
            let attached = g.mods.iter_mut().find(|m| m.name == mod_name).expect("checked above");
            if as_version {
                let base_sha1 = attached.releases.first().and_then(|r| r.base_sha1.clone());
                attached.releases.push(ModRelease {
                    label,
                    date: None,
                    base_sha1,
                    patch: None,
                    hardware: Default::default(),
                    artifacts: vec![artifact],
                });
                Ok(format!("{sha1} added to {mod_name:?} as a new version"))
            } else {
                artifact.label = label;
                match attached.releases.last_mut() {
                    Some(release) => release.artifacts.push(artifact),
                    None => attached.releases.push(ModRelease {
                        label: None,
                        date: None,
                        base_sha1: None,
                        patch: None,
                        hardware: Default::default(),
                        artifacts: vec![artifact],
                    }),
                }
                Ok(format!("{sha1} added to {mod_name:?} as another dump"))
            }
        })
    }

    /// Drop a release that holds nothing — a phantom left by re-filing its
    /// only dump. Refuses while it still carries dumps unless the curator
    /// explicitly discards them, so evidence never vanishes quietly.
    /// Record a release the catalogue knows shipped but holds no dump of —
    /// a rare cart whose ROM has never surfaced. Returns its index.
    pub fn add_release(&mut self) -> usize {
        common!(self, g => {
            g.releases.push(missingno_gamedb::Release {
                title: None,
                label: None,
                regions: Vec::new(),
                languages: Vec::new(),
                date: None,
                publisher: None,
                status: Default::default(),
                hardware: Default::default(),
                artifacts: Vec::new(),
            });
            g.releases.len() - 1
        })
    }

    pub fn remove_release(&mut self, index: usize, discard_dumps: bool) -> Result<(), String> {
        common!(self, g => {
            let Some(release) = g.releases.get(index) else {
                return Err(format!("no release {index}"));
            };
            if !discard_dumps && !release.artifacts.is_empty() {
                return Err(format!(
                    "release {index} still holds {} dump(s); pass discard_dumps to drop them",
                    release.artifacts.len()
                ));
            }
            g.releases.remove(index);
            Ok(())
        })
    }

    /// The broadcast standard one release shipped on (VCS only). Per-release,
    /// not per-game: one entry can hold an NTSC, a PAL and a PAL-M release.
    pub fn set_release_tv_format(&mut self, index: usize, format: TvFormat) -> bool {
        match self {
            AnyGame::Vcs(g) => g.releases.get_mut(index).map(|r| {
                r.hardware.tv_format = Some(format);
            }),
            _ => None,
        }
        .is_some()
    }

    pub fn set_release_controllers(&mut self, index: usize, controllers: Vec<Controller>) -> bool {
        match self {
            AnyGame::Vcs(g) => g.releases.get_mut(index).map(|r| {
                r.hardware.controllers = controllers;
            }),
            _ => None,
        }
        .is_some()
    }

    /// An empty code clears the override back to auto-detect.
    pub fn set_release_cart_type(&mut self, index: usize, code: &str) -> Result<(), String> {
        match self {
            AnyGame::Vcs(g) => {
                let board = vcs_board(code)?;
                release_at(g, index)?.hardware.cart_type = board;
                Ok(())
            }
            AnyGame::Sg1000(g) => {
                let board = sg1000_board(code)?;
                release_at(g, index)?.hardware.cart_type = board;
                Ok(())
            }
            _ => Err("cart_type applies to Atari VCS and SG-1000 entries only".to_owned()),
        }
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

fn first_release<P: Platform>(game: &mut Game<P>) -> Result<&mut Release<P>, String> {
    game.releases
        .first_mut()
        .ok_or_else(|| "entry has no releases".to_owned())
}

fn release_at<P: Platform>(game: &mut Game<P>, index: usize) -> Result<&mut Release<P>, String> {
    game.releases
        .get_mut(index)
        .ok_or_else(|| format!("entry has no release {index}"))
}

/// Fill in the board the header names, or record why the two disagree.
fn stage_mapper(
    stated: &mut Option<GbCartType>,
    header: Result<GbCartType, u8>,
    staged: &mut Vec<String>,
    conflicts: &mut Vec<String>,
) {
    match (*stated, header) {
        (_, Err(byte)) => conflicts.push(format!("mapper: header byte ${byte:02x} names no board")),
        (None, Ok(board)) => {
            *stated = Some(board);
            staged.push(format!("mapper: {}", board.display_name()));
        }
        (Some(current), Ok(board)) if current != board => conflicts.push(format!(
            "mapper: db {} vs header {}",
            current.display_name(),
            board.display_name()
        )),
        _ => {}
    }
}

/// The board a code names; an empty code is the field cleared. An unlisted
/// code names no board the core builds, so the refusal carries the vocabulary.
fn board_code<T>(
    code: &str,
    named: impl FnOnce(&str) -> Option<T>,
    vocabulary: impl FnOnce() -> Vec<&'static str>,
) -> Result<Option<T>, String> {
    if code.is_empty() {
        return Ok(None);
    }
    named(code).map(Some).ok_or_else(|| {
        format!(
            "unknown board code {code:?}; expected one of: {}",
            vocabulary().join(", ")
        )
    })
}

fn gb_board(code: &str) -> Result<Option<GbCartType>, String> {
    board_code(code, GbCartType::from_code, || {
        GbCartType::all().map(GbCartType::code).collect()
    })
}

fn vcs_board(code: &str) -> Result<Option<VcsCartType>, String> {
    board_code(code, VcsCartType::from_code, || {
        VcsCartType::all().map(VcsCartType::code).collect()
    })
}

fn sg1000_board(code: &str) -> Result<Option<Sg1000CartType>, String> {
    board_code(code, Sg1000CartType::from_code, || {
        Sg1000CartType::all().map(Sg1000CartType::code).collect()
    })
}

/// A JSON string → LinkType, rejecting unknowns with the valid set named.
pub fn parse_link_type(value: &str) -> Result<LinkType, String> {
    vocabulary::LINK_TYPES.parse(value)
}

/// Parse a language name for a link's `languages` list.
pub fn parse_language(value: &str) -> Result<Language, String> {
    vocabulary::LANGUAGES.parse(value)
}

pub fn parse_release_status(value: &str) -> Result<ReleaseStatus, String> {
    vocabulary::RELEASE_STATUSES.parse(value)
}

/// Parse a defect argument: a name sets it, `"None"`/`""` clears it.
pub fn parse_defect(value: &str) -> Result<Option<Defect>, String> {
    if value.is_empty() {
        return Ok(None);
    }
    vocabulary::DEFECTS.parse(value)
}

/// PAL-M is Brazil's: PAL colour on System M's 525-line/59.94 Hz raster.
pub fn parse_tv_format(value: &str) -> Result<TvFormat, String> {
    vocabulary::TV_FORMATS.parse(value)
}

/// Non-default VCS controllers; a joystick game leaves the field unset.
pub fn parse_controller(value: &str) -> Result<Controller, String> {
    vocabulary::CONTROLLERS.parse(value)
}

/// The region vocabulary is closed: unknown text is a data error, not a value.
pub fn parse_region(value: &str) -> Result<Region, String> {
    vocabulary::REGIONS
        .lookup(value)
        .ok_or_else(|| format!("unknown region {value:?}"))
}

pub fn parse_mod_category(value: &str) -> Result<ModCategory, String> {
    vocabulary::MOD_CATEGORIES.parse(value)
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

/// The starting point an unmatched dump gets: a single release holding it, with
/// nothing else stated.
fn lone_dump_entry<P: Platform>(title: String, artifact: missingno_gamedb::Artifact) -> Game<P> {
    Game {
        title,
        kind: GameKind::Game,
        developer: None,
        description: None,
        tags: Vec::new(),
        links: Vec::new(),
        covers: Vec::new(),
        screenshots: Vec::new(),
        mod_of: None,
        mods: Vec::new(),
        curated: false,
        adult: false,
        recommended_by: Vec::new(),
        releases: vec![Release {
            title: None,
            label: None,
            regions: Vec::new(),
            languages: Vec::new(),
            date: None,
            publisher: None,
            status: ReleaseStatus::Released,
            hardware: Default::default(),
            artifacts: vec![artifact],
        }],
    }
}

/// The release whose hardware describes one dump: the release holding it, or —
/// for a mod's dump — the release holding the dump that mod patches.
fn release_holding<'a, P: Platform>(game: &'a Game<P>, sha1: &str) -> Option<&'a Release<P>> {
    let holds = |r: &Release<P>| r.artifacts.iter().any(|a| a.sha1.as_str() == sha1);
    if let Some(release) = game.releases.iter().find(|r| holds(r)) {
        return Some(release);
    }
    game.mods
        .iter()
        .flat_map(|m| &m.releases)
        .find(|r| r.artifacts.iter().any(|a| a.sha1.as_str() == sha1))
        .and_then(|patched| patched.base_sha1.as_ref())
        .and_then(|base| {
            game.releases
                .iter()
                .find(|r| r.artifacts.iter().any(|a| a.sha1 == *base))
        })
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
            tags: Vec::new(),
            links: homepage
                .map(|url| {
                    vec![Link {
                        name: "Homepage".to_owned(),
                        url,
                        link_type: LinkType::Community,
                        languages: Vec::new(),
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
            curated: false,
            adult: false,
            recommended_by: Vec::new(),
            releases: vec![Release {
                title: None,
                label: None,
                regions: Vec::new(),
                languages: Vec::new(),
                date: None,
                publisher: None,
                status: release.status,
                hardware: release.hardware.clone(),
                artifacts: vec![artifact],
            }],
        });
    }
    None
}

/// Take a release out of an entry and make it an entry of its own: an import
/// that lumped two unrelated games together is undone by moving one out whole,
/// keeping its publisher, date and hardware. Mods based on a dump that leaves
/// travel with it.
fn split_game_from<P: Platform>(
    source: &mut Game<P>,
    release_index: usize,
    title: String,
) -> Result<Game<P>, String> {
    if release_index >= source.releases.len() {
        return Err(format!(
            "no release {release_index}; entry has {}",
            source.releases.len()
        ));
    }
    if source.releases.len() == 1 {
        return Err("that is the entry's only release — rename it instead".to_owned());
    }
    let release = source.releases.remove(release_index);
    let moved: Vec<&Sha1> = release.artifacts.iter().map(|a| &a.sha1).collect();
    let (mods, kept) = source.mods.drain(..).partition(|m: &Mod<P>| {
        !m.releases.is_empty()
            && m.releases.iter().all(|r| {
                r.base_sha1
                    .as_ref()
                    .is_some_and(|base| moved.contains(&base))
            })
    });
    source.mods = kept;
    Ok(Game {
        title,
        kind: GameKind::Game,
        developer: None,
        description: None,
        tags: Vec::new(),
        links: Vec::new(),
        covers: Vec::new(),
        screenshots: Vec::new(),
        mod_of: None,
        mods,
        curated: false,
        adult: source.adult,
        recommended_by: Vec::new(),
        releases: vec![release],
    })
}

/// Move `sha1` into its own release (a pre-retail build, say), inheriting the
/// source release's hardware and publisher — but not its date: a prototype's
/// date is not the retail date.
fn split_release_from<P: Platform>(
    source: &mut Game<P>,
    sha1: &str,
    status: ReleaseStatus,
    title: Option<String>,
    label: Option<String>,
    date: Option<missingno_gamedb::ReleaseDate>,
) -> bool {
    for at in 0..source.releases.len() {
        let Some(pos) = source.releases[at]
            .artifacts
            .iter()
            .position(|a| a.sha1.as_str() == sha1)
        else {
            continue;
        };
        let artifact = source.releases[at].artifacts.remove(pos);
        let hardware = source.releases[at].hardware.clone();
        let publisher = source.releases[at].publisher.clone();
        let regions = source.releases[at].regions.clone();
        let languages = source.releases[at].languages.clone();
        source.releases.push(Release {
            title,
            label,
            regions,
            languages,
            date,
            publisher,
            status,
            hardware,
            artifacts: vec![artifact],
        });
        return true;
    }
    false
}

/// Move `sha1` into an existing release; releases left with no artifacts
/// stopped describing anything and are pruned.
fn move_artifact_in<P: Platform>(
    source: &mut Game<P>,
    sha1: &str,
    to_index: usize,
) -> Result<bool, String> {
    if to_index >= source.releases.len() {
        return Err(format!(
            "no release {to_index}; entry has {}",
            source.releases.len()
        ));
    }
    let mut from = None;
    for (r, release) in source.releases.iter().enumerate() {
        if release.artifacts.iter().any(|a| a.sha1.as_str() == sha1) {
            from = Some(r);
            break;
        }
    }
    let Some(from) = from else {
        return Err(format!("{sha1} is not a release artifact of this entry"));
    };
    if from == to_index {
        return Err("artifact is already in that release".to_owned());
    }
    let pos = source.releases[from]
        .artifacts
        .iter()
        .position(|a| a.sha1.as_str() == sha1)
        .expect("found above");
    let artifact = source.releases[from].artifacts.remove(pos);
    source.releases[to_index].artifacts.push(artifact);
    let emptied = source.releases[from].artifacts.is_empty();
    if emptied {
        source.releases.remove(from);
    }
    Ok(emptied)
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
    for index in 0..source.releases.len() {
        let release = &mut source.releases[index];
        let Some(at) = release
            .artifacts
            .iter()
            .position(|a| a.sha1.as_str() == sha1)
        else {
            continue;
        };
        let artifact = release.artifacts.remove(at);
        let emptied = release.artifacts.is_empty();
        source.mods.push(Mod {
            name,
            category,
            author: None,
            curated: false,
            recommended_by: Vec::new(),
            links: homepage
                .map(|url| {
                    vec![Link {
                        name: "Homepage".to_owned(),
                        url,
                        link_type: LinkType::Community,
                        languages: Vec::new(),
                    }]
                })
                .unwrap_or_default(),
            releases: vec![ModRelease {
                label: None,
                date: None,
                base_sha1,
                patch: None,
                hardware: Default::default(),
                artifacts: vec![artifact],
            }],
        });
        // A release that held only the hack described a product that never
        // shipped — the same phantom move_artifact prunes.
        if emptied {
            source.releases.remove(index);
        }
        return true;
    }
    false
}

/// Move `from`'s releases, mods and review state into `into`, skipping dumps
/// already held and releases those dumps were the whole of. Returns what landed.
fn absorb_into<P: Platform>(into: &mut Game<P>, from: Game<P>, held: &[String]) -> (usize, usize) {
    into.curated |= from.curated;
    for id in from.recommended_by {
        if !into.recommended_by.contains(&id) {
            into.recommended_by.push(id);
        }
    }
    into.adult |= from.adult;
    let mut releases = 0;
    for mut release in from.releases {
        let had_artifacts = !release.artifacts.is_empty();
        release
            .artifacts
            .retain(|a| !held.iter().any(|s| s == a.sha1.as_str()));
        if had_artifacts && release.artifacts.is_empty() {
            continue;
        }
        into.releases.push(release);
        releases += 1;
    }
    let mut mods = 0;
    for m in from.mods {
        if into.mods.iter().any(|existing| existing.name == m.name) {
            continue;
        }
        into.mods.push(m);
        mods += 1;
    }
    (releases, mods)
}

/// Mod fields an edit may set; `None` leaves the field as it stands.
#[derive(Default)]
pub struct ModEdits {
    pub name: Option<String>,
    pub category: Option<ModCategory>,
    pub author: Option<String>,
    pub url: Option<String>,
    pub release_index: usize,
    /// Outer None leaves the base; Some(None) clears it.
    pub base_sha1: Option<Option<Sha1>>,
    pub label: Option<String>,
    pub date: Option<missingno_gamedb::ReleaseDate>,
}

impl ModEdits {
    fn touches_release(&self) -> bool {
        self.base_sha1.is_some() || self.label.is_some() || self.date.is_some()
    }
}

/// Release fields an edit may set; `None` leaves the field as it stands.
pub struct ReleaseEdits {
    pub status: Option<ReleaseStatus>,
    pub title: Option<String>,
    pub label: Option<String>,
    /// Outer None leaves the date; Some(None) clears it (an empty-string edit).
    pub date: Option<Option<missingno_gamedb::ReleaseDate>>,
    pub publisher: Option<String>,
    pub regions: Option<Vec<Region>>,
    pub languages: Option<Vec<Language>>,
}

pub struct EntryHandle {
    pub tree: TreeId,
    pub slug: String,
    pub game: AnyGame,
    pub dirty: bool,
    /// Discovered from a local ROM that matches no manifest; not on disk until
    /// curated. Distinguishes an empty starting-point record from a real entry.
    pub synthetic: bool,
}

impl EntryHandle {
    pub fn key(&self) -> String {
        format!("{}/{}", self.tree.dir(), self.slug)
    }

    /// Every normalised title the entry answers to — the game's and each of its
    /// releases' — with empties dropped.
    pub fn title_needles(&self) -> Vec<String> {
        let mut needles = vec![missingno_gamedb::normalized_title(self.game.title())];
        for release_title in self.game.release_titles() {
            needles.push(missingno_gamedb::normalized_title(&release_title));
        }
        needles.retain(|n| !n.is_empty());
        needles
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
                    synthetic: false,
                });
            }
            Ok(())
        }
        load_tree::<GameBoy>(&data_root, TreeId::Gb, AnyGame::Gb, &mut entries)?;
        load_tree::<GameBoyColor>(&data_root, TreeId::Gbc, AnyGame::Gbc, &mut entries)?;
        load_tree::<Sg1000>(&data_root, TreeId::Sg1000, AnyGame::Sg1000, &mut entries)?;
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
            .filter(|e| e.tree == tree && !e.game.curated())
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

    /// Surface local ROMs that match no manifest as empty in-memory entries —
    /// one per dump, titled from its filename — so an unknown ROM becomes a
    /// curatable starting point instead of staying invisible. Idempotent: a hash
    /// already held by any entry (including one added here) is skipped, so a
    /// re-scan adds only genuinely new ROMs. Returns how many were added.
    pub fn add_unmatched_roms(&mut self, index: &crate::verify::RomIndex) -> usize {
        let mut known: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut taken: std::collections::HashSet<String> = std::collections::HashSet::new();
        for e in &self.entries {
            known.extend(e.game.artifact_sha1s());
            for m in 0..e.game.mod_lines().len() {
                for (sha1, _, _) in e.game.mod_artifacts(m) {
                    known.insert(sha1);
                }
            }
            taken.insert(e.key());
        }
        // Stable order so slugs stay put between scans.
        let mut roms: Vec<(&String, &PathBuf)> = index
            .by_sha1
            .iter()
            .map(|(sha1, rom)| (sha1, &rom.path))
            .collect();
        roms.sort_by(|a, b| a.1.cmp(b.1));
        let mut added = 0;
        for (sha1, path) in roms {
            if known.contains(sha1) {
                continue;
            }
            // The filename is all an unmatched dump says about its console, so
            // only the two headerless platforms are read from one; the shared
            // `.bin` goes to the Atari, as current collections do.
            let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase());
            let tree = match ext.as_deref() {
                Some("sg") => TreeId::Sg1000,
                Some("a26") | Some("bin") => TreeId::Vcs,
                _ => continue,
            };
            let Ok(parsed) = sha1.parse::<Sha1>() else {
                continue;
            };
            let title = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| sha1.clone());
            let size = fs::metadata(path).ok().map(|m| m.len());
            let base = {
                let s = slugify(&title);
                if s.is_empty() {
                    format!("unmatched-{}", &sha1[..8])
                } else {
                    s
                }
            };
            let mut slug = base.clone();
            let mut n = 1;
            while taken.contains(&format!("{}/{slug}", tree.dir())) {
                n += 1;
                slug = format!("{base}-{n}");
            }
            taken.insert(format!("{}/{slug}", tree.dir()));
            known.insert(sha1.clone());
            let artifact = missingno_gamedb::Artifact {
                sha1: parsed,
                label: None,
                defect: None,
                size,
            };
            let game = match tree {
                TreeId::Sg1000 => AnyGame::Sg1000(lone_dump_entry(title, artifact)),
                TreeId::Vcs => AnyGame::Vcs(lone_dump_entry(title, artifact)),
                // The extension named one of the two headerless platforms.
                TreeId::Gb | TreeId::Gbc => continue,
            };
            self.entries.push(EntryHandle {
                tree,
                slug,
                game,
                dirty: false,
                synthetic: true,
            });
            added += 1;
        }
        added
    }

    /// A dump that turned out to be a mod. Modifications of the game — QoL,
    /// content changes, compatibility conversions, translations (a translated
    /// game is still the same game, exactly as official localizations are
    /// releases of it) — attach as mods; only total conversions get their own
    /// entry.
    pub fn mark_mod(
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
            let name = title.unwrap_or_else(|| format!("Unnamed mod of {source_title}"));
            let base = self.resolve_base(source, sha1, base_override)?;
            let attached = common!(&mut self.entries[source].game, g =>
                attach_mod(g, sha1, name.clone(), category, homepage, base));
            if !attached {
                return Err(format!("{sha1} is not an artifact of this entry"));
            }
            self.entries[source].dirty = true;
            self.write_entry(source).map_err(|e| e.to_string())?;
            return Ok(format!(
                "{} (as attached mod {name:?})",
                self.entries[source].key()
            ));
        }
        self.split_out_conversion(source, sha1, title, category, base_override, homepage)
    }

    /// The dump a mod derives from: an explicit base may be any dump the entry
    /// holds, a release's or another mod's, since a derived work can itself be
    /// derived from. Without one, a single remaining release dump is used,
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
                // An explicit "unknown" beats forcing a guessed derivation.
                if base == "none" {
                    return Ok(None);
                }
                if base == hack_sha1 {
                    return Err("base_sha1 is the hack itself".to_owned());
                }
                let mod_dumps = self.entries[source].game.mod_artifact_sha1s();
                if !candidates.contains(&base) && !mod_dumps.contains(&base) {
                    return Err(format!(
                        "base_sha1 {base} is not an artifact of this entry; artifacts: {}",
                        candidates
                            .iter()
                            .chain(mod_dumps.iter())
                            .cloned()
                            .collect::<Vec<_>>()
                            .join(", ")
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

        let slug = self.free_slug(tree, &title);

        let hack = match &mut self.entries[source].game {
            AnyGame::Gb(g) => {
                split_hack_from(g, sha1, title, category, base, homepage).map(AnyGame::Gb)
            }
            AnyGame::Gbc(g) => {
                split_hack_from(g, sha1, title, category, base, homepage).map(AnyGame::Gbc)
            }
            AnyGame::Sg1000(g) => {
                split_hack_from(g, sha1, title, category, base, homepage).map(AnyGame::Sg1000)
            }
            AnyGame::Vcs(g) => {
                split_hack_from(g, sha1, title, category, base, homepage).map(AnyGame::Vcs)
            }
        }
        .ok_or("artifact vanished mid-operation")?;

        self.entries.push(EntryHandle {
            tree,
            slug: slug.clone(),
            game: hack,
            dirty: true,
            synthetic: false,
        });
        let new_index = self.entries.len() - 1;
        self.write_entry(source).map_err(|e| e.to_string())?;
        self.write_entry(new_index).map_err(|e| e.to_string())?;
        Ok(format!("{}/{slug}", tree.dir()))
    }

    /// The inverse of `merge_entry`: one entry catalogued two different games,
    /// so a release moves out whole and becomes an entry of its own.
    pub fn split_game(
        &mut self,
        source: usize,
        release_index: usize,
        title: &str,
        slug: Option<&str>,
    ) -> Result<String, String> {
        let tree = self.entries[source].tree;
        let slug = match slug {
            Some(slug) => {
                let slug: Slug = slug.parse()?;
                if self
                    .entries
                    .iter()
                    .any(|e| e.tree == tree && e.slug == slug.as_str())
                {
                    return Err(format!("{}/{slug} already exists", tree.dir()));
                }
                slug.as_str().to_owned()
            }
            None => self.free_slug(tree, title),
        };
        let split = match &mut self.entries[source].game {
            AnyGame::Gb(g) => split_game_from(g, release_index, title.to_owned()).map(AnyGame::Gb),
            AnyGame::Gbc(g) => {
                split_game_from(g, release_index, title.to_owned()).map(AnyGame::Gbc)
            }
            AnyGame::Sg1000(g) => {
                split_game_from(g, release_index, title.to_owned()).map(AnyGame::Sg1000)
            }
            AnyGame::Vcs(g) => {
                split_game_from(g, release_index, title.to_owned()).map(AnyGame::Vcs)
            }
        }?;
        self.entries.push(EntryHandle {
            tree,
            slug: slug.clone(),
            game: split,
            dirty: true,
            synthetic: false,
        });
        let new_index = self.entries.len() - 1;
        self.entries[source].dirty = true;
        self.write_entry(source).map_err(|e| e.to_string())?;
        self.write_entry(new_index).map_err(|e| e.to_string())?;
        Ok(format!("{}/{slug}", tree.dir()))
    }

    /// A slug for `title` that no entry in the tree has taken yet.
    fn free_slug(&self, tree: TreeId, title: &str) -> String {
        let taken: std::collections::HashSet<&str> = self
            .entries
            .iter()
            .filter(|e| e.tree == tree)
            .map(|e| e.slug.as_str())
            .collect();
        let base = slugify(title);
        let mut slug = base.clone();
        let mut n = 1;
        while taken.contains(slug.as_str()) {
            n += 1;
            slug = format!("{base}-{n}");
        }
        slug
    }

    /// Rename an entry's slug: move its directory and re-point flags at the
    /// new key. The manifest's content is untouched, so curations stand.
    pub fn rename_entry(&mut self, index: usize, new_slug: &str) -> Result<String, String> {
        let new_slug: Slug = new_slug.parse()?;
        let tree = self.entries[index].tree;
        let old_slug = self.entries[index].slug.clone();
        if new_slug.as_str() == old_slug {
            return Err(format!("{} is already the slug", new_slug.as_str()));
        }
        if self
            .entries
            .iter()
            .any(|e| e.tree == tree && e.slug == new_slug.as_str())
        {
            return Err(format!("{}/{new_slug} already exists", tree.dir()));
        }
        let tree_dir = self.repo_root.join("data").join(tree.dir());
        let new_dir = tree_dir.join(new_slug.as_str());
        if new_dir.exists() {
            return Err(format!("{} already exists on disk", new_dir.display()));
        }
        let old_dir = tree_dir.join(&old_slug);
        if old_dir.exists() {
            fs::rename(&old_dir, &new_dir).map_err(|e| e.to_string())?;
            self.uncommitted += 1;
        } else {
            self.entries[index].dirty = true;
        }
        let old_key = self.entries[index].key();
        self.entries[index].slug = new_slug.as_str().to_owned();
        let new_key = self.entries[index].key();
        let mut flags_changed = false;
        for flag in &mut self.flags.flags {
            for subject in &mut flag.subject {
                if *subject == old_key {
                    *subject = new_key.clone();
                    flags_changed = true;
                }
            }
        }
        if flags_changed {
            self.save_flags().map_err(|e| e.to_string())?;
        }
        if self.entries[index].dirty {
            self.write_entry(index).map_err(|e| e.to_string())?;
        }
        Ok(new_key)
    }

    /// Fold one entry into another: the two catalogued the same game, so
    /// `source`'s releases become releases of `target` and its directory
    /// goes. Flags follow the surviving key. Returns (message, source key).
    pub fn merge_entry(&mut self, target: usize, source: usize) -> Result<String, String> {
        if target == source {
            return Err("an entry cannot absorb itself".to_owned());
        }
        if self.entries[target].tree != self.entries[source].tree {
            return Err("the two entries are in different trees".to_owned());
        }
        let source_key = self.entries[source].key();
        let target_key = self.entries[target].key();
        let source_dir = self
            .repo_root
            .join("data")
            .join(self.entries[source].tree.dir())
            .join(&self.entries[source].slug);
        // Taking the entry by value avoids deep-copying a manifest; removing
        // it first shifts every later index, target included.
        let absorbed = self.entries.remove(source);
        let target = if source < target { target - 1 } else { target };
        let (releases, mods) = self.entries[target].game.absorb(absorbed.game)?;
        self.entries[target].dirty = true;
        self.write_entry(target).map_err(|e| e.to_string())?;

        if source_dir.exists() {
            fs::remove_dir_all(&source_dir).map_err(|e| e.to_string())?;
            self.uncommitted += 1;
        }

        let mut flags_changed = false;
        for flag in &mut self.flags.flags {
            for subject in &mut flag.subject {
                if *subject == source_key {
                    *subject = target_key.clone();
                    flags_changed = true;
                }
            }
        }
        if flags_changed {
            self.save_flags().map_err(|e| e.to_string())?;
        }
        Ok(format!(
            "{source_key} merged into {target_key}: {releases} release(s), {mods} mod(s) carried over"
        ))
    }

    /// An artifact that is really its own release (prototype, beta build).
    pub fn split_release(
        &mut self,
        entry: usize,
        sha1: &str,
        status: ReleaseStatus,
        title: Option<String>,
        label: Option<String>,
        date: Option<missingno_gamedb::ReleaseDate>,
    ) -> Result<(), String> {
        // Splitting a release's only dump moves it sideways and leaves an
        // empty release behind: what the caller wants is update_release.
        for r in 0..self.entries[entry].game.release_lines().len() {
            let dumps = self.entries[entry].game.release_artifacts(r);
            if dumps.len() == 1 && dumps[0].0.eq_ignore_ascii_case(sha1) {
                return Err(format!(
                    "{sha1} is the only dump of release {r} — splitting it would leave that \
                     release empty. Use update_release to change its status, title or label."
                ));
            }
        }
        let split = common!(&mut self.entries[entry].game, g =>
            split_release_from(g, sha1, status, title, label, date));
        if !split {
            return Err(format!("{sha1} is not a release artifact of this entry"));
        }
        self.entries[entry].dirty = true;
        self.write_entry(entry).map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Entries that may be the same game as this one. Title normalisation
    /// alone misses the two commonest shapes — an entry split by slug suffix
    /// (`-ntsc`, `-pal`, `-a`) and a hack filed under its own name — so this
    /// also walks slug prefixes in both directions and reports why each
    /// candidate matched, since an adjacent slug is often a different game.
    pub fn related_entries(&self, entry: usize) -> Vec<(String, String, &'static str, bool)> {
        let this = &self.entries[entry];
        let (tree, slug) = (this.tree, this.slug.as_str());
        let needles = this.title_needles();
        let title_lower = this.game.title().to_lowercase();

        let mut out = Vec::new();
        for other in &self.entries {
            if other.tree != tree || other.slug == slug {
                continue;
            }
            let reason = if other.slug.starts_with(&format!("{slug}-")) {
                "slug suffix"
            } else if slug.starts_with(&format!("{}-", other.slug)) {
                "slug prefix"
            } else if needles.contains(&missingno_gamedb::normalized_title(other.game.title()))
                || other
                    .game
                    .release_titles()
                    .iter()
                    .any(|rt| needles.contains(&missingno_gamedb::normalized_title(rt)))
            {
                "same title"
            } else if title_lower.len() >= 4
                && other.game.title().to_lowercase().contains(&title_lower)
            {
                "title contains"
            } else {
                continue;
            };
            out.push((
                other.key(),
                other.game.title().to_owned(),
                reason,
                other.game.curated(),
            ));
        }
        out
    }

    /// Where a hash sits in the database: its entry, and whether it is a
    /// release dump or belongs to a mod.
    pub fn find_dump(&self, sha1: &str) -> Option<(String, String, String)> {
        for entry in &self.entries {
            for r in 0..entry.game.release_lines().len() {
                for (hash, label, _) in entry.game.release_artifacts(r) {
                    if hash.eq_ignore_ascii_case(sha1) {
                        let what = if label.is_empty() {
                            format!("release {r}")
                        } else {
                            format!("release {r} ({label})")
                        };
                        return Some((entry.key(), entry.game.title().to_owned(), what));
                    }
                }
            }
            for (m, name) in entry.game.mod_names().into_iter().enumerate() {
                if entry
                    .game
                    .mod_artifacts(m)
                    .iter()
                    .any(|(h, _, _)| h.eq_ignore_ascii_case(sha1))
                {
                    return Some((
                        entry.key(),
                        entry.game.title().to_owned(),
                        format!("mod \"{name}\""),
                    ));
                }
            }
        }
        None
    }

    /// Move a dump into another release; returns whether the source release
    /// was pruned (a release that only existed because of the dump).
    pub fn move_artifact(
        &mut self,
        entry: usize,
        sha1: &str,
        to_index: usize,
    ) -> Result<bool, String> {
        let emptied = common!(&mut self.entries[entry].game, g =>
            move_artifact_in(g, sha1, to_index))?;
        self.entries[entry].dirty = true;
        self.write_entry(entry).map_err(|e| e.to_string())?;
        Ok(emptied)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Load the real checkout when present: every manifest parses into the
    // curator's editing surface and the flag file round-trips.
    #[test]
    fn real_gamedb_loads() {
        let repo =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../missingno-gamedb");
        if !repo.join("data/gb").is_dir() {
            return;
        }
        let db = Db::load(repo).expect("gamedb loads");
        assert!(db.entries.len() > 8000, "{}", db.entries.len());
        // A floor far below the current backlog: curating and merging shrink
        // it every session, so a tight bound would fail on progress alone.
        assert!(db.backlog_count(TreeId::Vcs) > 1000);

        // A flag is future work, so it has to name work someone can reach:
        // every subject resolves to an entry that still exists.
        let keys: std::collections::HashSet<String> = db.entries.iter().map(|e| e.key()).collect();
        for flag in db.flags.open() {
            assert!(!flag.subject.is_empty(), "flag #{} names no entry", flag.id);
            for subject in &flag.subject {
                assert!(
                    keys.contains(subject),
                    "flag #{} points at missing entry {subject}",
                    flag.id
                );
            }
        }
    }
}

#[cfg(test)]
mod link_tests {
    use super::*;
    use missingno_gamedb::{Game, GameBoy};

    #[test]
    fn upsert_link_is_idempotent_and_updates() {
        let game = Game::<GameBoy>::from_ron(
            r#"(title: "T", releases: [(artifacts: [(sha1: "0123456789abcdef0123456789abcdef01234567")])])"#,
        )
        .unwrap();
        let mut any = AnyGame::Gb(game);
        any.upsert_link(
            "AtariAge",
            "https://atariage.com/a",
            LinkType::Community,
            Vec::new(),
        );
        any.upsert_link(
            "AtariAge",
            "https://atariage.com/a",
            LinkType::Community,
            Vec::new(),
        );
        assert_eq!(any.links().len(), 1);
        any.upsert_link(
            "AtariAge",
            "https://atariage.com/b",
            LinkType::TechnicalReference,
            Vec::new(),
        );
        assert_eq!(
            any.links(),
            vec![(
                "AtariAge".to_owned(),
                "https://atariage.com/b".to_owned(),
                String::new()
            )]
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
mod mark_mod_tests {
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
                db.mark_mod(
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
            .mark_mod(0, HACK_A, None, ModCategory::ContentChange, None, None)
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
        db.mark_mod(
            0,
            HACK_A,
            None,
            ModCategory::ContentChange,
            Some(REAL.to_owned()),
            None,
        )
        .unwrap();
        // Two dumps left (REAL, HACK_B): marking HACK_B has one candidate.
        db.mark_mod(0, HACK_B, None, ModCategory::ContentChange, None, None)
            .unwrap();
        assert_eq!(mod_bases(&db)[1], Some(REAL.to_owned()));
    }

    #[test]
    fn bogus_base_is_rejected_not_stored() {
        let (_dir, mut db) = db_with_three_dumps();
        let err = db
            .mark_mod(
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

    // A conversion of a hack patches the hack, so an already-attached mod's
    // dump is a legitimate base.
    #[test]
    fn a_mods_dump_can_be_another_mods_base() {
        let (_dir, mut db) = db_with_three_dumps();
        db.mark_mod(
            0,
            HACK_A,
            Some("hack".to_owned()),
            ModCategory::ContentChange,
            Some(REAL.to_owned()),
            None,
        )
        .unwrap();
        db.mark_mod(
            0,
            HACK_B,
            Some("conversion of the hack".to_owned()),
            ModCategory::Compatibility,
            Some(HACK_A.to_owned()),
            None,
        )
        .unwrap();
        assert_eq!(mod_bases(&db)[1], Some(HACK_A.to_owned()));
    }
}

#[cfg(test)]
mod rename_tests {
    use super::*;

    fn db_with_entry(slug: &str) -> (tempfile::TempDir, Db) {
        let dir = tempfile::tempdir().unwrap();
        let game_dir = dir.path().join("data/gb").join(slug);
        std::fs::create_dir_all(&game_dir).unwrap();
        std::fs::write(game_dir.join("manifest.ron"), "(\n    title: \"T\",\n)\n").unwrap();
        let db = Db::load(dir.path().to_path_buf()).unwrap();
        (dir, db)
    }

    #[test]
    fn rename_moves_directory_and_repoints_flags() {
        let (dir, mut db) = db_with_entry("old-name");
        db.flags.flags.push(missingno_gamedb::Flag {
            id: 1,
            kind: missingno_gamedb::FlagKind::Custom,
            subject: vec!["gb/old-name".to_owned(), "gb/other".to_owned()],
            note: "check title".to_owned(),
        });
        let new_key = db.rename_entry(0, "new-name").unwrap();
        assert_eq!(new_key, "gb/new-name");
        assert_eq!(db.entries[0].key(), "gb/new-name");
        assert!(dir.path().join("data/gb/new-name/manifest.ron").is_file());
        assert!(!dir.path().join("data/gb/old-name").exists());
        assert_eq!(
            db.flags.flags[0].subject,
            vec!["gb/new-name".to_owned(), "gb/other".to_owned()]
        );
        let saved = std::fs::read_to_string(dir.path().join("curation/flags.ron")).unwrap();
        assert!(saved.contains("gb/new-name"));
        assert!(db.uncommitted > 0);
    }

    #[test]
    fn rename_refuses_collisions_and_bad_slugs() {
        let dir = tempfile::tempdir().unwrap();
        for slug in ["old-name", "taken"] {
            let game_dir = dir.path().join("data/gb").join(slug);
            std::fs::create_dir_all(&game_dir).unwrap();
            std::fs::write(game_dir.join("manifest.ron"), "(\n    title: \"T\",\n)\n").unwrap();
        }
        let mut db = Db::load(dir.path().to_path_buf()).unwrap();
        let at = db
            .entries
            .iter()
            .position(|e| e.slug == "old-name")
            .unwrap();
        assert!(
            db.rename_entry(at, "taken")
                .unwrap_err()
                .contains("already exists")
        );
        assert!(db.rename_entry(at, "Bad Slug").is_err());
        assert!(
            db.rename_entry(at, "old-name")
                .unwrap_err()
                .contains("already")
        );
        assert_eq!(db.entries[at].key(), "gb/old-name");
        assert!(dir.path().join("data/gb/old-name/manifest.ron").is_file());
    }
}

#[cfg(test)]
mod phantom_release_tests {
    use super::*;

    const A: &str = "0123456789abcdef0123456789abcdef01234567";
    const B: &str = "89abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn marking_a_lone_hack_prunes_the_release_it_invented() {
        let game = Game::<GameBoy>::from_ron(&format!(
            "(title: \"T\", releases: [\
               (artifacts: [(sha1: \"{A}\")]),\
               (publisher: Some(\"a hacker\"), artifacts: [(sha1: \"{B}\")]),\
             ])"
        ))
        .unwrap();
        let mut game = AnyGame::Gb(game);
        let attached = match &mut game {
            AnyGame::Gb(g) => attach_mod(
                g,
                B,
                "Unnamed hack".to_owned(),
                ModCategory::ContentChange,
                None,
                None,
            ),
            _ => unreachable!(),
        };
        assert!(attached);
        // The hack's release existed only to hold it.
        assert_eq!(game.release_lines().len(), 1);
        assert_eq!(game.artifact_sha1s(), vec![A]);
    }

    #[test]
    fn a_release_keeping_other_dumps_survives() {
        let game = Game::<GameBoy>::from_ron(&format!(
            "(title: \"T\", releases: [(artifacts: [(sha1: \"{A}\"), (sha1: \"{B}\")])])"
        ))
        .unwrap();
        let mut game = AnyGame::Gb(game);
        match &mut game {
            AnyGame::Gb(g) => attach_mod(
                g,
                B,
                "Unnamed hack".to_owned(),
                ModCategory::ContentChange,
                None,
                None,
            ),
            _ => unreachable!(),
        };
        assert_eq!(game.release_lines().len(), 1);
        assert_eq!(game.artifact_sha1s(), vec![A]);
    }

    #[test]
    fn a_hacks_later_build_joins_the_mod_rather_than_forking_one() {
        let game = Game::<GameBoy>::from_ron(&format!(
            "(title: \"T\", mods: [(name: \"Deluxe\", category: ContentChange,\
                releases: [(base_sha1: Some(\"{A}\"), artifacts: [(sha1: \"{A}\")])])],\
              releases: [(artifacts: [(sha1: \"{B}\")])])"
        ))
        .unwrap();
        let mut game = AnyGame::Gb(game);
        game.attach_dump_to_mod("Deluxe", B, true, Some("8K".to_owned()))
            .unwrap();
        assert!(game.artifact_sha1s().is_empty());
        assert!(game.release_lines().is_empty());
        match &game {
            AnyGame::Gb(g) => {
                assert_eq!(g.mods.len(), 1, "no second mod invented");
                assert_eq!(g.mods[0].releases.len(), 2);
                assert_eq!(g.mods[0].releases[1].label.as_deref(), Some("8K"));
                // A version inherits what the mod is a hack of.
                assert_eq!(
                    g.mods[0].releases[1].base_sha1.as_ref().unwrap().as_str(),
                    A
                );
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn a_bad_dump_of_a_hack_joins_that_hacks_version() {
        let game = Game::<GameBoy>::from_ron(&format!(
            "(title: \"T\", mods: [(name: \"Deluxe\", category: ContentChange,\
                releases: [(artifacts: [(sha1: \"{A}\")])])],\
              releases: [(artifacts: [(sha1: \"{B}\")])])"
        ))
        .unwrap();
        let mut game = AnyGame::Gb(game);
        game.attach_dump_to_mod("Deluxe", B, false, Some("overdump".to_owned()))
            .unwrap();
        match &game {
            AnyGame::Gb(g) => {
                assert_eq!(g.mods[0].releases.len(), 1, "not a distinct version");
                assert_eq!(g.mods[0].releases[0].artifacts.len(), 2);
                assert_eq!(
                    g.mods[0].releases[0].artifacts[1].label.as_deref(),
                    Some("overdump")
                );
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn a_build_wrongly_filed_as_its_own_mod_folds_back_in() {
        let game = Game::<GameBoy>::from_ron(&format!(
            "(title: \"T\", mods: [\
               (name: \"Deluxe\", category: ContentChange, releases: [(artifacts: [(sha1: \"{A}\")])]),\
               (name: \"Deluxe (8K)\", category: ContentChange, releases: [(artifacts: [(sha1: \"{B}\")])]),\
             ])"
        ))
        .unwrap();
        let mut game = AnyGame::Gb(game);
        game.attach_dump_to_mod("Deluxe", B, true, Some("8K".to_owned()))
            .unwrap();
        match &game {
            AnyGame::Gb(g) => {
                assert_eq!(g.mods.len(), 1, "the emptied mod is gone");
                assert_eq!(g.mods[0].name, "Deluxe");
                assert_eq!(g.mods[0].releases.len(), 2);
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn attaching_names_the_mods_it_knows_when_asked_for_one_it_does_not() {
        let game = Game::<GameBoy>::from_ron(&format!(
            "(title: \"T\", mods: [(name: \"Deluxe\", category: ContentChange)],\
              releases: [(artifacts: [(sha1: \"{B}\")])])"
        ))
        .unwrap();
        let mut game = AnyGame::Gb(game);
        let err = game.attach_dump_to_mod("Typo", B, false, None).unwrap_err();
        assert!(err.contains("Deluxe"), "{err}");
        // The dump stays put when the mod is not found.
        assert_eq!(game.artifact_sha1s(), vec![B]);
    }

    #[test]
    fn remove_empty_release_refuses_while_evidence_remains() {
        let game = Game::<GameBoy>::from_ron(&format!(
            "(title: \"T\", releases: [(artifacts: [(sha1: \"{A}\")]), ()])"
        ))
        .unwrap();
        let mut game = AnyGame::Gb(game);
        assert!(game.remove_release(0, false).unwrap_err().contains("holds"));
        assert!(game.remove_release(0, true).is_ok());
        assert!(game.remove_release(0, false).is_ok());
        assert_eq!(game.release_lines().len(), 0);
        assert!(game.remove_release(9, false).is_err());
    }
}

#[cfg(test)]
mod merge_tests {
    use super::*;

    const A: &str = "0123456789abcdef0123456789abcdef01234567";
    const B: &str = "89abcdef0123456789abcdef0123456789abcdef";

    fn db_with(entries: &[(&str, &str)]) -> (tempfile::TempDir, Db) {
        let dir = tempfile::tempdir().unwrap();
        for (slug, manifest) in entries {
            let game_dir = dir.path().join("data/gb").join(slug);
            std::fs::create_dir_all(&game_dir).unwrap();
            std::fs::write(game_dir.join("manifest.ron"), manifest).unwrap();
        }
        let db = Db::load(dir.path().to_path_buf()).unwrap();
        (dir, db)
    }

    fn index_of(db: &Db, slug: &str) -> usize {
        db.entries.iter().position(|e| e.slug == slug).unwrap()
    }

    #[test]
    fn merge_carries_releases_and_deletes_the_absorbed_entry() {
        let (dir, mut db) = db_with(&[
            (
                "keeper",
                &format!(
                    "(title: \"T\", curated: true, recommended_by: [\"a\"],\
                      releases: [(artifacts: [(sha1: \"{A}\")])])"
                ),
            ),
            (
                "reissue",
                &format!(
                    "(title: \"T2\", releases: [(publisher: Some(\"CCE\"), artifacts: [(sha1: \"{B}\")])])"
                ),
            ),
        ]);
        db.flags.flags.push(missingno_gamedb::Flag {
            id: 1,
            kind: missingno_gamedb::FlagKind::Custom,
            subject: vec!["gb/reissue".to_owned()],
            note: "same game?".to_owned(),
        });
        let (keeper, reissue) = (index_of(&db, "keeper"), index_of(&db, "reissue"));
        db.merge_entry(keeper, reissue).unwrap();

        assert!(db.entries.iter().all(|e| e.slug != "reissue"));
        assert!(!dir.path().join("data/gb/reissue").exists());
        let keeper = index_of(&db, "keeper");
        assert_eq!(db.entries[keeper].game.artifact_sha1s(), vec![A, B]);
        // Editing at the curator's request preserves their endorsement.
        assert!(db.entries[keeper].game.curated());
        assert_eq!(db.flags.flags[0].subject, vec!["gb/keeper".to_owned()]);
    }

    #[test]
    fn merge_carries_the_absorbed_entrys_curated_stamps() {
        let (_dir, mut db) = db_with(&[
            (
                "original",
                &format!("(title: \"T\", releases: [(artifacts: [(sha1: \"{A}\")])])"),
            ),
            (
                "reissue",
                &format!(
                    "(title: \"T2\", curated: true, recommended_by: [\"a\"],\
                      releases: [(artifacts: [(sha1: \"{B}\")])])"
                ),
            ),
        ]);
        let (original, reissue) = (index_of(&db, "original"), index_of(&db, "reissue"));
        db.merge_entry(original, reissue).unwrap();

        let original = index_of(&db, "original");
        // The vouch follows the game into the surviving entry.
        assert!(db.entries[original].game.curated());
        assert_eq!(db.entries[original].game.recommended_by(), ["a"]);
    }

    #[test]
    fn merge_drops_dumps_the_target_already_holds() {
        let (_dir, mut db) = db_with(&[
            (
                "keeper",
                &format!("(title: \"T\", releases: [(artifacts: [(sha1: \"{A}\")])])"),
            ),
            (
                "dupe",
                &format!("(title: \"T\", releases: [(artifacts: [(sha1: \"{A}\")])])"),
            ),
        ]);
        let (keeper, dupe) = (index_of(&db, "keeper"), index_of(&db, "dupe"));
        db.merge_entry(keeper, dupe).unwrap();
        let keeper = index_of(&db, "keeper");
        // The release held nothing new, so it is not carried as an empty one.
        assert_eq!(db.entries[keeper].game.artifact_sha1s(), vec![A]);
        assert_eq!(db.entries[keeper].game.release_lines().len(), 1);
    }

    #[test]
    fn merge_refuses_itself_and_survives_a_lower_source_index() {
        let (_dir, mut db) = db_with(&[
            (
                "absorbed",
                &format!("(title: \"T\", releases: [(artifacts: [(sha1: \"{B}\")])])"),
            ),
            (
                "keeper",
                &format!("(title: \"T\", releases: [(artifacts: [(sha1: \"{A}\")])])"),
            ),
        ]);
        let keeper = index_of(&db, "keeper");
        assert!(db.merge_entry(keeper, keeper).is_err());
        let absorbed = index_of(&db, "absorbed");
        // Removing a lower-indexed source shifts the target down by one.
        db.merge_entry(keeper, absorbed).unwrap();
        let keeper = index_of(&db, "keeper");
        assert_eq!(db.entries[keeper].game.artifact_sha1s(), vec![A, B]);
    }
}

#[cfg(test)]
mod release_surgery_tests {
    use super::*;
    use missingno_gamedb::ReleaseStatus;

    fn pitfall_like() -> AnyGame {
        AnyGame::Vcs(
            Game::from_ron(
                r#"(
    title: "Pitfall!",
    curated: true,
    releases: [
        (
            regions: [Usa],
            hardware: (tv_format: Some(Ntsc), cart_type: Some("4K")),
            artifacts: [
                (sha1: "8d52548063ba852f47ae0d0d8b7f6c847bb5f5b0"),
                (sha1: "c084539e364cfb0b1c74ba55ff2dee76d5e2f36f"),
            ],
        ),
        (
            regions: [Usa],
            hardware: (tv_format: Some(Ntsc), cart_type: Some("F6")),
            artifacts: [(sha1: "a10308a3f1051068c908d1e29fd57de5b911d31d")],
        ),
    ],
)"#,
            )
            .unwrap(),
        )
    }

    #[test]
    fn split_release_leaves_retail_intact_and_skips_retail_date() {
        let mut any = pitfall_like();
        let AnyGame::Vcs(g) = &mut any else {
            unreachable!()
        };
        let ok = split_release_from(
            g,
            "c084539e364cfb0b1c74ba55ff2dee76d5e2f36f",
            ReleaseStatus::Prototype,
            Some("Jungle Runner".to_owned()),
            None,
            None,
        );
        assert!(ok);
        assert_eq!(g.releases.len(), 3);
        assert_eq!(g.releases[0].artifacts.len(), 1, "retail keeps its dump");
        let proto = &g.releases[2];
        assert_eq!(proto.status, ReleaseStatus::Prototype);
        assert_eq!(proto.title.as_deref(), Some("Jungle Runner"));
        assert_eq!(
            proto.date, None,
            "a prototype never inherits the retail date"
        );
        assert_eq!(
            proto.hardware.cart_type,
            Some(VcsCartType::Plain4K),
            "hardware inherited"
        );
    }

    #[test]
    fn moving_the_overdump_prunes_the_fabricated_release() {
        let mut any = pitfall_like();
        let AnyGame::Vcs(g) = &mut any else {
            unreachable!()
        };
        let emptied = move_artifact_in(g, "a10308a3f1051068c908d1e29fd57de5b911d31d", 0).unwrap();
        assert!(
            emptied,
            "the F6 release existed only because of the overdump"
        );
        assert_eq!(g.releases.len(), 1);
        assert_eq!(g.releases[0].artifacts.len(), 3);
        assert_eq!(g.releases[0].hardware.cart_type, Some(VcsCartType::Plain4K));
    }

    fn two_games_in_one_entry() -> AnyGame {
        AnyGame::Vcs(
            Game::from_ron(
                r#"(
    title: "Labyrinth",
    mods: [
        (
            name: "Hack of the reissue",
            category: ContentChange,
            releases: [(
                base_sha1: Some("8d52548063ba852f47ae0d0d8b7f6c847bb5f5b0"),
                artifacts: [(sha1: "b2a5f9c1e04d7d3f6c1b8e2a4d7f0c3b6e9a2d5f")],
            )],
        ),
        (
            name: "Hack of the homebrew",
            category: ContentChange,
            releases: [(
                base_sha1: Some("a10308a3f1051068c908d1e29fd57de5b911d31d"),
                artifacts: [(sha1: "c3b6e9a2d5f8b1e4d7a0c3f6b9e2d5a8c1f4b7e0")],
            )],
        ),
    ],
    releases: [
        (
            regions: [Germany],
            date: Some("1983"),
            publisher: Some("Quelle"),
            hardware: (tv_format: Some(Pal), cart_type: Some("4K")),
            artifacts: [(sha1: "8d52548063ba852f47ae0d0d8b7f6c847bb5f5b0")],
        ),
        (
            date: Some("2006"),
            publisher: Some("Bill Collins"),
            status: WorkInProgress,
            hardware: (tv_format: Some(Ntsc), cart_type: Some("4K")),
            artifacts: [(sha1: "a10308a3f1051068c908d1e29fd57de5b911d31d")],
        ),
    ],
)"#,
            )
            .unwrap(),
        )
    }

    #[test]
    fn split_game_carries_the_release_whole_and_takes_its_mods() {
        let mut any = two_games_in_one_entry();
        let AnyGame::Vcs(g) = &mut any else {
            unreachable!()
        };
        let split = split_game_from(g, 1, "Labyrinth".to_owned()).unwrap();
        assert_eq!(g.releases.len(), 1, "the rebrand stays behind");
        assert_eq!(g.releases[0].publisher.as_deref(), Some("Quelle"));
        assert_eq!(split.releases.len(), 1);
        assert_eq!(split.releases[0].publisher.as_deref(), Some("Bill Collins"));
        assert_eq!(
            split.releases[0].date.as_ref().map(ToString::to_string),
            Some("2006".to_owned())
        );
        assert_eq!(split.releases[0].status, ReleaseStatus::WorkInProgress);
        assert!(
            split.mod_of.is_none(),
            "neither game derives from the other"
        );
        assert_eq!(g.mods.len(), 1);
        assert_eq!(g.mods[0].name, "Hack of the reissue");
        assert_eq!(split.mods.len(), 1, "a mod follows the dump it patches");
        assert_eq!(split.mods[0].name, "Hack of the homebrew");
    }

    #[test]
    fn split_game_refuses_the_only_release() {
        let mut any = pitfall_like();
        let AnyGame::Vcs(g) = &mut any else {
            unreachable!()
        };
        g.releases.truncate(1);
        assert!(split_game_from(g, 0, "Anything".to_owned()).is_err());
        assert!(split_game_from(g, 4, "Anything".to_owned()).is_err());
    }
}

#[cfg(test)]
mod board_tests {
    use super::*;

    fn castle() -> AnyGame {
        AnyGame::Sg1000(
            Game::from_ron(
                r#"(
    title: "The Castle",
    releases: [(
        hardware: (cart_type: Some("CASTLE")),
        artifacts: [(sha1: "0123456789abcdef0123456789abcdef01234567")],
    )],
)"#,
            )
            .unwrap(),
        )
    }

    #[test]
    fn a_board_code_is_taken_typed_and_cleared_by_an_empty_one() {
        let mut game = castle();
        assert_eq!(game.cart_hint().as_deref(), Some("CASTLE"));
        game.set_cart_type("DAHJEE-A").unwrap();
        assert_eq!(game.cart_hint().as_deref(), Some("DAHJEE-A"));
        game.set_release_cart_type(0, "").unwrap();
        assert_eq!(game.cart_hint(), None);
    }

    #[test]
    fn a_code_from_another_platform_is_refused_with_the_vocabulary() {
        let mut game = castle();
        let error = game.set_cart_type("F6SC").unwrap_err();
        assert!(
            error.contains("\"F6SC\"") && error.contains("DAHJEE-B"),
            "{error}"
        );
        assert!(game.set_mapper("MBC3").unwrap_err().contains("Game Boy"));
        assert!(
            game.set_release_cart_type(4, "FLAT")
                .unwrap_err()
                .contains("no release 4")
        );
        assert_eq!(
            game.cart_hint().as_deref(),
            Some("CASTLE"),
            "nothing landed"
        );
    }
}
