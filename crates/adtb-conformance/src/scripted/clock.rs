//! Instants of the simulated clock in a script's values, compared by rank: a script states the
//! instant of rank N among the distinct instants printed where its compared values hold a token
//! as the token `tN`.

/// The instant a token stands for in a run that has not yet learned its instants. No
/// simulated clock reads it.
const UNKNOWN: &str = "1970-01-01T00:00:00.000000Z";

/// Returns the length of the instant in textual syntax that starts `s`, if one does.
fn instant_at(s: &[u8]) -> Option<usize> {
    let shape = b"dddd-dd-ddTdd:dd:dd";
    if s.len() < shape.len() + 1 {
        return None;
    }
    let fits = shape.iter().zip(s).all(|(&want, &c)| match want {
        b'd' => c.is_ascii_digit(),
        _ => c == want,
    });
    if !fits {
        return None;
    }
    let mut end = shape.len();
    if s[end] == b'.' {
        let digits = s[end + 1..]
            .iter()
            .take_while(|c| c.is_ascii_digit())
            .count();
        if digits == 0 {
            return None;
        }
        end += 1 + digits;
    }
    (s.get(end) == Some(&b'Z')).then_some(end + 1)
}

/// Returns each instant in textual syntax `text` holds, in order.
fn instants_in(text: &str) -> Vec<&str> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let starts = i == 0 || !bytes[i - 1].is_ascii_digit();
        match instant_at(&bytes[i..]).filter(|_| starts) {
            Some(n) => {
                out.push(&text[i..i + n]);
                i += n;
            }
            None => i += 1,
        }
    }
    out
}

/// Returns the key `instant` sorts by in time order: its seconds, then its fraction padded.
fn when(instant: &str) -> (String, String) {
    let body = instant.trim_end_matches('Z');
    let (seconds, fraction) = body.split_once('.').unwrap_or((body, ""));
    (seconds.to_owned(), format!("{fraction:0<9}"))
}

/// Returns the distinct instants `texts` hold, in time order, leaving out the one a token
/// stands for before a run learns its instants.
pub(super) fn ranked(texts: &[String]) -> Vec<String> {
    let mut all: Vec<String> = texts
        .iter()
        .flat_map(|t| instants_in(t))
        .filter(|&i| i != UNKNOWN)
        .map(str::to_owned)
        .collect();
    all.sort_by_key(|i| when(i));
    all.dedup_by_key(|i| when(i));
    all
}

/// Call `f` with the byte range and rank of each token `tN` in `text` outside a text literal.
fn each_token(text: &str, mut f: impl FnMut(std::ops::Range<usize>, usize)) {
    let bytes = text.as_bytes();
    let word = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let (mut quoted, mut escaped) = (false, false);
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if quoted {
            (quoted, escaped) = (escaped || c != b'"', !escaped && c == b'\\');
            i += 1;
            continue;
        }
        if c == b'"' {
            quoted = true;
        }
        let digits = bytes[i + 1..]
            .iter()
            .take_while(|d| d.is_ascii_digit())
            .count();
        let end = i + 1 + digits;
        let alone = (i == 0 || !word(bytes[i - 1])) && bytes.get(end).is_none_or(|&n| !word(n));
        if c == b't' && digits > 0 && alone {
            f(i..end, text[i + 1..end].parse().expect("digits"));
            i = end;
        } else {
            i += 1;
        }
    }
}

/// Returns whether `text` states a token.
pub(super) fn tokened(text: &str) -> bool {
    let mut any = false;
    each_token(text, |_, _| any = true);
    any
}

/// What a script's tokens stand for in one run.
pub(super) enum Clock {
    /// The script states no token: its values are compared as written.
    Untokened,
    /// A first run, learning the instants: each token stands for an instant no run prints.
    Learning,
    /// Each token stands for the instant of its rank.
    Ranked(Vec<String>),
}

impl Clock {
    /// Returns compared value `text` with each token replaced by the instant it stands for.
    pub(super) fn place(&self, text: &str) -> Result<String, String> {
        match self {
            Clock::Untokened => Ok(text.to_owned()),
            Clock::Learning => placed(text, None),
            Clock::Ranked(instants) => placed(text, Some(instants)),
        }
    }
}

/// Returns `text` with each token replaced by the instant of its rank in `instants`, or by an
/// instant no run prints when `instants` is not known yet.
fn placed(text: &str, instants: Option<&[String]>) -> Result<String, String> {
    let mut out = String::new();
    let mut last = 0;
    let mut missing = None;
    each_token(text, |range, rank| {
        let instant = match instants {
            None => UNKNOWN,
            Some(all) => match rank.checked_sub(1).and_then(|i| all.get(i)) {
                Some(i) => i,
                None => {
                    missing.get_or_insert(format!(
                        "`t{rank}` names no instant: the run prints {}",
                        all.len()
                    ));
                    UNKNOWN
                }
            },
        };
        out.push_str(&text[last..range.start]);
        out.push_str(instant);
        last = range.end;
    });
    out.push_str(&text[last..]);
    missing.map_or(Ok(out), Err)
}

/// Returns `text` with each instant of `instants` replaced by its token.
pub(super) fn shown(text: &str, instants: &[String]) -> String {
    let mut out = text.to_owned();
    for (rank, instant) in instants.iter().enumerate().rev() {
        out = out.replace(instant.as_str(), &format!("t{}", rank + 1));
    }
    out
}

#[test]
fn tokens_rank_the_instants_in_time_order() {
    let printed = vec![
        "[2026-01-01T04:00:00.1Z, 2026-01-01T00:00:00.143200Z]".to_owned(),
        "Retired { at: 2026-01-01T00:00:00.143200Z }".to_owned(),
        "[2026-01-01T00:00:01Z]".to_owned(),
    ];
    let instants = ranked(&printed);
    assert_eq!(
        vec![
            "2026-01-01T00:00:00.143200Z",
            "2026-01-01T00:00:01Z",
            "2026-01-01T04:00:00.1Z"
        ],
        instants
    );
    assert_eq!(
        Ok("[2026-01-01T04:00:00.1Z, \"t1\", t1x]".to_owned()),
        placed("[t3, \"t1\", t1x]", Some(&instants))
    );
    assert_eq!(
        Err("`t4` names no instant: the run prints 3".to_owned()),
        placed("t4", Some(&instants))
    );
    assert_eq!("[t3, t1]", shown(&printed[0], &instants));
}
