//! The editor grammar of `tools/tree-sitter-adtb`, compiled from its generated parser and its
//! external scanner.

use tree_sitter_language::LanguageFn;

unsafe extern "C" {
    fn tree_sitter_adtb() -> *const ();
}

// SAFETY: the generated parser's entry point takes nothing and returns its static language.
pub const LANGUAGE: LanguageFn = unsafe { LanguageFn::from_raw(tree_sitter_adtb) };
