//! Text functions that follow the Unicode tables: normalisation, case, segmentation, regular
//! expressions and collation.

mod common;

use adtb_reference::Value;
use common::*;

fn fixture() -> Fixture {
    Fixture::new(
        "type Note = { id: Key notes, body: Text }\nrelation notes : Note key id generate sequence",
    )
}

fn yes() -> Result<Value, adtb_reference::Abort> {
    Ok(Value::bool(true))
}

fn no() -> Result<Value, adtb_reference::Abort> {
    Ok(Value::bool(false))
}

#[test]
fn concatenation_is_renormalised_to_nfc() {
    let f = fixture();
    assert_eq!(Ok(text("\u{e9}")), f.eval("\"e\" ++ \"\\u{301}\""));
    assert_eq!(
        Ok(text("\u{1e0d}\u{307}")),
        f.eval("\"\\u{1e0b}\" ++ \"\\u{323}\""),
        "canonical reordering puts the dot below first"
    );
    assert_eq!(
        Ok(Value::bool(true)),
        f.eval("\"e\" ++ \"\\u{301}\" == \"\\u{e9}\"")
    );
}

#[test]
fn case_mapping_is_full_and_renormalised() {
    let f = fixture();
    assert_eq!(Ok(text("\u{3aa}\u{301}")), f.eval("upper(\"\u{390}\")"));
    assert_eq!(Ok(text("i\u{307}")), f.eval("lower(\"\u{130}\")"));
    assert_eq!(
        Ok(text("\u{3c3}\u{3c2}")),
        f.eval("lower(\"\u{3a3}\u{3a3}\")")
    );
    assert_eq!(Ok(text("FFI")), f.eval("upper(\"\u{fb03}\")"));
    assert_eq!(
        Ok(text("istanbul")),
        f.eval(r#"lower("ISTANBUL")"#),
        "no locale: never the Turkish dotless i"
    );
}

#[test]
fn fold_is_full_case_folding_renormalised() {
    let f = fixture();
    assert_eq!(Ok(text("ss")), f.eval("fold(\"\u{1e9e}\")"));
    assert_eq!(Ok(text("\u{390}")), f.eval("fold(\"\u{390}\")"));
    assert_eq!(Ok(text("ffi")), f.eval("fold(\"\u{fb03}\")"));
    assert_eq!(yes(), f.eval(r#"fold("Straße") == fold("STRASSE")"#));
    assert_eq!(Ok(text("\u{3c3}")), f.eval("fold(\"\u{3c2}\")"));
}

#[test]
fn utf8_decodes_to_nfc() {
    let f = fixture();
    assert_eq!(
        Ok(Value::some(text("\u{e9}"))),
        f.eval(r#"match Bytes.parse("0x65cc81") | Some b -> utf8(b.value) | None -> None"#)
    );
    assert_eq!(
        Ok(Value::none()),
        f.eval(r#"match Bytes.parse("0xff") | Some b -> utf8(b.value) | None -> None"#)
    );
}

#[test]
fn grapheme_len_counts_extended_clusters() {
    let f = fixture();
    assert_eq!(Ok(int(0)), f.eval(r#"grapheme_len("")"#));
    assert_eq!(Ok(int(3)), f.eval(r#"grapheme_len("abc")"#));
    assert_eq!(Ok(int(1)), f.eval(r#"grapheme_len("\r\n")"#));
    assert_eq!(
        Ok(int(2)),
        f.eval("grapheme_len(\"\u{1f1eb}\u{1f1f7}\u{1f1e9}\u{1f1ea}\")")
    );
    assert_eq!(
        Ok(int(1)),
        f.eval("grapheme_len(\"\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}\")")
    );
    assert_eq!(
        Ok(int(1)),
        f.eval("grapheme_len(\"\u{1100}\u{1161}\u{11a8}\")")
    );
    assert_eq!(
        Ok(int(1)),
        f.eval("scalar_len(\"\u{1100}\u{1161}\u{11a8}\")"),
        "NFC composes the jamo into one syllable"
    );
    assert_eq!(
        Ok(int(1)),
        f.eval("grapheme_len(\"\u{1100}\u{1161}\u{11a8}\u{11a8}\")")
    );
}

#[test]
fn matches_is_unanchored_unless_anchored() {
    let f = fixture();
    assert_eq!(yes(), f.eval(r#"matches("xaby", "a.y$")"#));
    assert_eq!(no(), f.eval(r#"matches("xaby", "^a")"#));
    assert_eq!(yes(), f.eval(r#"matches("", "")"#));
    assert_eq!(
        no(),
        f.eval(r#"matches("ab\nc", "^c")"#),
        "`^` is the start of text"
    );
}

#[test]
fn matches_works_over_scalar_values() {
    let f = fixture();
    assert_eq!(yes(), f.eval("matches(\"\u{e9}\", \"^.$\")"));
    assert_eq!(yes(), f.eval("matches(\"\u{1f600}\", \"^.$\")"));
    assert_eq!(yes(), f.eval(r#"matches("añb", "^\\p{L}+$")"#));
    assert_eq!(no(), f.eval(r#"matches("a1", "^\\p{L}+$")"#));
    assert_eq!(no(), f.eval(r#"matches("ПРИВЕТ", "^[а-я]+$")"#));
}

#[test]
fn the_perl_classes_of_matches_are_unicode() {
    let f = fixture();
    assert_eq!(yes(), f.eval(r#"matches("٣", "^\\d$")"#));
    assert_eq!(no(), f.eval(r#"matches("x", "^\\d$")"#));
    assert_eq!(yes(), f.eval(r#"matches("é_1", "^\\w+$")"#));
    assert_eq!(no(), f.eval(r#"matches("a-b", "^\\w+$")"#));
    assert_eq!(yes(), f.eval("matches(\"a\u{2003}b\", \"^a\\\\sb$\")"));
    assert_eq!(yes(), f.eval(r#"matches("café bar", "café\\b")"#));
    assert_eq!(no(), f.eval(r#"matches("cafés", "café\\b")"#));
}

#[test]
fn matches_is_case_sensitive_unless_it_begins_with_i() {
    let f = fixture();
    assert_eq!(no(), f.eval(r#"matches("ABC", "abc")"#));
    assert_eq!(yes(), f.eval(r#"matches("ABC", "(?i)abc")"#));
    assert_eq!(yes(), f.eval(r#"matches("ПРИВЕТ", "(?i)^привет$")"#));
    assert_eq!(
        no(),
        f.eval("matches(\"\u{df}\", \"(?i)^ss$\")"),
        "simple case folding never maps one scalar to two"
    );
}

#[test]
fn collate_gives_uts10_sort_keys() {
    let f = fixture();
    let less = |a: &str, b: &str, c: &str| {
        f.eval(&format!(r#"collate("{a}", "{c}") < collate("{b}", "{c}")"#))
    };
    assert_eq!(yes(), less("a", "B", "und@cldr-49"));
    assert_eq!(
        yes(),
        f.eval(r#""B" < "a""#),
        "code point order is not collation order"
    );
    assert_eq!(yes(), less("\u{f6}", "z", "de@cldr-49"));
    assert_eq!(yes(), less("z", "\u{f6}", "sv@cldr-49"));
    assert_eq!(yes(), less("a2", "a10", "en-u-kn-true@cldr-49"));
    assert_eq!(yes(), less("a10", "a2", "en@cldr-49"));
}

#[test]
fn collation_strength_decides_what_differs() {
    let f = fixture();
    let same = |a: &str, b: &str, c: &str| {
        f.eval(&format!(
            r#"collate("{a}", "{c}") == collate("{b}", "{c}")"#
        ))
    };
    assert_eq!(yes(), same("a", "A", "und-u-ks-level1@cldr-49"));
    assert_eq!(yes(), same("a", "\u{e1}", "und-u-ks-level1@cldr-49"));
    assert_eq!(no(), same("a", "\u{e1}", "und-u-ks-level2@cldr-49"));
    assert_eq!(yes(), same("a", "A", "und-u-ks-level2@cldr-49"));
    assert_eq!(no(), same("a", "A", "und-u-ks-level3@cldr-49"));
    assert_eq!(no(), same("a", "A", "und@cldr-49"));
}

#[test]
fn a_sort_key_is_stable_for_its_name() {
    let f = fixture();
    let k = f.eval(r#"collate("Straße", "de@cldr-49")"#).unwrap();
    let Value::SortKey(bytes) = &k else {
        panic!("a sort key, found {k:?}")
    };
    assert!(!bytes.is_empty());
    assert_eq!(Ok(k.clone()), f.eval(r#"collate("Straße", "de@cldr-49")"#));
}
