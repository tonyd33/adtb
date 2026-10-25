use adtb_lang::collation::{self, CaseFirst, MaxVariable, Strength};
use icu_casemap::CaseMapper;
use icu_collator::options::{self, AlternateHandling, CaseLevel, CollatorOptions};
use icu_collator::preferences::{CollationCaseFirst, CollationNumericOrdering};
use icu_collator::{Collator, CollatorPreferences};
use icu_locale_core::{LanguageIdentifier, Locale};
use icu_segmenter::GraphemeClusterSegmenter;
use unicode_normalization::UnicodeNormalization;

pub fn nfc(t: &str) -> String {
    t.nfc().collect()
}

pub fn lower(t: &str) -> String {
    nfc(&CaseMapper::new().lowercase_to_string(t, &LanguageIdentifier::UNKNOWN))
}

pub fn upper(t: &str) -> String {
    nfc(&CaseMapper::new().uppercase_to_string(t, &LanguageIdentifier::UNKNOWN))
}

pub fn fold(t: &str) -> String {
    nfc(&CaseMapper::new().fold_string(t))
}

pub fn grapheme_len(t: &str) -> usize {
    GraphemeClusterSegmenter::new()
        .segment_str(t)
        .skip(1)
        .count()
}

/// Returns whether pattern `re` matches somewhere in `t`.
///
/// Preconditions: `re` is a pattern the checker accepted.
pub fn matches(t: &str, re: &str) -> bool {
    regex::Regex::new(re)
        .unwrap_or_else(|e| panic!("the checker accepted the pattern `{re}`: {e}"))
        .is_match(t)
}

/// Returns the sort key of `t` under collation `name`.
///
/// Preconditions: `name` is a collation name the checker accepted.
pub fn collate(t: &str, name: &str) -> Vec<u8> {
    let c = collation::parse(name)
        .unwrap_or_else(|e| panic!("the checker accepted the collation name: {e}"));
    let locale = Locale::try_from_str(&c.locale)
        .unwrap_or_else(|e| panic!("`{}` is a BCP 47 locale: {e}", c.locale));
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
        .map(|on| if on { CaseLevel::On } else { CaseLevel::Off });
    opts.max_variable = c.max_variable.map(|m| match m {
        MaxVariable::Space => options::MaxVariable::Space,
        MaxVariable::Punctuation => options::MaxVariable::Punctuation,
        MaxVariable::Symbol => options::MaxVariable::Symbol,
        MaxVariable::Currency => options::MaxVariable::Currency,
    });
    let collator = Collator::try_new(prefs, opts)
        .unwrap_or_else(|e| panic!("compiled data has a collator for `{name}`: {e}"));
    let mut key = Vec::new();
    let Ok(()) = collator.write_sort_key_to(t, &mut key);
    key
}
