//! Fixture files of sections: a `=== KIND NAME...` line, a text, then parts each under a
//! `--- PART` line; and expectation files of `KEY = ANSWER` lines.

use std::collections::BTreeMap;

/// One section: its kind, its name words, its text before its first part, and its parts.
#[derive(Debug)]
pub struct Section {
    pub kind: String,
    pub name: Vec<String>,
    pub text: String,
    pub parts: Vec<(String, String)>,
}

impl Section {
    /// Returns the text of part `part`, empty when the section has none.
    pub fn part(&self, part: &str) -> &str {
        self.parts
            .iter()
            .find(|(k, _)| k == part)
            .map_or("", |(_, v)| v.as_str())
    }
}

/// Returns the sections of `text`, in order.
pub fn sections(text: &str) -> Vec<Section> {
    text.split("=== ")
        .filter(|s| !s.is_empty())
        .map(|s| {
            let (head, body) = s.split_once('\n').expect("a section has a body");
            let mut words = head.split(' ').map(str::to_owned);
            let kind = words.next().expect("a section has a kind");
            let mut chunks = body.split("--- ");
            let text = chunks.next().unwrap_or("").to_owned();
            let parts = chunks
                .map(|c| {
                    let (k, v) = c.split_once('\n').expect("a part has a body");
                    (k.to_owned(), v.trim_end().to_owned())
                })
                .collect();
            Section {
                kind,
                name: words.collect(),
                text,
                parts,
            }
        })
        .collect()
}

/// Returns the answer of each key of the `KEY = ANSWER` lines of `text`.
pub fn expectations(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .map(|l| {
            let (k, v) = l
                .split_once(" = ")
                .expect("an expectation has a key and an answer");
            (k.to_owned(), v.to_owned())
        })
        .collect()
}
