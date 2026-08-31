//! The curator's view of the gamedb checkout: typed manifests behind a
//! platform-agnostic editing surface, plus flags and git state.

use std::{fs, io, path::PathBuf};

use missingno_core::cartridge::{
    AttributeKind, AttributeValue, BoardSpec, BoardValue, BoardVocabulary,
};
use missingno_gamedb::{
    Artifact, Defect, Enhancement, FactKind, FactValue, FlagFile, Game, GameBoy, GameBoyColor,
    GameKind, GbCartType, HardwareFacts, Language, Link, LinkType, Mod, ModCategory, ModOf,
    ModRelease, Peripheral, Platform, Region, RejectedFile, Rejection, Release, ReleaseStatus,
    Sg1000, Sha1, Slug, Tree, TvStandard, Vcs, with_platforms,
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

    pub fn for_dir(dir: &str) -> Option<Self> {
        [TreeId::Gb, TreeId::Gbc, TreeId::Sg1000, TreeId::Vcs]
            .into_iter()
            .find(|tree| tree.dir() == dir)
    }
}

/// The Game Boy platform a header names: the CGB flag's `$C0` requires a Color,
/// and everything else — dual-mode `$80` included — plays on a Game Boy.
fn gb_tree(header: &crate::verify::GbHeader) -> TreeId {
    match header.cgb_flag & 0xC0 == 0xC0 {
        true => TreeId::Gbc,
        false => TreeId::Gb,
    }
}

/// What a ROM-folder scan did to the database, for the status line.
#[derive(Default)]
pub struct ScanOutcome {
    /// Unmatched dumps that became new records.
    pub added: usize,
    /// Inbox dumps whose hash belongs to a tree other than the declared one.
    pub strays: Vec<String>,
    /// Dumps whose own header named a platform other than the declared tree.
    pub refinements: Vec<String>,
    /// Dumps passed over because a curator already turned them away.
    pub rejected: usize,
}

/// A hardware fact's description for a tool schema: every platform that states
/// the key, carrying the guidance its own descriptor gives.
pub fn fact_description(key: &str) -> String {
    let mut stated: Vec<(&'static str, &'static str)> = Vec::new();
    macro_rules! stated_by {
        ($($P:ident),* $(,)?) => {$(
            if let Some(fact) = <<$P as Platform>::ReleaseHardware as HardwareFacts>::descriptors()
                .iter()
                .find(|fact| fact.key == key)
            {
                let dir = <$P as Platform>::DIR;
                stated.push((TreeId::for_dir(dir).map_or(dir, TreeId::label), fact.doc));
            }
        )*};
    }
    with_platforms!(stated_by);
    let mut shared: Vec<(Vec<&str>, &str)> = Vec::new();
    for (platform, doc) in stated {
        match shared.iter_mut().find(|(_, shared)| *shared == doc) {
            Some((platforms, _)) => platforms.push(platform),
            None => shared.push((vec![platform], doc)),
        }
    }
    shared
        .iter()
        .map(|(platforms, doc)| format!("{}: {doc}", platforms.join(" and ")))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Every hardware fact key any platform states, in platform order, each with
/// the kind of value it takes — the union a tool surface offers and parses. A
/// key two platforms share appears once; the payload being edited answers for
/// a key its own platform doesn't state.
pub fn fact_kinds() -> Vec<(&'static str, &'static FactKind)> {
    let mut kinds: Vec<(&'static str, &'static FactKind)> = Vec::new();
    macro_rules! declared_by {
        ($($P:ident),* $(,)?) => {$(
            for fact in <<$P as Platform>::ReleaseHardware as HardwareFacts>::descriptors() {
                if !kinds.iter().any(|(key, _)| *key == fact.key) {
                    kinds.push((fact.key, &fact.kind));
                }
            }
        )*};
    }
    with_platforms!(declared_by);
    kinds
}

/// Every board catalogue stated under one fact key, with the platform stating
/// it — a key several platforms share carries one vocabulary each.
fn board_catalogues(key: &str) -> Vec<(&'static str, &'static [BoardSpec])> {
    let mut catalogues: Vec<(&'static str, &'static [BoardSpec])> = Vec::new();
    macro_rules! stated_by {
        ($($P:ident),* $(,)?) => {$(
            for fact in <<$P as Platform>::ReleaseHardware as HardwareFacts>::descriptors() {
                if fact.key == key
                    && let FactKind::Board { catalogue } = fact.kind
                {
                    let dir = <$P as Platform>::DIR;
                    catalogues.push((TreeId::for_dir(dir).map_or(dir, TreeId::label), catalogue()));
                }
            }
        )*};
    }
    with_platforms!(stated_by);
    catalogues
}

/// One property of a board statement: a part some board under this fact
/// carries, what a caller calls it, the JSON types it takes, and the values a
/// choice accepts.
pub struct BoardAttributeSchema {
    pub key: &'static str,
    pub label: &'static str,
    pub types: Vec<&'static str>,
    pub choices: Vec<&'static str>,
}

/// The parts every board stated under `key` can carry, merged across the
/// platforms that state it — the properties a board statement takes.
pub fn board_attributes(key: &str) -> Vec<BoardAttributeSchema> {
    let mut merged: Vec<BoardAttributeSchema> = Vec::new();
    for (_, catalogue) in board_catalogues(key) {
        for attribute in catalogue.iter().flat_map(|spec| spec.attributes) {
            let json_type = match attribute.kind {
                AttributeKind::Choice { .. } => "string",
                AttributeKind::Toggle => "boolean",
                AttributeKind::Bytes => "integer",
            };
            let entry = match merged.iter_mut().find(|m| m.key == attribute.key) {
                Some(entry) => entry,
                None => {
                    merged.push(BoardAttributeSchema {
                        key: attribute.key,
                        label: attribute.label,
                        types: Vec::new(),
                        choices: Vec::new(),
                    });
                    merged.last_mut().expect("just pushed")
                }
            };
            if !entry.types.contains(&json_type) {
                entry.types.push(json_type);
            }
            if let AttributeKind::Choice { names } = attribute.kind {
                for name in names {
                    if !entry.choices.contains(name) {
                        entry.choices.push(name);
                    }
                }
            }
        }
    }
    merged
}

/// Each platform's boards under `key` and the parts each carries, `?` marking a
/// part the board may go without — the vocabulary a caller names a board from.
/// Platforms sharing one vocabulary are listed together.
pub fn board_vocabulary_doc(key: &str) -> String {
    let mut shared: Vec<(Vec<&str>, String)> = Vec::new();
    for (platform, catalogue) in board_catalogues(key) {
        let boards = catalogue
            .iter()
            .map(|spec| {
                let parts: Vec<String> = spec
                    .attributes
                    .iter()
                    .map(|a| match a.optional {
                        true => format!("{}?", a.key),
                        false => a.key.to_owned(),
                    })
                    .collect();
                match parts.is_empty() {
                    true => spec.name.to_owned(),
                    false => format!("{}({})", spec.name, parts.join(", ")),
                }
            })
            .collect::<Vec<_>>()
            .join(", ");
        match shared.iter_mut().find(|(_, listed)| *listed == boards) {
            Some((platforms, _)) => platforms.push(platform),
            None => shared.push((vec![platform], boards)),
        }
    }
    shared
        .iter()
        .map(|(platforms, boards)| format!("{}: {boards}", platforms.join(" and ")))
        .collect::<Vec<_>>()
        .join(" · ")
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

/// The hardware facts a payload states, as one display string.
fn hardware_line<H: HardwareFacts>(hardware: &H) -> String {
    H::descriptors()
        .iter()
        .filter_map(|fact| match hardware.get(fact.key)? {
            FactValue::TvStandard(tv) => tv.map(|tv| format!("{tv:?}")),
            FactValue::Board(board) => {
                let FactKind::Board { catalogue } = fact.kind else {
                    return None;
                };
                board.map(|board| board_line(&board, catalogue()))
            }
            FactValue::Enhancements(stated) => term_list(stated),
            FactValue::Peripherals(stated) => term_list(stated),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// A stated list as one display string; a release stating none shows nothing.
fn term_list<T: std::fmt::Debug>(stated: Option<Vec<T>>) -> Option<String> {
    let stated = stated.filter(|list| !list.is_empty())?;
    let terms: Vec<String> = stated.iter().map(|term| format!("{term:?}")).collect();
    Some(terms.join("/"))
}

/// A stated board and the parts populated on it, in the order the board's own
/// catalogue row lists them.
fn board_line(board: &BoardValue, catalogue: &'static [BoardSpec]) -> String {
    let keys = catalogue
        .iter()
        .find(|spec| spec.name == board.board)
        .map(|spec| spec.attributes.iter().map(|a| a.key).collect::<Vec<_>>())
        .unwrap_or_default();
    let parts: Vec<String> = keys
        .iter()
        .filter_map(|key| Some((key, board.attributes.get(*key)?)))
        .filter_map(|(key, value)| match value {
            AttributeValue::Choice(name) => Some(format!("{key} {name}")),
            AttributeValue::Toggle(true) => Some((*key).to_owned()),
            AttributeValue::Toggle(false) => None,
            AttributeValue::Bytes(bytes) => Some(format!("{key} {}", byte_count(*bytes))),
        })
        .collect();
    match parts.is_empty() {
        true => board.board.clone(),
        false => format!("{} ({})", board.board, parts.join(", ")),
    }
}

/// A measured byte count, in KB where it divides evenly.
fn byte_count(bytes: u32) -> String {
    if bytes.is_multiple_of(1024) {
        format!("{}K", bytes / 1024)
    } else {
        format!("{bytes} bytes")
    }
}

fn stated_tv<H: HardwareFacts>(hardware: &H) -> Option<TvStandard> {
    match hardware.get("tv_format")? {
        FactValue::TvStandard(tv) => tv,
        _ => None,
    }
}

/// The board a payload states under its own key — the whole statement, so a
/// core that cannot read the parts off the dump gets them from the database.
fn stated_board<H: HardwareFacts>(hardware: &H) -> Option<BoardValue> {
    match hardware.get("cart_type")? {
        FactValue::Board(board) => board,
        _ => None,
    }
}

fn stated_peripherals<H: HardwareFacts>(hardware: &H) -> Vec<Peripheral> {
    match hardware.get("peripherals") {
        Some(FactValue::Peripherals(Some(stated))) => stated,
        _ => Vec::new(),
    }
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
        fn line<P: Platform>(r: &Release<P>) -> ReleaseLine {
            let extra = hardware_line(&r.hardware);
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
            if !r.languages.is_empty() {
                parts.push(
                    r.languages
                        .iter()
                        .map(|language| language.label())
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
                parts.push(extra);
            }
            ReleaseLine {
                title: r.title.clone(),
                label: r.label.clone(),
                detail: parts.join(" · "),
            }
        }
        common!(self, g => g.releases.iter().map(line).collect())
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
    pub fn stage_artifact(&mut self, sha1: &str) -> bool {
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
        self.upsert_link(
            "Wikipedia",
            &decoded_article_url(url),
            LinkType::Wiki,
            Vec::new(),
        );
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
        fn versions<P: Platform>(game: &Game<P>, index: usize) -> Vec<String> {
            game.mods
                .get(index)
                .map(|m| {
                    m.releases
                        .iter()
                        .map(|r| describe(&r.label, &r.date, &hardware_line(&r.hardware)))
                        .collect()
                })
                .unwrap_or_default()
        }
        common!(self, g => versions(g, index))
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
            if let Some(link) = edits.link {
                m.links.retain(|l| l.name != link.name);
                if !link.url.is_empty() {
                    m.links.push(link);
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

    /// What a conversion moved off the game's own hardware — the NTSC build of
    /// a PAL cart, the joystick build of a keypad game.
    pub fn set_mod_fact(
        &mut self,
        name: &str,
        index: usize,
        key: &str,
        value: FactValue,
    ) -> Result<(), String> {
        common!(self, g => {
            let release = g
                .mods
                .iter_mut()
                .find(|m| m.name == name)
                .and_then(|m| m.releases.get_mut(index))
                .ok_or_else(|| format!("mod {name:?} has no release {index}"))?;
            release.hardware.set(key, value)
        })
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

    /// Board hint for the session factory: the whole statement, so a core that
    /// cannot read the parts off the dump — or whose header lies about them —
    /// boots on the board the database names.
    pub fn cart_hint(&self) -> Option<BoardValue> {
        common!(self, g => g.releases.iter().find_map(|r| stated_board(&r.hardware)))
    }

    /// TV/board hints for booting one specific dump: the release that owns it
    /// speaks first; a mod's dump answers through its base's release; only
    /// then fall back to the entry's first stated values.
    pub fn hints_for(&self, sha1: &str) -> (Option<String>, Option<BoardValue>) {
        let stated = common!(self, g => release_holding(g, sha1).map(|r| (
            stated_tv(&r.hardware).map(|tv| tv.name().to_owned()),
            stated_board(&r.hardware),
        )));
        stated.unwrap_or_else(|| (self.tv_hint(), self.cart_hint()))
    }

    /// Console hint for the session factory, as the runner launch choice: a
    /// Game Boy release states whether it exploits a Color, and one whose
    /// stated enhancements leave it out plays on a Game Boy however its header
    /// is flagged. Unstated enhancements — and every other platform — leave the
    /// header to decide. Same precedence as [`AnyGame::hints_for`].
    pub fn runner_hint(&self, sha1: &str) -> Option<&'static str> {
        let AnyGame::Gb(game) = self else {
            return None;
        };
        let stated = match release_holding(game, sha1) {
            Some(release) => release.hardware.enhancements.as_deref(),
            None => game
                .releases
                .iter()
                .find_map(|r| r.hardware.enhancements.as_deref()),
        };
        stated.map(
            |enhancements| match enhancements.contains(&Enhancement::GameBoyColor) {
                true => "cgb",
                false => "dmg",
            },
        )
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

    /// The peripherals the release holding this dump states — what the play
    /// pane puts in the jacks before the game boots.
    pub fn peripherals_for(&self, sha1: &str) -> Vec<Peripheral> {
        common!(self, g => g
            .releases
            .iter()
            .find(|r| r.artifacts.iter().any(|a| a.sha1.as_str() == sha1))
            .or(g.releases.first())
            .map(|r| stated_peripherals(&r.hardware))
            .unwrap_or_default())
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

    /// Stage what a Game Boy header states about the dump it was read from,
    /// filling only unknown fields. Returns (staged, conflicts-with-db)
    /// descriptions.
    pub fn stage_gb_header(
        &mut self,
        header: &crate::verify::GbHeader,
        sha1: &str,
    ) -> (Vec<String>, Vec<String>) {
        let mut staged = Vec::new();
        let mut conflicts = Vec::new();
        match self {
            AnyGame::Gb(g) => {
                if header.cgb_flag == 0xC0 {
                    conflicts
                        .push("header says CGB-only, but this entry is in the gb tree".to_owned());
                }
                let Some(index) = header_release(g, sha1) else {
                    return (staged, conflicts);
                };
                let release = &mut g.releases[index];
                let mut stated = Vec::new();
                if header.sgb {
                    stated.push(Enhancement::SuperGameBoy);
                }
                if header.cgb_flag & 0x80 != 0 {
                    stated.push(Enhancement::GameBoyColor);
                }
                match &release.hardware.enhancements {
                    None => {
                        if !stated.is_empty() {
                            staged.push(format!("enhancements: {stated:?}"));
                            release.hardware.enhancements = Some(stated);
                        }
                    }
                    Some(list) if *list != stated => {
                        conflicts.push(format!("enhancements: db {list:?} vs header {stated:?}"))
                    }
                    Some(_) => {}
                }
                stage_board(
                    &mut release.hardware.cart_type,
                    header.board,
                    &mut staged,
                    &mut conflicts,
                );
            }
            AnyGame::Gbc(g) => {
                if header.cgb_flag & 0xC0 != 0xC0 {
                    conflicts.push(
                        "header does not require the CGB, but this entry is in the gbc tree"
                            .to_owned(),
                    );
                }
                let Some(index) = header_release(g, sha1) else {
                    return (staged, conflicts);
                };
                stage_board(
                    &mut g.releases[index].hardware.cart_type,
                    header.board,
                    &mut staged,
                    &mut conflicts,
                );
            }
            AnyGame::Sg1000(_) | AnyGame::Vcs(_) => {}
        }
        (staged, conflicts)
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

    /// State one hardware fact on one release. Per-release, not per-game: one
    /// entry can hold an NTSC, a PAL and a PAL-M release. The platform's own
    /// declaration answers for a key it doesn't state.
    pub fn set_release_fact(
        &mut self,
        index: usize,
        key: &str,
        value: FactValue,
    ) -> Result<(), String> {
        common!(self, g => release_at(g, index)?.hardware.set(key, value))
    }

    /// Broadcast-standard hint for the session factory.
    pub fn tv_hint(&self) -> Option<String> {
        common!(self, g => g
            .releases
            .iter()
            .find_map(|r| stated_tv(&r.hardware))
            .map(|tv| tv.name().to_owned()))
    }

    pub fn to_ron_string(&self) -> Result<String, String> {
        common!(self, g => g.to_ron_string().map_err(|e| e.to_string()))
    }

    /// The same manifest filed under another platform. Everything but hardware
    /// is platform-agnostic; a hardware fact carries over where the target
    /// declares its key and its own vocabulary accepts the value, and is
    /// reported where it does not.
    fn refiled(self, tree: TreeId) -> (AnyGame, FactMoves) {
        let mut moves = FactMoves::default();
        let game = common!(self, g => match tree {
            TreeId::Gb => AnyGame::Gb(refile(g, &mut moves)),
            TreeId::Gbc => AnyGame::Gbc(refile(g, &mut moves)),
            TreeId::Sg1000 => AnyGame::Sg1000(refile(g, &mut moves)),
            TreeId::Vcs => AnyGame::Vcs(refile(g, &mut moves)),
        });
        (game, moves)
    }
}

/// What became of the stated hardware facts when an entry changed platforms.
#[derive(Default)]
pub struct FactMoves {
    pub carried: std::collections::BTreeSet<String>,
    pub dropped: std::collections::BTreeSet<String>,
}

/// One game under another platform's hardware type.
fn refile<A: Platform, B: Platform>(game: Game<A>, moves: &mut FactMoves) -> Game<B> {
    Game {
        title: game.title,
        kind: game.kind,
        developer: game.developer,
        description: game.description,
        tags: game.tags,
        links: game.links,
        covers: game.covers,
        screenshots: game.screenshots,
        mod_of: game.mod_of,
        mods: game
            .mods
            .into_iter()
            .map(|m| Mod {
                name: m.name,
                category: m.category,
                author: m.author,
                links: m.links,
                curated: m.curated,
                recommended_by: m.recommended_by,
                releases: m
                    .releases
                    .into_iter()
                    .map(|r| ModRelease {
                        label: r.label,
                        date: r.date,
                        base_sha1: r.base_sha1,
                        patch: r.patch,
                        hardware: carried_facts(&r.hardware, moves),
                        artifacts: r.artifacts,
                    })
                    .collect(),
            })
            .collect(),
        curated: game.curated,
        adult: game.adult,
        recommended_by: game.recommended_by,
        releases: game
            .releases
            .into_iter()
            .map(|r| Release {
                title: r.title,
                label: r.label,
                regions: r.regions,
                languages: r.languages,
                date: r.date,
                publisher: r.publisher,
                status: r.status,
                hardware: carried_facts(&r.hardware, moves),
                artifacts: r.artifacts,
            })
            .collect(),
    }
}

/// The stated facts one platform's hardware carries into another's: a key the
/// target does not declare, or a value its vocabulary refuses, is reported and
/// left unstated rather than failing the move.
fn carried_facts<A: HardwareFacts, B: HardwareFacts + Default>(
    from: &A,
    moves: &mut FactMoves,
) -> B {
    let mut into = B::default();
    for fact in A::descriptors() {
        let Some(value) = from.get(fact.key) else {
            continue;
        };
        if is_unstated(&value) {
            continue;
        }
        match into.set(fact.key, value) {
            Ok(()) => moves.carried.insert(fact.key.to_owned()),
            Err(refusal) => moves.dropped.insert(format!("{}: {refusal}", fact.key)),
        };
    }
    into
}

/// Whether a fact reads as "nothing stated", which is not a value to carry.
fn is_unstated(value: &FactValue) -> bool {
    match value {
        FactValue::TvStandard(tv) => tv.is_none(),
        FactValue::Board(board) => board.is_none(),
        FactValue::Enhancements(stated) => stated.is_none(),
        FactValue::Peripherals(stated) => stated.is_none(),
    }
}

fn release_at<P: Platform>(game: &mut Game<P>, index: usize) -> Result<&mut Release<P>, String> {
    game.releases
        .get_mut(index)
        .ok_or_else(|| format!("entry has no release {index}"))
}

/// Fill in the whole board the header states, or record where it and the
/// database's statement differ — a stated board replaces the header entire, so
/// the two are compared as whole statements.
fn stage_board(
    stated: &mut Option<GbCartType>,
    header: Result<GbCartType, u8>,
    staged: &mut Vec<String>,
    conflicts: &mut Vec<String>,
) {
    match (stated.as_ref(), header) {
        (_, Err(byte)) => {
            conflicts.push(format!("cart_type: header byte ${byte:02x} names no board"))
        }
        (None, Ok(board)) => {
            staged.push(format!("cart_type: {}", board.display_name()));
            *stated = Some(board);
        }
        (Some(current), Ok(board)) if *current != board => conflicts.push(format!(
            "cart_type: db {} vs header {}",
            current.display_name(),
            board.display_name()
        )),
        _ => {}
    }
}

/// Wikipedia serves its articles under their real characters, so a percent-escaped
/// title costs the reader legibility and buys nothing. An escape is left alone only
/// where decoding it would re-punctuate the URL — a title's own `?` or `#` would
/// start a query or fragment — or produce whitespace no URL should carry.
fn decoded_article_url(url: &str) -> String {
    let structural = |byte: u8| b"#?/%".contains(&byte) || byte <= b' ' || byte == 0x7F;
    let bytes = url.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match hex_escape(&bytes[i..]) {
            Some(byte) if !structural(byte) => {
                decoded.push(byte);
                i += 3;
            }
            _ => {
                decoded.push(bytes[i]);
                i += 1;
            }
        }
    }
    String::from_utf8(decoded).unwrap_or_else(|_| url.to_owned())
}

fn hex_escape(bytes: &[u8]) -> Option<u8> {
    let [b'%', high, low, ..] = bytes else {
        return None;
    };
    let nibble = |b: u8| char::from(b).to_digit(16).map(|d| d as u8);
    Some(nibble(*high)? << 4 | nibble(*low)?)
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
pub fn parse_tv_format(value: &str) -> Result<TvStandard, String> {
    vocabulary::TV_FORMATS.parse(value)
}

/// A device stated beside the console; the platform refuses one it is not
/// played with.
pub fn parse_peripheral(value: &str) -> Result<Peripheral, String> {
    vocabulary::PERIPHERALS.parse(value)
}

/// A console variant the release exploits; the Game Boy tree is the only one
/// that states any.
pub fn parse_enhancement(value: &str) -> Result<Enhancement, String> {
    vocabulary::ENHANCEMENTS.parse(value)
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

/// The release a booted dump's header speaks for — the cart it was read from,
/// not the entry's first. A dump no release holds answers for the first, the
/// one `stage_artifact` would file it on; a mod's dump answers for nothing,
/// since a hack's header states the hack.
fn header_release<P: Platform>(game: &Game<P>, sha1: &str) -> Option<usize> {
    let holds = |artifacts: &[Artifact]| artifacts.iter().any(|a| a.sha1.as_str() == sha1);
    let patched = game
        .mods
        .iter()
        .flat_map(|m| &m.releases)
        .any(|r| holds(&r.artifacts));
    if game.releases.is_empty() || patched {
        return None;
    }
    Some(
        game.releases
            .iter()
            .position(|r| holds(&r.artifacts))
            .unwrap_or(0),
    )
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
    homepage: Option<Link>,
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
            links: homepage.map(|link| vec![link]).unwrap_or_default(),
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
        // The split is the same silicon as its source; only the product facts differ.
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
        return demote_mod_artifact(source, sha1, to_index);
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

/// Pull a dump held by one of the entry's mods back into a release — the undo
/// for a wrong mark_mod. The artifact keeps its own label and defect; the mod
/// version's base, label and date die with it. Returns whether the mod version
/// it came from was pruned.
fn demote_mod_artifact<P: Platform>(
    source: &mut Game<P>,
    sha1: &str,
    to_index: usize,
) -> Result<bool, String> {
    for m in 0..source.mods.len() {
        for r in 0..source.mods[m].releases.len() {
            let version = &mut source.mods[m].releases[r];
            let Some(at) = version
                .artifacts
                .iter()
                .position(|a| a.sha1.as_str() == sha1)
            else {
                continue;
            };
            let taken = version.artifacts.remove(at);
            let emptied = version.artifacts.is_empty();
            if emptied {
                source.mods[m].releases.remove(r);
                if source.mods[m].releases.is_empty() {
                    source.mods.remove(m);
                }
            }
            source.releases[to_index].artifacts.push(taken);
            return Ok(emptied);
        }
    }
    Err(format!("{sha1} is not a release artifact of this entry"))
}

/// Move `sha1` out of its release into a mod attached to the same game.
fn attach_mod<P: Platform>(
    source: &mut Game<P>,
    sha1: &str,
    name: String,
    category: ModCategory,
    homepage: Option<Link>,
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
            links: homepage.map(|link| vec![link]).unwrap_or_default(),
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
    /// Upserts by link name; an empty url drops that named link.
    pub link: Option<Link>,
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
    /// Dumps a curator has turned away, so a rescan never re-offers them.
    pub rejected: RejectedFile,
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
        let rejected = RejectedFile::load(&repo_root)?;
        Ok(Self {
            repo_root,
            entries,
            flags,
            rejected,
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

    pub fn save_rejected(&mut self) -> io::Result<()> {
        self.rejected.save(&self.repo_root)?;
        self.uncommitted += 1;
        Ok(())
    }

    /// Surface local ROMs that match no manifest as empty in-memory entries —
    /// one per dump, titled from its filename — so an unknown ROM becomes a
    /// curatable starting point instead of staying invisible. Idempotent: a hash
    /// already held by any entry (including one added here) is skipped, so a
    /// re-scan adds only genuinely new ROMs. Returns how many were added.
    /// New records for inbox/collection dumps no manifest holds. A declared
    /// tree files every unmatched inbox dump and recognition fills in where
    /// nothing was declared, but a dump whose own header names its platform —
    /// a Game Boy family cartridge — is filed by that header either way.
    pub fn add_unmatched_roms(
        &mut self,
        index: &crate::verify::RomIndex,
        declared: Option<TreeId>,
    ) -> ScanOutcome {
        let mut known: std::collections::HashMap<String, TreeId> = std::collections::HashMap::new();
        let mut taken: std::collections::HashSet<String> = std::collections::HashSet::new();
        for e in &self.entries {
            for sha1 in e.game.artifact_sha1s() {
                known.insert(sha1, e.tree);
            }
            for m in 0..e.game.mod_lines().len() {
                for (sha1, _, _) in e.game.mod_artifacts(m) {
                    known.insert(sha1, e.tree);
                }
            }
            taken.insert(e.key());
        }
        // Stable order so slugs stay put between scans.
        let mut roms: Vec<(&String, &crate::verify::ScannedRom)> = index.by_sha1.iter().collect();
        roms.sort_by(|a, b| a.1.path.cmp(&b.1.path));
        let mut outcome = ScanOutcome::default();
        for (sha1, scanned) in roms {
            let path = &scanned.path;
            let inbox = scanned.home == crate::verify::RomHome::Inbox;
            if self.rejected.holds(sha1) {
                outcome.rejected += 1;
                continue;
            }
            if let Some(&owner) = known.get(sha1) {
                // A declared-system inbox holding another tree's dump is a
                // stray to hand back, not work to queue.
                if inbox && declared.is_some_and(|tree| tree != owner) {
                    outcome
                        .strays
                        .push(format!("{} is {}", path.display(), owner.label()));
                }
                continue;
            }
            // The operator's declaration files an inbox dump whatever its
            // extension says; without one, only a factory whose media the
            // dump unambiguously is may claim it — `.bin` names nobody. Either
            // way the extension is never consulted for the platform.
            let rom = fs::read(path).ok();
            let header = rom.as_deref().and_then(crate::verify::gb_header);
            let tree = match declared.filter(|_| inbox) {
                Some(declared) => {
                    // A declaration names the family; a Game Boy header names
                    // which of its two platforms the dump is.
                    let tree = match (declared, &header) {
                        (TreeId::Gb | TreeId::Gbc, Some(header)) => gb_tree(header),
                        _ => declared,
                    };
                    if tree != declared {
                        outcome
                            .refinements
                            .push(format!("{} is {}", path.display(), tree.label()));
                    }
                    tree
                }
                None => {
                    let Some(rom) = rom.as_deref() else {
                        continue;
                    };
                    let Some(factory) = missingno_session::factory::factory_for(path, rom) else {
                        continue;
                    };
                    match factory.name {
                        "Game Boy" => match &header {
                            Some(header) => gb_tree(header),
                            None => continue,
                        },
                        "SG-1000" => TreeId::Sg1000,
                        "Atari VCS" => TreeId::Vcs,
                        _ => continue,
                    }
                }
            };
            let Ok(parsed) = sha1.parse::<Sha1>() else {
                continue;
            };
            let title = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| sha1.clone());
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
            known.insert(sha1.clone(), tree);
            let artifact = missingno_gamedb::Artifact {
                sha1: parsed,
                label: None,
                defect: None,
            };
            let game = match tree {
                TreeId::Gb => AnyGame::Gb(lone_dump_entry(title, artifact)),
                TreeId::Gbc => AnyGame::Gbc(lone_dump_entry(title, artifact)),
                TreeId::Sg1000 => AnyGame::Sg1000(lone_dump_entry(title, artifact)),
                TreeId::Vcs => AnyGame::Vcs(lone_dump_entry(title, artifact)),
            };
            self.entries.push(EntryHandle {
                tree,
                slug,
                game,
                dirty: false,
                synthetic: true,
            });
            outcome.added += 1;
        }
        outcome
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
        homepage: Option<Link>,
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
        homepage: Option<Link>,
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

    /// Re-file an entry under another platform: an import that guessed the
    /// wrong tree is undone by rebuilding the manifest under the target's
    /// hardware and moving its directory. A fact the target does not state is
    /// dropped and reported, so a wrong tree is never a reason to lose an entry.
    pub fn move_game(&mut self, index: usize, target: TreeId) -> Result<String, String> {
        let source_tree = self.entries[index].tree;
        let old_key = self.entries[index].key();
        if source_tree == target {
            return Err(format!("{old_key} is already in the {} tree", target.dir()));
        }
        let slug = self.entries[index].slug.clone();
        if self
            .entries
            .iter()
            .any(|e| e.tree == target && e.slug == slug)
        {
            return Err(format!("{}/{slug} already exists", target.dir()));
        }
        let data = self.repo_root.join("data");
        let source_dir = data.join(source_tree.dir()).join(&slug);
        let target_dir = data.join(target.dir()).join(&slug);
        if target_dir.exists() {
            return Err(format!("{} already exists on disk", target_dir.display()));
        }
        // Removing and re-inserting at the same position leaves every other
        // index — and anything holding one — where it was.
        let entry = self.entries.remove(index);
        let (game, moves) = entry.game.refiled(target);
        self.entries.insert(
            index,
            EntryHandle {
                tree: target,
                slug,
                game,
                dirty: true,
                synthetic: entry.synthetic,
            },
        );
        if source_dir.exists() {
            if let Some(tree_dir) = target_dir.parent() {
                fs::create_dir_all(tree_dir).map_err(|e| e.to_string())?;
            }
            fs::rename(&source_dir, &target_dir).map_err(|e| e.to_string())?;
            self.uncommitted += 1;
        }
        self.write_entry(index).map_err(|e| e.to_string())?;

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

        let listed = |facts: &std::collections::BTreeSet<String>| {
            facts
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join("; ")
        };
        let mut report = format!("{old_key} moved → {new_key}");
        if !moves.carried.is_empty() {
            report.push_str(&format!("; carried {}", listed(&moves.carried)));
        }
        if !moves.dropped.is_empty() {
            report.push_str(&format!("; dropped {}", listed(&moves.dropped)));
        }
        Ok(report)
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

    /// Turn an entry away for good: record its dumps as out of scope so a
    /// rescan cannot re-offer them, drop the flags that named it, and delete
    /// its manifest. Returns its dumps, for the caller to clear out of the
    /// inbox.
    pub fn reject_entry(&mut self, index: usize, reason: &str) -> Result<Vec<String>, String> {
        if reason.trim().is_empty() {
            return Err("a rejection states why, so it is not revisited".to_owned());
        }
        let key = self.entries[index].key();
        let title = self.entries[index].game.title().to_owned();
        let mut dumps = self.entries[index].game.artifact_sha1s();
        for m in 0..self.entries[index].game.mod_lines().len() {
            for (sha1, _, _) in self.entries[index].game.mod_artifacts(m) {
                dumps.push(sha1);
            }
        }
        let parsed = dumps
            .iter()
            .filter_map(|sha1| sha1.parse::<Sha1>().ok())
            .collect();
        self.rejected.rejected.push(Rejection {
            title,
            reason: reason.to_owned(),
            dumps: parsed,
        });
        self.save_rejected().map_err(|e| e.to_string())?;

        let before = self.flags.flags.len();
        self.flags
            .flags
            .retain(|flag| flag.subject != [key.clone()]);
        for flag in &mut self.flags.flags {
            flag.subject.retain(|subject| *subject != key);
        }
        if self.flags.flags.len() != before {
            self.save_flags().map_err(|e| e.to_string())?;
        }

        let dir = self
            .repo_root
            .join("data")
            .join(self.entries[index].tree.dir())
            .join(&self.entries[index].slug);
        self.entries.remove(index);
        if dir.exists() {
            fs::remove_dir_all(&dir).map_err(|e| e.to_string())?;
            self.uncommitted += 1;
        }
        Ok(dumps)
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
        /// Two slugs naming the same game with the punctuation in different
        /// places. Compared with every separator gone, and the import's
        /// `zzz-unk-` noise stripped, so `monster-cise` reaches
        /// `zzz-unk-monstercise-2`.
        /// Whether `needle` appears inside a bracketed part of `title` — where
        /// an import parks the real game's name — and not in the run-on list a
        /// compilation's title is.
        fn named_in_a_parenthetical(title: &str, needle: &str) -> bool {
            title.split('(').skip(1).any(|rest| {
                rest.split_once(')')
                    .map(|(inside, _)| inside.contains(needle))
                    .unwrap_or(false)
            })
        }

        fn squashed_slugs_overlap(a: &str, b: &str) -> bool {
            fn squash(s: &str) -> String {
                s.trim_start_matches("zzz-unk-")
                    .chars()
                    .filter(char::is_ascii_alphanumeric)
                    .flat_map(char::to_lowercase)
                    .collect()
            }
            let (a, b) = (squash(a), squash(b));
            let (long, short) = if a.len() >= b.len() {
                (&a, &b)
            } else {
                (&b, &a)
            };
            if short.len() < 6 || !long.starts_with(short.as_str()) {
                return false;
            }
            let tail = &long[short.len()..];
            // A dump-flag or import tail, not the next entry in a numbered
            // series: `…contest-jeffthompson-1` and `…-10` differ by a digit on
            // something already ending in one.
            !tail.is_empty()
                && tail.len() <= 2
                && !(tail.chars().all(|c| c.is_ascii_digit())
                    && short.ends_with(|c: char| c.is_ascii_digit()))
        }

        let this = &self.entries[entry];
        let (tree, slug) = (this.tree, this.slug.as_str());
        let needles = this.title_needles();
        let title_lower = this.game.title().to_lowercase();

        let mut out = Vec::new();
        for other in &self.entries {
            if other.tree != tree || other.slug == slug {
                continue;
            }
            let other_lower = other.game.title().to_lowercase();
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
            } else if title_lower.len() >= 4 && other_lower.contains(&title_lower) {
                "title contains"
            } else if other_lower.len() >= 8 && named_in_a_parenthetical(&title_lower, &other_lower)
            {
                // A filename-shaped title names the real game in a bracket:
                // "Monkey Music (Grover's Music Maker Beta) (08-18-1982)",
                // "Space Harrier (Moonsweeper Hack)". A compilation lists its
                // games instead, and listing them is not being them.
                "named in this title's brackets"
            } else if squashed_slugs_overlap(slug, &other.slug) {
                // Hyphens fall differently either side of the same word
                // (monster-cise / zzz-unk-monstercise-2), which the prefix and
                // suffix checks above both miss.
                "slug words overlap"
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

    /// Move a dump into another release, from a release or from an attached
    /// mod; returns whether the source release was pruned (one that only
    /// existed because of the dump).
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
        // Floors far below the current counts: curating and merging shrink
        // them every session, so a tight bound would fail on progress alone.
        assert!(db.entries.len() > 7000, "{}", db.entries.len());
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
mod release_line_tests {
    use super::*;

    const A: &str = "0123456789abcdef0123456789abcdef01234567";

    #[test]
    fn a_releases_languages_reach_its_display_line() {
        let game = Game::<GameBoy>::from_ron(&format!(
            "(title: \"T\", releases: [(regions: [Japan], languages: [Japanese],\
               artifacts: [(sha1: \"{A}\")])])"
        ))
        .unwrap();
        let line = &AnyGame::Gb(game).release_lines()[0];
        assert!(line.detail.contains("Japanese"), "{}", line.detail);
    }

    #[test]
    fn a_release_reading_in_no_stated_language_says_nothing() {
        let game = Game::<GameBoy>::from_ron(&format!(
            "(title: \"T\", releases: [(regions: [Japan], artifacts: [(sha1: \"{A}\")])])"
        ))
        .unwrap();
        let line = &AnyGame::Gb(game).release_lines()[0];
        // The region is not a language: "Japan" must not read as "Japanese".
        assert!(!line.detail.contains("Japanese"), "{}", line.detail);
    }
}

#[cfg(test)]
mod link_tests {
    use super::*;
    use missingno_gamedb::{Game, GameBoy};

    // An unmatched dump files by the operator's declaration; recognition only
    // fills in where nothing was declared, and `.bin` names nobody.
    #[test]
    fn unmatched_dumps_follow_the_declaration() {
        let repo =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../missingno-gamedb");
        if !repo.join("data/gb").is_dir() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let mut index = crate::verify::RomIndex::default();
        for (name, size, sha1) in [
            (
                "uncatalogued game.sg",
                8192,
                "aa00000000000000000000000000000000000001",
            ),
            (
                "generic dump.bin",
                4096,
                "aa00000000000000000000000000000000000002",
            ),
        ] {
            let path = dir.path().join(name);
            std::fs::write(&path, vec![0xA7u8; size]).unwrap();
            index.by_sha1.insert(
                sha1.to_owned(),
                crate::verify::ScannedRom {
                    path,
                    home: crate::verify::RomHome::Inbox,
                },
            );
        }
        let tree_of = |db: &Db, title: &str| {
            db.entries
                .iter()
                .find(|e| e.game.title() == title)
                .map(|e| e.tree)
        };

        let mut undeclared = Db::load(repo.clone()).expect("gamedb loads");
        let outcome = undeclared.add_unmatched_roms(&index, None);
        assert_eq!(outcome.added, 1);
        assert!(outcome.strays.is_empty());
        assert_eq!(
            tree_of(&undeclared, "uncatalogued game"),
            Some(TreeId::Sg1000)
        );
        assert_eq!(tree_of(&undeclared, "generic dump"), None);

        let mut declared = Db::load(repo).expect("gamedb loads");
        let outcome = declared.add_unmatched_roms(&index, Some(TreeId::Sg1000));
        assert_eq!(outcome.added, 2);
        assert_eq!(tree_of(&declared, "generic dump"), Some(TreeId::Sg1000));

        // A dump another tree owns is handed back, not queued as new work.
        let stray_sha1 = declared
            .entries
            .iter()
            .find(|e| e.tree == TreeId::Vcs)
            .and_then(|e| e.game.artifact_sha1s().into_iter().next())
            .expect("the VCS tree holds a dump");
        let stray = dir.path().join("stray.bin");
        std::fs::write(&stray, [0u8; 16]).unwrap();
        index.by_sha1.insert(
            stray_sha1,
            crate::verify::ScannedRom {
                path: stray,
                home: crate::verify::RomHome::Inbox,
            },
        );
        let outcome = declared.add_unmatched_roms(&index, Some(TreeId::Sg1000));
        assert_eq!(outcome.added, 0);
        assert_eq!(outcome.strays.len(), 1);
        assert!(
            outcome.strays[0].contains("stray.bin"),
            "{:?}",
            outcome.strays
        );
    }

    /// A Game Boy family dump files by its own header: the declaration names
    /// the family, the header names which of the two platforms it is, and the
    /// extension is never consulted.
    #[test]
    fn a_game_boy_header_refines_the_declared_tree() {
        let repo = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(repo.path().join("data/gb")).unwrap();
        let dumps = tempfile::tempdir().unwrap();
        let mut index = crate::verify::RomIndex::default();
        for (name, cgb_flag, sha1) in [
            (
                "colour game.gb",
                0xC0u8,
                "bb00000000000000000000000000000000000001",
            ),
            (
                "dual mode game.gbc",
                0x80,
                "bb00000000000000000000000000000000000002",
            ),
        ] {
            let mut rom = vec![0u8; 0x8000];
            rom[0x143] = cgb_flag;
            rom[0x147] = 0x01;
            let path = dumps.path().join(name);
            std::fs::write(&path, &rom).unwrap();
            index.by_sha1.insert(
                sha1.to_owned(),
                crate::verify::ScannedRom {
                    path,
                    home: crate::verify::RomHome::Inbox,
                },
            );
        }
        let tree_of = |db: &Db, title: &str| {
            db.entries
                .iter()
                .find(|e| e.game.title() == title)
                .map(|e| e.tree)
        };

        let mut declared = Db::load(repo.path().to_path_buf()).unwrap();
        let outcome = declared.add_unmatched_roms(&index, Some(TreeId::Gb));
        assert_eq!(outcome.added, 2);
        assert_eq!(tree_of(&declared, "colour game"), Some(TreeId::Gbc));
        assert_eq!(tree_of(&declared, "dual mode game"), Some(TreeId::Gb));
        assert_eq!(outcome.refinements.len(), 1);
        assert!(
            outcome.refinements[0].contains("colour game.gb"),
            "{:?}",
            outcome.refinements
        );

        // With nothing declared the family's factory claims the dump, and the
        // header still says which platform it is.
        let mut undeclared = Db::load(repo.path().to_path_buf()).unwrap();
        let outcome = undeclared.add_unmatched_roms(&index, None);
        assert_eq!(outcome.added, 2);
        assert!(outcome.refinements.is_empty());
        assert_eq!(tree_of(&undeclared, "colour game"), Some(TreeId::Gbc));
        assert_eq!(tree_of(&undeclared, "dual mode game"), Some(TreeId::Gb));
    }

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
    fn wikipedia_links_are_stored_legibly() {
        assert_eq!(
            decoded_article_url("https://ja.wikipedia.org/wiki/GB%E5%8E%9F%E4%BA%BA"),
            "https://ja.wikipedia.org/wiki/GB原人"
        );
        assert_eq!(
            decoded_article_url("https://en.wikipedia.org/wiki/Bump_%27n%27_Jump"),
            "https://en.wikipedia.org/wiki/Bump_'n'_Jump"
        );
        // A title's own `?` arrives as %3F; decoding it would start a query string.
        assert_eq!(
            decoded_article_url("https://en.wikipedia.org/wiki/Who%3F_(film)"),
            "https://en.wikipedia.org/wiki/Who%3F_(film)"
        );
        // %20 would put raw whitespace in the URL.
        assert_eq!(
            decoded_article_url("https://en.wikipedia.org/wiki/A%20B"),
            "https://en.wikipedia.org/wiki/A%20B"
        );
        assert_eq!(
            decoded_article_url("https://en.wikipedia.org/wiki/Tetris"),
            "https://en.wikipedia.org/wiki/Tetris"
        );
        // A truncated escape is left alone rather than eating the following bytes.
        assert_eq!(decoded_article_url("https://x/%E5%8"), "https://x/%E5%8");
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
    const C: &str = "fedcba9876543210fedcba9876543210fedcba98";

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
    fn a_dump_wrongly_marked_a_mod_moves_back_into_a_release() {
        let game = Game::<GameBoy>::from_ron(&format!(
            "(title: \"T\", mods: [(name: \"Deluxe\", category: ContentChange,\
                releases: [(base_sha1: Some(\"{A}\"), label: Some(\"v2\"),\
                  artifacts: [(sha1: \"{B}\", label: Some(\"alt\"))])])],\
              releases: [(artifacts: [(sha1: \"{A}\")])])"
        ))
        .unwrap();
        let mut game = AnyGame::Gb(game);
        let emptied = match &mut game {
            AnyGame::Gb(g) => move_artifact_in(g, B, 0).unwrap(),
            _ => unreachable!(),
        };
        assert!(emptied, "the mod's only version held nothing else");
        match &game {
            AnyGame::Gb(g) => {
                assert!(g.mods.is_empty(), "the emptied mod is gone");
                assert_eq!(g.releases.len(), 1);
                let moved = &g.releases[0].artifacts[1];
                assert_eq!(moved.sha1.as_str(), B);
                assert_eq!(
                    moved.label.as_deref(),
                    Some("alt"),
                    "the artifact's own label is a fact about the dump"
                );
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn pulling_one_version_back_leaves_the_mods_other_build() {
        let game = Game::<GameBoy>::from_ron(&format!(
            "(title: \"T\", mods: [(name: \"Deluxe\", category: ContentChange,\
                releases: [(artifacts: [(sha1: \"{B}\")]),\
                           (label: Some(\"8K\"), artifacts: [(sha1: \"{C}\")])])],\
              releases: [(artifacts: [(sha1: \"{A}\")])])"
        ))
        .unwrap();
        let mut game = AnyGame::Gb(game);
        match &mut game {
            AnyGame::Gb(g) => assert!(move_artifact_in(g, C, 0).unwrap()),
            _ => unreachable!(),
        }
        match &game {
            AnyGame::Gb(g) => {
                assert_eq!(g.mods.len(), 1, "the mod still has a build");
                assert_eq!(g.mods[0].releases.len(), 1);
                assert_eq!(g.mods[0].releases[0].artifacts[0].sha1.as_str(), B);
                assert_eq!(g.releases[0].artifacts.len(), 2);
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn moving_a_dump_the_entry_does_not_hold_still_fails() {
        let game = Game::<GameBoy>::from_ron(&format!(
            "(title: \"T\", mods: [(name: \"Deluxe\", category: ContentChange,\
                releases: [(artifacts: [(sha1: \"{B}\")])])],\
              releases: [(artifacts: [(sha1: \"{A}\")])])"
        ))
        .unwrap();
        let mut game = AnyGame::Gb(game);
        let err = match &mut game {
            AnyGame::Gb(g) => move_artifact_in(g, C, 0).unwrap_err(),
            _ => unreachable!(),
        };
        assert!(err.contains("not a release artifact"), "{err}");
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
    use missingno_gamedb::{ReleaseStatus, VcsCartType};

    fn pitfall_like() -> AnyGame {
        AnyGame::Vcs(
            Game::from_ron(
                r#"(
    title: "Pitfall!",
    curated: true,
    releases: [
        (
            regions: [Usa],
            hardware: (tv_format: Some(Ntsc), cart_type: Some(Plain4K)),
            artifacts: [
                (sha1: "8d52548063ba852f47ae0d0d8b7f6c847bb5f5b0"),
                (sha1: "c084539e364cfb0b1c74ba55ff2dee76d5e2f36f"),
            ],
        ),
        (
            regions: [Usa],
            hardware: (tv_format: Some(Ntsc), cart_type: Some(Atari16K)),
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
            hardware: (tv_format: Some(Pal), cart_type: Some(Plain4K)),
            artifacts: [(sha1: "8d52548063ba852f47ae0d0d8b7f6c847bb5f5b0")],
        ),
        (
            date: Some("2006"),
            publisher: Some("Bill Collins"),
            status: WorkInProgress,
            hardware: (tv_format: Some(Ntsc), cart_type: Some(Plain4K)),
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
        hardware: (cart_type: Some(CastleRam(rom: None))),
        artifacts: [(sha1: "0123456789abcdef0123456789abcdef01234567")],
    )],
)"#,
            )
            .unwrap(),
        )
    }

    fn board(name: &str) -> FactValue {
        FactValue::Board(Some(BoardValue::new(name)))
    }

    fn hinted(game: &AnyGame) -> Option<String> {
        game.cart_hint().map(|board| board.board)
    }

    #[test]
    fn a_board_name_is_taken_typed_and_cleared_by_an_empty_one() {
        let mut game = castle();
        assert_eq!(hinted(&game).as_deref(), Some("CastleRam"));
        game.set_release_fact(0, "cart_type", board("DahjeeA"))
            .unwrap();
        assert_eq!(hinted(&game).as_deref(), Some("DahjeeA"));
        game.set_release_fact(0, "cart_type", FactValue::Board(None))
            .unwrap();
        assert_eq!(game.cart_hint(), None);
    }

    #[test]
    fn a_name_from_another_platform_is_refused_with_the_vocabulary() {
        let mut game = castle();
        let error = game
            .set_release_fact(0, "cart_type", board("Atari16KSuperchip"))
            .unwrap_err();
        assert!(
            error.contains("unknown SG-1000 board") && error.contains("\"Atari16KSuperchip\""),
            "{error}"
        );
        assert!(
            game.set_release_fact(4, "cart_type", board("Flat"))
                .unwrap_err()
                .contains("no release 4")
        );
        assert_eq!(
            hinted(&game).as_deref(),
            Some("CastleRam"),
            "nothing landed"
        );
    }

    /// A key the platform doesn't state is refused by its own declaration,
    /// naming the facts it does carry.
    #[test]
    fn a_fact_another_platform_states_names_this_ones_keys() {
        let error = castle()
            .set_release_fact(
                0,
                "enhancements",
                FactValue::Enhancements(Some(vec![Enhancement::SuperGameBoy])),
            )
            .unwrap_err();
        assert!(
            error.contains("\"enhancements\"") && error.contains("cart_type"),
            "{error}"
        );
    }

    /// A colour entry is CGB-required, so it takes the link-port hardware it
    /// is played with and states no enhancement at all.
    #[test]
    fn a_gbc_entry_states_the_hardware_it_drives() {
        let mut game =
            AnyGame::Gbc(Game::from_ron("(title: \"Printed\", releases: [()])").unwrap());
        game.set_release_fact(
            0,
            "peripherals",
            FactValue::Peripherals(Some(vec![Peripheral::Printer])),
        )
        .unwrap();
        let AnyGame::Gbc(g) = &game else {
            panic!("a gbc entry")
        };
        assert_eq!(
            g.releases[0].hardware.peripherals,
            Some(vec![Peripheral::Printer])
        );
        assert!(
            game.set_release_fact(
                0,
                "enhancements",
                FactValue::Enhancements(Some(vec![Enhancement::GameBoyColor])),
            )
            .is_err()
        );
    }

    /// The VCS states the same key with its own catalogue: the controllers a
    /// jack takes, never the Game Boy's link-port hardware.
    #[test]
    fn a_vcs_entry_states_the_controllers_its_jacks_take() {
        let mut game = AnyGame::Vcs(Game::from_ron("(title: \"Knobs\", releases: [()])").unwrap());
        game.set_release_fact(
            0,
            "peripherals",
            FactValue::Peripherals(Some(vec![Peripheral::Paddle])),
        )
        .unwrap();
        assert_eq!(
            game.peripherals_for("0000000000000000000000000000000000000000"),
            vec![Peripheral::Paddle]
        );
        let refusal = game
            .set_release_fact(
                0,
                "peripherals",
                FactValue::Peripherals(Some(vec![Peripheral::Printer])),
            )
            .unwrap_err();
        assert!(refusal.contains("Printer"), "{refusal}");
    }

    /// Tool schemas describe a hardware field from the descriptors, so the
    /// platforms that state a key and their guidance travel together.
    #[test]
    fn a_fact_description_names_every_platform_that_states_it() {
        let tv = fact_description("tv_format");
        assert!(
            tv.starts_with("SG-1000: ") && tv.contains("Atari VCS: "),
            "{tv}"
        );
        assert!(tv.contains("PAL-M"), "{tv}");
        assert_eq!(
            fact_description("cart_type").split(':').next(),
            Some("Game Boy and Game Boy Color"),
        );
        assert_eq!(fact_description("no_such_fact"), "");
    }

    /// The tool surface offers every key some platform states, once each.
    #[test]
    fn the_fact_union_carries_each_key_once_with_its_kind() {
        let keys: Vec<&str> = fact_kinds().into_iter().map(|(key, _)| key).collect();
        assert_eq!(
            keys,
            ["enhancements", "peripherals", "cart_type", "tv_format"]
        );
        let kind = fact_kinds()
            .into_iter()
            .find(|(key, _)| *key == "cart_type")
            .map(|(_, kind)| kind);
        assert!(matches!(kind, Some(FactKind::Board { .. })));
    }

    /// The board properties a tool schema offers are the union of the parts
    /// every platform's boards carry, in the JSON types those parts take.
    #[test]
    fn board_attributes_merge_every_platforms_parts() {
        let parts = board_attributes("cart_type");
        let rom = parts
            .iter()
            .find(|part| part.key == "rom")
            .expect("boards state a ROM");
        // A Game Boy names its chip; an SG-1000 board is measured in bytes.
        assert_eq!(rom.types, ["string", "integer"]);
        assert!(rom.choices.contains(&"1M"), "{:?}", rom.choices);
        let battery = parts
            .iter()
            .find(|part| part.key == "battery")
            .expect("boards carry a battery");
        assert_eq!(battery.types, ["boolean"]);

        let doc = board_vocabulary_doc("cart_type");
        assert!(doc.contains("Mbc5(rom, ram?, battery?, rumble?)"), "{doc}");
        assert!(doc.contains("Atari VCS: "), "{doc}");
    }

    /// A stated board reads as its own name plus the parts on it, in the order
    /// its catalogue row lists them.
    #[test]
    fn a_board_line_names_the_board_and_its_parts() {
        let mbc5 = BoardValue::new("Mbc5")
            .with_choice("rom", "1M")
            .with_choice("ram", "32K")
            .with_toggle("battery", true)
            .with_toggle("rumble", true);
        assert_eq!(
            board_line(&mbc5, <GbCartType as BoardVocabulary>::catalogue()),
            "Mbc5 (rom 1M, ram 32K, battery, rumble)"
        );
        let measured = BoardValue::new("DahjeeA").with("rom", AttributeValue::Bytes(49152));
        assert_eq!(
            board_line(
                &measured,
                <missingno_gamedb::Sg1000CartType as BoardVocabulary>::catalogue()
            ),
            "DahjeeA (rom 48K)"
        );
        assert_eq!(
            board_line(
                &BoardValue::new("Plain4K"),
                <missingno_gamedb::VcsCartType as BoardVocabulary>::catalogue()
            ),
            "Plain4K"
        );
    }
}

#[cfg(test)]
mod move_tests {
    use super::*;

    const COLOUR_GAME: &str = "(title: \"Colour Game\", releases: [(hardware: (enhancements: \
         [SuperGameBoy, GameBoyColor], cart_type: Some(Mbc5(rom: Mb1, ram: \
         Some(Kb32), battery: true, \
         rumble: false))))])\n";

    fn repo_with(entries: &[(&str, &str, &str)]) -> tempfile::TempDir {
        let repo = tempfile::tempdir().unwrap();
        for (tree, slug, manifest) in entries {
            let dir = repo.path().join("data").join(tree).join(slug);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("manifest.ron"), manifest).unwrap();
        }
        repo
    }

    #[test]
    fn a_moved_game_keeps_what_the_target_states_and_reports_the_rest() {
        let repo = repo_with(&[("gb", "colour-game", COLOUR_GAME)]);
        let mut db = Db::load(repo.path().to_path_buf()).unwrap();
        let report = db.move_game(0, TreeId::Gbc).unwrap();

        assert!(
            report.starts_with("gb/colour-game moved → gbc/colour-game"),
            "{report}"
        );
        assert!(report.contains("carried cart_type"), "{report}");
        assert!(report.contains("enhancements"), "{report}");
        assert_eq!(db.entries[0].tree, TreeId::Gbc);
        assert!(matches!(db.entries[0].game, AnyGame::Gbc(_)));
        assert_eq!(
            db.entries[0].game.cart_hint().map(|board| board.board),
            Some("Mbc5".to_owned())
        );
        assert!(!repo.path().join("data/gb/colour-game").exists());
        assert!(
            repo.path()
                .join("data/gbc/colour-game/manifest.ron")
                .exists()
        );
    }

    #[test]
    fn a_slug_the_target_tree_already_holds_refuses_the_move() {
        let repo = repo_with(&[
            ("gb", "colour-game", COLOUR_GAME),
            ("gbc", "colour-game", "(title: \"Colour Game\")\n"),
        ]);
        let mut db = Db::load(repo.path().to_path_buf()).unwrap();
        let source = db
            .entries
            .iter()
            .position(|e| e.tree == TreeId::Gb)
            .unwrap();
        let error = db.move_game(source, TreeId::Gbc).unwrap_err();
        assert!(error.contains("gbc/colour-game already exists"), "{error}");
        assert_eq!(db.entries[source].tree, TreeId::Gb);
        assert!(db.move_game(source, TreeId::Gb).is_err(), "already there");
    }
}

#[cfg(test)]
mod header_tests {
    use super::*;

    /// A Game Boy image whose header carries `cgb_flag` at $0143 and declares
    /// an MBC5 with a 1 MB ROM and a 32 KB RAM chip.
    fn rom(cgb_flag: u8) -> Vec<u8> {
        let mut rom = vec![0; 0x100000];
        rom[0x143] = cgb_flag;
        rom[0x147] = 0x1b;
        rom[0x148] = 0x05;
        rom[0x149] = 0x03;
        rom
    }

    /// A hash no release holds, so staging falls back to the first release.
    const UNHELD: &str = "0000000000000000000000000000000000000000";

    fn gb_entry(hardware: &str) -> AnyGame {
        AnyGame::Gb(
            Game::from_ron(&format!(
                "(title: \"Header\", releases: [(hardware: ({hardware}))])"
            ))
            .unwrap(),
        )
    }

    #[test]
    fn the_cgb_flag_names_the_platform_dual_mode_included() {
        let tree = |flag| gb_tree(&crate::verify::gb_header(&rom(flag)).unwrap());
        assert_eq!(tree(0x00), TreeId::Gb);
        assert_eq!(tree(0x80), TreeId::Gb);
        assert_eq!(tree(0xC0), TreeId::Gbc);
    }

    #[test]
    fn an_unstated_board_takes_the_whole_header_statement() {
        let mut game = gb_entry("");
        let header = crate::verify::gb_header(&rom(0x00)).unwrap();
        let (staged, conflicts) = game.stage_gb_header(&header, UNHELD);
        assert!(conflicts.is_empty(), "{conflicts:?}");
        assert!(
            staged
                .iter()
                .any(|line| line == "cart_type: MBC5 (1M, RAM 32K, battery)"),
            "{staged:?}"
        );
        let board = game.cart_hint().expect("the header stated a board");
        assert_eq!(board.board, "Mbc5");
        assert_eq!(
            board.attributes.get("ram"),
            Some(&AttributeValue::Choice("32K".to_owned()))
        );
    }

    /// A stated board is a whole statement, so a part the header disagrees
    /// about is a conflict for the curator, not a silent overwrite.
    #[test]
    fn a_stated_board_that_differs_in_one_part_conflicts() {
        let mut game = gb_entry(
            "cart_type: Some(Mbc5(rom: Mb1, ram: Some(Kb8), battery: true, rumble: false))",
        );
        let header = crate::verify::gb_header(&rom(0x00)).unwrap();
        let (staged, conflicts) = game.stage_gb_header(&header, UNHELD);
        assert!(staged.iter().all(|line| !line.starts_with("cart_type")));
        assert!(
            conflicts
                .iter()
                .any(|line| line.starts_with("cart_type: db MBC5 (1M, RAM 8K, battery)")),
            "{conflicts:?}"
        );
    }

    /// The header states the cart it was read from, so a dump held by the
    /// second release answers for that release and leaves the first alone.
    #[test]
    fn the_header_stages_onto_the_release_holding_the_dump() {
        let dump = "1e475cbd5fb7099df91155a103698e4e66da7a86";
        let mut game = AnyGame::Gb(
            Game::from_ron(&format!(
                "(title: \"Header\", releases: [(regions: [Japan]), (regions: [Usa], \
                 artifacts: [(sha1: \"{dump}\")])])"
            ))
            .unwrap(),
        );
        let header = crate::verify::gb_header(&rom(0x00)).unwrap();
        let (staged, conflicts) = game.stage_gb_header(&header, dump);
        assert!(conflicts.is_empty(), "{conflicts:?}");
        assert!(
            staged.iter().any(|line| line.starts_with("cart_type")),
            "{staged:?}"
        );
        let AnyGame::Gb(g) = &game else {
            panic!("a gb entry")
        };
        assert_eq!(g.releases[0].hardware.cart_type, None);
        assert!(g.releases[1].hardware.cart_type.is_some());
    }

    /// A stated list is the whole statement, so a header naming another
    /// console is reported rather than folded in.
    #[test]
    fn a_stated_list_the_header_disagrees_with_is_a_conflict() {
        let mut game = gb_entry("enhancements: [SuperGameBoy]");
        let header = crate::verify::gb_header(&rom(0x80)).unwrap();
        let (staged, conflicts) = game.stage_gb_header(&header, UNHELD);
        assert!(
            staged.iter().all(|line| !line.starts_with("enhancements")),
            "{staged:?}"
        );
        assert!(
            conflicts
                .iter()
                .any(|line| line.starts_with("enhancements: db [SuperGameBoy]")),
            "{conflicts:?}"
        );
        let AnyGame::Gb(g) = &game else {
            panic!("a gb entry")
        };
        assert_eq!(
            g.releases[0].hardware.enhancements,
            Some(vec![Enhancement::SuperGameBoy])
        );
    }

    /// A release stated as exploiting nothing keeps that across a boot; only an
    /// unstated list is the header's to fill.
    #[test]
    fn a_cleared_list_survives_the_header_and_an_unstated_one_is_filled() {
        let header = crate::verify::gb_header(&rom(0x80)).unwrap();

        let mut cleared = gb_entry("enhancements: []");
        let (staged, conflicts) = cleared.stage_gb_header(&header, UNHELD);
        assert!(
            staged.iter().all(|line| !line.starts_with("enhancements")),
            "{staged:?}"
        );
        assert!(
            conflicts
                .iter()
                .any(|line| line.starts_with("enhancements: db []")),
            "{conflicts:?}"
        );
        let AnyGame::Gb(g) = &cleared else {
            panic!("a gb entry")
        };
        assert_eq!(g.releases[0].hardware.enhancements, Some(Vec::new()));

        let mut unstated = gb_entry("cart_type: None");
        let (staged, _) = unstated.stage_gb_header(&header, UNHELD);
        assert!(
            staged.iter().any(|line| line.starts_with("enhancements")),
            "{staged:?}"
        );
        let AnyGame::Gb(g) = &unstated else {
            panic!("a gb entry")
        };
        assert_eq!(
            g.releases[0].hardware.enhancements,
            Some(vec![Enhancement::GameBoyColor])
        );
    }

    /// The peripherals are box facts, so a boot never touches them.
    #[test]
    fn a_boot_leaves_the_peripherals_alone() {
        let header = crate::verify::gb_header(&rom(0x80)).unwrap();
        let mut game = gb_entry("peripherals: [Printer]");
        let (staged, conflicts) = game.stage_gb_header(&header, UNHELD);
        assert!(
            staged
                .iter()
                .chain(&conflicts)
                .all(|line| !line.starts_with("peripherals")),
            "{staged:?} {conflicts:?}"
        );
        let AnyGame::Gb(g) = &game else {
            panic!("a gb entry")
        };
        assert_eq!(
            g.releases[0].hardware.peripherals,
            Some(vec![Peripheral::Printer])
        );
    }

    /// Dual-mode media boots enhanced but is a Game Boy cartridge, so filing
    /// one in the gbc tree is the mistake the check exists to catch.
    #[test]
    fn a_gbc_entry_wants_a_header_that_requires_the_colour_console() {
        let staged = |flag| {
            AnyGame::Gbc(Game::from_ron("(title: \"Header\", releases: [()])").unwrap())
                .stage_gb_header(&crate::verify::gb_header(&rom(flag)).unwrap(), UNHELD)
                .1
        };
        assert!(staged(0xC0).is_empty());
        for flag in [0x00, 0x80] {
            assert!(
                staged(flag).iter().any(|line| line.contains("gbc tree")),
                "${flag:02x}"
            );
        }
    }
}

#[cfg(test)]
mod runner_hint_tests {
    use super::*;

    const HELD: &str = "1e475cbd5fb7099df91155a103698e4e66da7a86";
    const UNHELD: &str = "0000000000000000000000000000000000000000";

    fn gb_entry(hardware: &str) -> AnyGame {
        AnyGame::Gb(
            Game::from_ron(&format!(
                "(title: \"Runner\", releases: [(hardware: ({hardware}), artifacts: [(sha1: \
                 \"{HELD}\")])])"
            ))
            .unwrap(),
        )
    }

    #[test]
    fn a_stated_enhancement_list_names_the_console_the_release_drives() {
        assert_eq!(
            gb_entry("enhancements: [GameBoyColor]").runner_hint(HELD),
            Some("cgb")
        );
        assert_eq!(gb_entry("enhancements: []").runner_hint(HELD), Some("dmg"));
        assert_eq!(
            gb_entry("enhancements: [SuperGameBoy]").runner_hint(HELD),
            Some("dmg")
        );
    }

    #[test]
    fn unstated_enhancements_leave_the_console_to_the_header() {
        assert_eq!(gb_entry("").runner_hint(HELD), None);
        assert_eq!(
            gb_entry("cart_type: Some(Mbc1(rom: Kb512, ram: None, battery: false))")
                .runner_hint(HELD),
            None
        );
        assert_eq!(gb_entry("peripherals: [Printer]").runner_hint(HELD), None);
    }

    /// The release holding the dump speaks first; a dump no release holds
    /// falls back to the first stated list, as the other hints do.
    #[test]
    fn the_release_holding_the_dump_states_its_own_console() {
        let game = AnyGame::Gb(
            Game::from_ron(&format!(
                "(title: \"Runner\", releases: [(hardware: (enhancements: \
                 [GameBoyColor])), (hardware: (enhancements: []), artifacts: [(sha1: \
                 \"{HELD}\")])])"
            ))
            .unwrap(),
        );
        assert_eq!(game.runner_hint(HELD), Some("dmg"));
        assert_eq!(game.runner_hint(UNHELD), Some("cgb"));
    }

    /// A Color entry states no enhancements, and its media requires the Color
    /// anyway, so it leaves the choice where it was.
    #[test]
    fn a_colour_entry_states_no_console() {
        let game = AnyGame::Gbc(Game::from_ron("(title: \"Runner\", releases: [()])").unwrap());
        assert_eq!(game.runner_hint(HELD), None);
    }
}

#[cfg(test)]
mod rejection_tests {
    use super::*;

    /// A rejection outlives the entry: the dumps stay recorded, so the next
    /// scan passes them over instead of offering them as new records.
    #[test]
    fn a_rejected_entrys_dumps_never_come_back() {
        let dump = "1e475cbd5fb7099df91155a103698e4e66da7a86";
        let dir = tempfile::tempdir().unwrap();
        let mut db = Db {
            repo_root: dir.path().to_owned(),
            entries: vec![EntryHandle {
                tree: TreeId::Gb,
                slug: "cheat-cart".to_owned(),
                game: AnyGame::Gb(
                    Game::from_ron(&format!(
                        "(title: \"Cheat Cart\", releases: [(artifacts: [(sha1: \"{dump}\")])])"
                    ))
                    .unwrap(),
                ),
                dirty: false,
                synthetic: false,
            }],
            flags: Default::default(),
            rejected: Default::default(),
            uncommitted: 0,
        };
        assert_eq!(db.reject_entry(0, "accessory firmware").unwrap(), [dump]);
        assert!(db.entries.is_empty());
        assert!(db.rejected.holds(dump));

        let mut index = crate::verify::RomIndex::default();
        let path = dir.path().join("cheat.gb");
        std::fs::write(&path, [0u8; 16]).unwrap();
        index.by_sha1.insert(
            dump.to_owned(),
            crate::verify::ScannedRom {
                path,
                home: crate::verify::RomHome::Inbox,
            },
        );
        let outcome = db.add_unmatched_roms(&index, Some(TreeId::Gb));
        assert_eq!((outcome.added, outcome.rejected), (0, 1));
        assert!(db.entries.is_empty(), "{:?}", db.entries.len());
    }
}

#[cfg(test)]
mod related_entries_tests {
    use super::*;

    fn db_with(entries: &[(&str, &str)]) -> (tempfile::TempDir, Db) {
        let dir = tempfile::tempdir().unwrap();
        for (slug, title) in entries {
            let game_dir = dir.path().join("data/vcs").join(slug);
            std::fs::create_dir_all(&game_dir).unwrap();
            std::fs::write(
                game_dir.join("manifest.ron"),
                format!("(\n    title: {title:?},\n)\n"),
            )
            .unwrap();
        }
        let db = Db::load(dir.path().to_path_buf()).unwrap();
        (dir, db)
    }

    fn keys_related_to(db: &Db, slug: &str) -> Vec<String> {
        let i = db.entries.iter().position(|e| e.slug == slug).unwrap();
        db.related_entries(i).into_iter().map(|(k, ..)| k).collect()
    }

    /// The import titles an entry from a dump's filename, so the real game's
    /// title is a substring of it rather than the other way round.
    #[test]
    fn a_filename_title_reaches_the_game_named_inside_it() {
        let (_dir, db) = db_with(&[
            (
                "monkey-music",
                "Monkey Music (Grover's Music Maker Beta) (Kid's Controller) (08-18-1982)",
            ),
            ("grovers-music-maker", "Grover's Music Maker"),
        ]);
        assert_eq!(
            keys_related_to(&db, "monkey-music"),
            vec!["vcs/grovers-music-maker"]
        );
    }

    /// Neither slug is the other's prefix or suffix: the hyphen falls inside
    /// the word on one side and the other carries the import's own noise.
    #[test]
    fn slugs_hyphenated_differently_still_meet() {
        let (_dir, db) = db_with(&[
            ("monster-cise", "Monster Cise"),
            ("zzz-unk-monstercise-2", "ZZZ-UNK-Monstercise_2"),
        ]);
        assert_eq!(
            keys_related_to(&db, "monster-cise"),
            vec!["vcs/zzz-unk-monstercise-2"]
        );
    }

    /// A different game that merely shares a couple of words stays out.
    #[test]
    fn a_shared_word_is_not_a_relation() {
        let (_dir, db) = db_with(&[
            ("music-maker", "Music Maker"),
            ("moon-patrol", "Moon Patrol"),
        ]);
        assert!(keys_related_to(&db, "moon-patrol").is_empty());
    }
}
