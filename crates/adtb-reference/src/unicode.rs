use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use adtb_lang::collation::{self, CaseFirst, MaxVariable, Strength};
use icu_casemap::CaseMapper;
use icu_collator::options::{self, AlternateHandling, CaseLevel, CollatorOptions};
use icu_collator::preferences::{CollationCaseFirst, CollationNumericOrdering};
use icu_collator::{Collator, CollatorBorrowed, CollatorPreferences};
use icu_locale_core::{LanguageIdentifier, Locale};
use icu_segmenter::GraphemeClusterSegmenter;
use regex::Regex;
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

/// The patterns and collators one statement has compiled, by their text.
#[derive(Debug, Default)]
pub(crate) struct Compiled {
    regexes: RefCell<BTreeMap<String, Rc<Regex>>>,
    collators: RefCell<BTreeMap<String, Rc<CollatorBorrowed<'static>>>>,
}

impl Compiled {
    /// Returns whether pattern `re` matches somewhere in `t`.
    ///
    /// Preconditions: `re` is a pattern the checker accepted.
    pub(crate) fn matches(&self, t: &str, re: &str) -> bool {
        let found = self.regexes.borrow().get(re).cloned();
        let regex = found.unwrap_or_else(|| {
            let r = Rc::new(
                Regex::new(re)
                    .unwrap_or_else(|e| panic!("the checker accepted the pattern `{re}`: {e}")),
            );
            self.regexes.borrow_mut().insert(re.to_string(), r.clone());
            r
        });
        regex.is_match(t)
    }

    /// Returns the sort key of `t` under collation `name`.
    ///
    /// Preconditions: `name` is a collation name the checker accepted.
    pub(crate) fn collate(&self, t: &str, name: &str) -> Vec<u8> {
        let found = self.collators.borrow().get(name).cloned();
        let collator = found.unwrap_or_else(|| {
            let c = Rc::new(collator(name));
            self.collators
                .borrow_mut()
                .insert(name.to_string(), c.clone());
            c
        });
        let mut key = Vec::new();
        let Ok(()) = collator.write_sort_key_to(t, &mut key);
        key
    }
}

/// Builds the collator of collation `name`.
fn collator(name: &str) -> CollatorBorrowed<'static> {
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
    Collator::try_new(prefs, opts)
        .unwrap_or_else(|e| panic!("compiled data has a collator for `{name}`: {e}"))
}

#[cfg(test)]
mod tests {
    use super::Compiled;

    #[test]
    fn a_pattern_or_collation_is_compiled_once() {
        let c = Compiled::default();
        assert!(c.matches("ab", "a"));
        assert!(!c.matches("xy", "a"));
        assert!(c.matches("ba", "b"));
        assert_eq!(2, c.regexes.borrow().len());
        let k = c.collate("a", "de@cldr-49");
        assert_eq!(k, c.collate("a", "de@cldr-49"));
        assert_eq!(1, c.collators.borrow().len());
    }
}
