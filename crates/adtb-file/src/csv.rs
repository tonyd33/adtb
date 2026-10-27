//! CSV files of RFC 4180 with a header row, columns matched by name.

use std::collections::BTreeMap;

use adtb_lang::schema::Schema;
use adtb_lang::textual::Textual;
use adtb_lang::types::Ty;

use super::{
    Found, Shape, Sum, allowed, ctor, key_value_ty, none, parse, record, shape, some, text,
};

struct Cell {
    text: String,
    quoted: bool,
}

impl Cell {
    fn is_null(&self) -> bool {
        self.text.is_empty() && !self.quoted
    }
}

/// Returns each record of `text` after its header, as the file holds it, with its row read at
/// `row` or why it does not convert. Fails when the text is not CSV.
pub(super) fn records<V: Textual>(
    schema: &Schema,
    text: &str,
    row: &Ty,
) -> Result<Vec<Found<V>>, String> {
    let mut all = split(text)?.into_iter();
    let Some((_, header)) = all.next() else {
        return Ok(Vec::new());
    };
    let names: Vec<String> = header.into_iter().map(|c| c.text).collect();
    Ok(all
        .map(|(raw, cells)| {
            let read = if cells.len() == names.len() {
                let mut by_name = BTreeMap::new();
                for (n, c) in names.iter().zip(cells) {
                    by_name.entry(n.as_str()).or_insert(c);
                }
                Reader {
                    schema,
                    cells: by_name,
                }
                .read("", row)
            } else {
                Err(format!(
                    "the record has {} fields and the header {}",
                    cells.len(),
                    names.len()
                ))
            };
            (raw, read)
        })
        .collect())
}

/// Returns each nonempty record of `text`, as the file holds it and as its cells, past a leading
/// byte order mark. A line feed, a carriage return and the pair of them each end a record.
fn split(text: &str) -> Result<Vec<(String, Vec<Cell>)>, String> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut at = if text.starts_with('\u{feff}') {
        '\u{feff}'.len_utf8()
    } else {
        0
    };
    while at < bytes.len() {
        let start = at;
        let mut cells = Vec::new();
        loop {
            let (cell, next) = cell(text, at)?;
            cells.push(cell);
            at = next;
            match bytes.get(at) {
                Some(b',') => at += 1,
                _ => break,
            }
        }
        let end = at;
        match bytes.get(at) {
            None => {}
            Some(b'\r') if bytes.get(at + 1) == Some(&b'\n') => at += 2,
            Some(b'\n' | b'\r') => at += 1,
            Some(_) => {
                return Err(format!(
                    "byte {at}: a quoted cell ends before its separator"
                ));
            }
        }
        if end > start {
            out.push((text[start..end].to_string(), cells));
        }
    }
    Ok(out)
}

/// Returns the cell starting at byte `at` of `text` and the byte after it.
fn cell(text: &str, at: usize) -> Result<(Cell, usize), String> {
    let bytes = text.as_bytes();
    if bytes.get(at) != Some(&b'"') {
        let end = text[at..]
            .find([',', '\n', '\r'])
            .map_or(text.len(), |i| at + i);
        let raw = &text[at..end];
        if raw.contains('"') {
            return Err(format!("byte {at}: a quote inside an unquoted cell"));
        }
        return Ok((
            Cell {
                text: raw.to_string(),
                quoted: false,
            },
            end,
        ));
    }
    let mut out = String::new();
    let mut i = at + 1;
    loop {
        let Some(off) = text[i..].find('"') else {
            return Err(format!("byte {at}: a quoted cell is not closed"));
        };
        out.push_str(&text[i..i + off]);
        i += off + 1;
        if bytes.get(i) == Some(&b'"') {
            out.push('"');
            i += 1;
        } else {
            return Ok((
                Cell {
                    text: out,
                    quoted: true,
                },
                i,
            ));
        }
    }
}

fn join(at: &str, name: &str) -> String {
    if at.is_empty() {
        name.to_string()
    } else {
        format!("{at}.{name}")
    }
}

struct Reader<'s, 'c> {
    schema: &'s Schema,
    cells: BTreeMap<&'c str, Cell>,
}

impl Reader<'_, '_> {
    fn cell(&self, at: &str) -> Result<&Cell, String> {
        self.cells
            .get(at)
            .ok_or_else(|| format!("the file has no column `{at}`"))
    }

    /// Returns whether every column at or under `at` is missing or null.
    fn absent(&self, at: &str) -> bool {
        let under = format!("{at}.");
        self.cells
            .iter()
            .filter(|(n, _)| **n == at || n.starts_with(&under))
            .all(|(_, c)| c.is_null())
    }

    fn scalar<V: Textual>(&self, at: &str, ty: &Ty) -> Result<V, String> {
        let c = self.cell(at)?;
        parse(self.schema, &c.text, ty).map_err(|e| format!("column `{at}`: {e}"))
    }

    /// Reads the value of type `ty` the columns at `at` hold, `""` naming the row itself.
    fn read<V: Textual>(&self, at: &str, ty: &Ty) -> Result<V, String> {
        match shape(self.schema, ty) {
            Shape::Scalar | Shape::Bool => self.scalar(at, ty),
            Shape::Text => Ok(text(&self.cell(at)?.text)),
            Shape::Option(inner) => {
                if self.absent(at) {
                    Ok(none())
                } else {
                    Ok(some(self.read(at, &inner)?))
                }
            }
            Shape::Names(names, set) => {
                let name = &self.cell(at)?.text;
                let i = names
                    .iter()
                    .position(|n| n == name)
                    .ok_or_else(|| format!("column `{at}`: `{name}` is not a constructor"))?;
                let i = allowed(i as u32, name, set.as_ref())
                    .map_err(|e| format!("column `{at}`: {e}"))?;
                Ok(ctor(i, Vec::new()))
            }
            Shape::Key(rel) => Ok(V::of_key(self.read(at, &key_value_ty(self.schema, rel))?)),
            Shape::Newtype(base) => self.read(at, &base),
            Shape::Record(fields, declared) => Ok(record(self.fields(at, &fields)?, declared)),
            Shape::Sum(sum) => {
                if !at.is_empty() && self.cells.contains_key(at) {
                    return self.scalar(at, ty);
                }
                self.tagged(at, &sum)
            }
            Shape::Other => Err(format!("column `{at}`: the type has no flat form")),
        }
    }

    fn fields<V: Textual>(
        &self,
        at: &str,
        fields: &[(String, Ty)],
    ) -> Result<Vec<(String, V)>, String> {
        fields
            .iter()
            .map(|(n, t)| Ok((n.clone(), self.read(&join(at, n), t)?)))
            .collect()
    }

    /// Reads a sum from its tag columns: `tag`, its header fields, and `C.g` for each field `g`
    /// of each constructor `C`, null unless the tag is `C`.
    fn tagged<V: Textual>(&self, at: &str, sum: &Sum) -> Result<V, String> {
        let tag_at = join(at, "tag");
        let tag = &self.cell(&tag_at)?.text;
        let c = sum
            .ctor(tag)
            .map_err(|e| format!("column `{tag_at}`: {e}"))?;
        for (k, (name, fs)) in sum.ctors.iter().enumerate() {
            if k as u32 == c {
                continue;
            }
            for (g, _) in fs {
                let other = join(at, &format!("{name}.{g}"));
                if !self.absent(&other) {
                    return Err(format!("column `{other}` is set but the tag is `{tag}`"));
                }
            }
        }
        let (name, own) = &sum.ctors[c as usize];
        let mut fields = self.fields(at, &sum.header)?;
        fields.extend(self.fields(&join(at, name), own)?);
        Ok(ctor(c, fields))
    }
}

#[cfg(test)]
mod tests {
    use super::split;

    #[test]
    fn an_error_after_a_byte_order_mark_counts_bytes_from_the_file_s_start() {
        assert_eq!(
            Err("byte 7: a quoted cell is not closed".to_string()),
            split("\u{feff}a,b\n\"x,1\n").map(|_| ())
        );
        assert_eq!(
            Err("byte 4: a quoted cell is not closed".to_string()),
            split("a,b\n\"x,1\n").map(|_| ())
        );
    }

    #[test]
    fn a_byte_order_mark_is_not_part_of_the_first_record() {
        let records = split("\u{feff}a,b\n1,2\n").expect("a valid file");
        assert_eq!(
            vec!["a,b", "1,2"],
            records
                .iter()
                .map(|(raw, _)| raw.as_str())
                .collect::<Vec<_>>()
        );
        assert_eq!("a", records[0].1[0].text);
    }
}
