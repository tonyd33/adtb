use std::collections::BTreeSet;

use maud::{Markup, html};

#[derive(Debug, PartialEq, Eq)]
enum Part<'a> {
    Text(&'a str),
    Link { name: &'a str, label: &'a str },
}

fn parts(text: &str) -> Vec<Part<'_>> {
    let mut result = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("[[") {
        let Some(end) = rest[start + 2..].find("]]") else {
            break;
        };
        let end = start + 2 + end;
        let inner = &rest[start + 2..end];
        let (name, label) = inner.split_once('|').unwrap_or((inner, inner));
        if name.is_empty()
            || label.is_empty()
            || inner.contains(['[', ']', '\n', '\r'])
            || label.contains('|')
        {
            result.push(Part::Text(&rest[..end + 2]));
        } else {
            result.push(Part::Text(&rest[..start]));
            result.push(Part::Link { name, label });
        }
        rest = &rest[end + 2..];
    }
    result.push(Part::Text(rest));
    result
}

pub(super) fn names(text: &str) -> Vec<String> {
    parts(text)
        .into_iter()
        .filter_map(|part| match part {
            Part::Text(_) => None,
            Part::Link { name, .. } => Some(name.to_owned()),
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

pub(super) fn render(text: &str, targets: &[(i64, &str)], prefix: &str, gm: bool) -> Markup {
    html! {
        @for part in parts(text) {
            @match part {
                Part::Text(text) => { (text) }
                Part::Link { name, label } => {
                    @if let Some((id, _)) = targets.iter().find(|(_, target)| *target == name) {
                        a href=(format!("{prefix}/{id}")) { (label) }
                    } @else if gm {
                        span class="unresolved" title="Unresolved entity name" { (label) }
                    } @else {
                        (label)
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{names, render};

    #[test]
    fn parse_links_without_losing_literal_text() {
        assert_eq!(
            names("[[雪|Snow]] [[A]] [[A|again]] [[B]]"),
            ["A", "B", "雪"]
        );
        for text in [
            "[[",
            "[[A",
            "[[A|]]",
            "[[|label]]",
            "[[A|x|y]]",
            "[[A\nB]]",
            "[[[A]]",
        ] {
            assert!(names(text).is_empty(), "{text}");
            assert_eq!(render(text, &[], "/x", false).into_string(), text, "{text}");
        }
    }

    #[test]
    fn render_only_allowed_targets_and_escape_text() {
        let text = "<script> [[Known|<b>]] [[Hidden|alias]] [[Unknown]]";
        let player = render(text, &[(7, "Known")], "/play/entities", false).into_string();
        assert_eq!(
            player,
            "&lt;script&gt; <a href=\"/play/entities/7\">&lt;b&gt;</a> alias Unknown"
        );
        assert!(!player.contains("Hidden"));
        assert!(!player.contains("unresolved"));
        let gm = render(text, &[], "/gm", true).into_string();
        assert!(gm.contains("class=\"unresolved\""));
    }
}
