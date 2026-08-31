//! The closed vocabularies the curator speaks: each one listed once, and read
//! both by the tool surface — as a JSON-schema `enum` — and by the parser that
//! turns an agent's word back into a database value. A term the schema offers
//! is a term the parser accepts, by construction.

use missingno_gamedb::{
    Defect, Enhancement, GameKind, Language, LinkType, ModCategory, Peripheral, Region,
    ReleaseStatus, TvStandard,
};
use serde_json::{Value, json};

/// One vocabulary: its terms in the order the schema lists them, each paired
/// with the value it names.
pub struct Vocabulary<T: 'static> {
    /// What the argument is called, for the error an unknown term earns.
    noun: &'static str,
    /// What joins the last two terms where that error names them.
    conjunction: &'static str,
    terms: &'static [(&'static str, T)],
}

const fn vocabulary<T>(noun: &'static str, terms: &'static [(&'static str, T)]) -> Vocabulary<T> {
    Vocabulary {
        noun,
        conjunction: ", or ",
        terms,
    }
}

impl<T: Copy> Vocabulary<T> {
    /// The JSON-schema `enum` array.
    pub fn schema(&self) -> Value {
        json!(self.names())
    }

    /// The `enum` array for one platform's own list, in vocabulary order: a
    /// platform is offered only the terms it can carry.
    pub fn schema_of(&self, catalogue: &[T]) -> Value
    where
        T: PartialEq,
    {
        json!(
            self.terms
                .iter()
                .filter(|(_, value)| catalogue.contains(value))
                .map(|(term, _)| *term)
                .collect::<Vec<_>>()
        )
    }

    pub fn names(&self) -> Vec<&'static str> {
        self.terms.iter().map(|(term, _)| *term).collect()
    }

    pub fn lookup(&self, term: &str) -> Option<T> {
        self.terms
            .iter()
            .find(|(name, _)| *name == term)
            .map(|(_, value)| *value)
    }

    pub fn lookup_ignoring_case(&self, term: &str) -> Option<T> {
        self.terms
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(term))
            .map(|(_, value)| *value)
    }

    pub fn parse(&self, term: &str) -> Result<T, String> {
        self.lookup(term).ok_or_else(|| self.unknown(term))
    }

    pub fn unknown(&self, term: &str) -> String {
        format!("unknown {} {term:?}; expected {}", self.noun, self.listed())
    }

    fn listed(&self) -> String {
        let names = self.names();
        match names.split_last() {
            Some((last, [])) => (*last).to_owned(),
            Some((last, rest)) => format!("{}{}{last}", rest.join(", "), self.conjunction),
            None => String::new(),
        }
    }
}

pub static REGIONS: Vocabulary<Region> = vocabulary(
    "region",
    &[
        ("Japan", Region::Japan),
        ("Usa", Region::Usa),
        ("Europe", Region::Europe),
        ("World", Region::World),
        ("Taiwan", Region::Taiwan),
        ("Germany", Region::Germany),
        ("France", Region::France),
        ("China", Region::China),
        ("Spain", Region::Spain),
        ("Italy", Region::Italy),
        ("Australia", Region::Australia),
        ("UnitedKingdom", Region::UnitedKingdom),
        ("Korea", Region::Korea),
        ("HongKong", Region::HongKong),
        ("Sweden", Region::Sweden),
        ("Netherlands", Region::Netherlands),
        ("Canada", Region::Canada),
        ("Brazil", Region::Brazil),
        ("Argentina", Region::Argentina),
        ("Singapore", Region::Singapore),
        ("Thailand", Region::Thailand),
        ("NewZealand", Region::NewZealand),
    ],
);

pub static LANGUAGES: Vocabulary<Language> = vocabulary(
    "language",
    &[
        ("English", Language::English),
        ("French", Language::French),
        ("German", Language::German),
        ("Spanish", Language::Spanish),
        ("Italian", Language::Italian),
        ("Portuguese", Language::Portuguese),
        ("Dutch", Language::Dutch),
        ("Japanese", Language::Japanese),
        ("Swedish", Language::Swedish),
    ],
);

pub static LINK_TYPES: Vocabulary<LinkType> = vocabulary(
    "link_type",
    &[
        ("Wiki", LinkType::Wiki),
        ("Manual", LinkType::Manual),
        ("Source", LinkType::Source),
        ("Speedrun", LinkType::Speedrun),
        ("UnusedContent", LinkType::UnusedContent),
        ("TechnicalReference", LinkType::TechnicalReference),
        ("Guide", LinkType::Guide),
        ("Community", LinkType::Community),
        ("Store", LinkType::Store),
        ("DownloadPage", LinkType::DownloadPage),
        ("Download", LinkType::Download),
        ("Donate", LinkType::Donate),
    ],
);

pub static RELEASE_STATUSES: Vocabulary<ReleaseStatus> = vocabulary(
    "status",
    &[
        ("Released", ReleaseStatus::Released),
        ("Demo", ReleaseStatus::Demo),
        ("WorkInProgress", ReleaseStatus::WorkInProgress),
        ("Beta", ReleaseStatus::Beta),
        ("Prototype", ReleaseStatus::Prototype),
    ],
);

pub static TV_FORMATS: Vocabulary<TvStandard> = vocabulary(
    "tv_format",
    &[
        ("Ntsc", TvStandard::Ntsc),
        ("Pal", TvStandard::Pal),
        ("Pal60", TvStandard::Pal60),
        ("Ntsc50", TvStandard::Ntsc50),
        ("PalM", TvStandard::PalM),
        ("Secam", TvStandard::Secam),
    ],
);

pub static ENHANCEMENTS: Vocabulary<Enhancement> = vocabulary(
    "enhancement",
    &[
        ("SuperGameBoy", Enhancement::SuperGameBoy),
        ("GameBoyColor", Enhancement::GameBoyColor),
    ],
);

/// One list for every console: a platform is offered only the terms its own
/// catalogue carries.
pub static PERIPHERALS: Vocabulary<Peripheral> = vocabulary(
    "peripheral",
    &[
        ("Joystick", Peripheral::Joystick),
        ("Paddle", Peripheral::Paddle),
        ("Driving", Peripheral::Driving),
        ("Keypad", Peripheral::Keypad),
        ("Trackball", Peripheral::Trackball),
        ("BoosterGrip", Peripheral::BoosterGrip),
        ("KidVid", Peripheral::KidVid),
        ("MindLink", Peripheral::MindLink),
        ("LinkCable", Peripheral::LinkCable),
        ("Printer", Peripheral::Printer),
        ("BarcodeBoy", Peripheral::BarcodeBoy),
        ("FourPlayerAdapter", Peripheral::FourPlayerAdapter),
    ],
);

pub static MOD_CATEGORIES: Vocabulary<ModCategory> = vocabulary(
    "category",
    &[
        ("Translation", ModCategory::Translation),
        ("QualityOfLife", ModCategory::QualityOfLife),
        ("ContentChange", ModCategory::ContentChange),
        ("Compatibility", ModCategory::Compatibility),
        ("TotalConversion", ModCategory::TotalConversion),
    ],
);

/// `None` is a term of the vocabulary: it is how a defect is cleared.
pub static DEFECTS: Vocabulary<Option<Defect>> = vocabulary(
    "defect",
    &[
        ("Overdump", Some(Defect::Overdump)),
        ("BadDump", Some(Defect::BadDump)),
        ("MemoryMap", Some(Defect::MemoryMap)),
        ("None", None),
    ],
);

pub static GAME_KINDS: Vocabulary<GameKind> = Vocabulary {
    noun: "kind",
    conjunction: " or ",
    terms: &[
        ("Game", GameKind::Game),
        ("Demo", GameKind::Demo),
        ("Demoscene", GameKind::Demoscene),
        ("Test", GameKind::Test),
        ("Tool", GameKind::Tool),
        ("Software", GameKind::Software),
    ],
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_schema_array_and_the_parser_read_the_same_terms() {
        let names = PERIPHERALS.schema();
        for term in names.as_array().unwrap() {
            assert!(PERIPHERALS.lookup(term.as_str().unwrap()).is_some());
        }
        assert_eq!(names[0], "Joystick");
    }

    #[test]
    fn an_unknown_term_names_the_vocabulary_in_order() {
        assert_eq!(
            DEFECTS.unknown("Truncated"),
            "unknown defect \"Truncated\"; expected Overdump, BadDump, MemoryMap, or None"
        );
        assert_eq!(
            GAME_KINDS.unknown("Toy"),
            "unknown kind \"Toy\"; expected Game, Demo, Demoscene, Test, Tool or Software"
        );
    }
}
