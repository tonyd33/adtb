use std::fmt;

use adtb_lang::collation::{self, CaseFirst, MaxVariable, Strength};
use icu_casemap::CaseMapper;
use icu_collator::options::{self, AlternateHandling, CaseLevel, CollatorOptions};
use icu_collator::preferences::{CollationCaseFirst, CollationNumericOrdering};
use icu_collator::{CollatorBorrowed, CollatorPreferences};
use icu_locale_core::{LanguageIdentifier, Locale};
use icu_segmenter::GraphemeClusterSegmenter;
use regex::Regex;
use unicode_normalization::{IsNormalized, UnicodeNormalization, is_nfc_quick};

pub fn nfc(t: &str) -> String {
    match is_nfc_quick(t.chars()) {
        IsNormalized::Yes => t.to_string(),
        _ => t.nfc().collect(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Case {
    Lower,
    Upper,
    Fold,
}

/// Returns the full default case mapping of `t`, normalized to NFC.
pub fn case(t: &str, c: Case) -> String {
    let m = CaseMapper::new();
    let root = LanguageIdentifier::UNKNOWN;
    let mapped = match c {
        Case::Lower => m.lowercase_to_string(t, &root),
        Case::Upper => m.uppercase_to_string(t, &root),
        Case::Fold => m.fold_string(t),
    };
    nfc(&mapped)
}

/// Returns the number of extended grapheme clusters of `t`.
pub fn graphemes(t: &str) -> usize {
    if t.is_empty() {
        return 0;
    }
    GraphemeClusterSegmenter::new().segment_str(t).count() - 1
}

/// A compiled `matches` pattern.
#[derive(Clone, Debug)]
pub struct Pattern(Regex);

impl Pattern {
    pub fn new(re: &str) -> Result<Pattern, String> {
        Regex::new(re).map(Pattern).map_err(|e| e.to_string())
    }

    pub fn is_match(&self, t: &str) -> bool {
        self.0.is_match(t)
    }
}

/// A collator built from a collation name.
pub struct Collator(CollatorBorrowed<'static>);

impl fmt::Debug for Collator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Collator")
    }
}

impl Collator {
    pub fn new(name: &str) -> Result<Collator, String> {
        let c = collation::parse(name)?;
        let locale: Locale = c
            .locale
            .parse()
            .map_err(|e| format!("collation `{name}`: {e}"))?;
        let mut prefs = CollatorPreferences::from(&locale);
        prefs.case_first = c.case_first.map(|f| match f {
            CaseFirst::Upper => CollationCaseFirst::Upper,
            CaseFirst::Lower => CollationCaseFirst::Lower,
            CaseFirst::Off => CollationCaseFirst::False,
        });
        prefs.numeric_ordering = c.numeric.map(|n| {
            if n {
                CollationNumericOrdering::True
            } else {
                CollationNumericOrdering::False
            }
        });
        let mut opts = CollatorOptions::default();
        opts.strength = c.strength.map(|s| match s {
            Strength::Primary => options::Strength::Primary,
            Strength::Secondary => options::Strength::Secondary,
            Strength::Tertiary => options::Strength::Tertiary,
            Strength::Quaternary => options::Strength::Quaternary,
            Strength::Identical => options::Strength::Identical,
        });
        opts.alternate_handling = c.shifted.map(|s| {
            if s {
                AlternateHandling::Shifted
            } else {
                AlternateHandling::NonIgnorable
            }
        });
        opts.case_level = c
            .case_level
            .map(|l| if l { CaseLevel::On } else { CaseLevel::Off });
        opts.max_variable = c.max_variable.map(|v| match v {
            MaxVariable::Space => options::MaxVariable::Space,
            MaxVariable::Punctuation => options::MaxVariable::Punctuation,
            MaxVariable::Symbol => options::MaxVariable::Symbol,
            MaxVariable::Currency => options::MaxVariable::Currency,
        });
        CollatorBorrowed::try_new(prefs, opts)
            .map(Collator)
            .map_err(|e| format!("collation `{name}`: {e}"))
    }

    /// Returns the sort key of `t`.
    pub fn key(&self, t: &str) -> Vec<u8> {
        let mut out = Vec::new();
        let Ok(()) = self.0.write_sort_key_to(t, &mut out);
        out
    }
}
