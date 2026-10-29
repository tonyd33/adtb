use super::*;

impl Granted {
    /// Returns the name of the privilege, as `catalog::grants` writes it.
    pub fn privilege_name(&self) -> &'static str {
        match self {
            Granted::Read(..) => "read",
            Granted::Insert(_) => "insert",
            Granted::Update(..) => "update",
            Granted::Delete(_) => "delete",
            Granted::Retag(..) => "retag",
            Granted::CallFn(_) => "call",
        }
    }

    /// Returns what the privilege is granted on, in the current names of `schema`: the
    /// relation as `module::name`, then `.C`, `.(C | D)` or `.S` for a constructor list or a
    /// named set. A `read` or `update` follows with its fields, `(f, C.g)` when it lists them and
    /// `frozen (f, C.g)` when it does not, a field of a header or of a type of one constructor by
    /// its name and any other by its constructor's. A `retag` is `a -> b`, then `except (C -> D, ...)` when it excepts pairs.
    /// A `call` is the function as `module::name`. `None` when it names an object `schema`
    /// lacks.
    pub fn target_text(&self, schema: &Schema) -> Option<String> {
        Some(match self {
            Granted::Read(t, fs) | Granted::Update(t, fs) => {
                let fields = fs
                    .ids
                    .iter()
                    .map(|&id| field_name(schema, id))
                    .collect::<Option<Vec<_>>>()?
                    .join(", ");
                let frozen = if fs.listed { "" } else { "frozen " };
                format!("{} {frozen}({fields})", target(schema, t)?)
            }
            Granted::Insert(t) | Granted::Delete(t) => target(schema, t)?,
            Granted::Retag(a, b, except) => {
                let mut out = format!("{} -> {}", target(schema, a)?, target(schema, b)?);
                if !except.is_empty() {
                    let pairs = except
                        .iter()
                        .map(|&(x, y)| {
                            Some(format!(
                                "{} -> {}",
                                ctor_name(schema, x)?,
                                ctor_name(schema, y)?
                            ))
                        })
                        .collect::<Option<Vec<_>>>()?;
                    out.push_str(&format!(" except ({})", pairs.join(", ")));
                }
                out
            }
            Granted::CallFn(id) => {
                let (&f, _) = schema.ids.functions.iter().find(|(_, x)| **x == *id)?;
                let fd = schema.function(f);
                format!("{}::{}", schema.module(fd.module).name, fd.name)
            }
        })
    }

    /// Returns the privilege named `privilege` on the target `text`, as
    /// [`Granted::target_text`] writes it over `schema`; `None` when `text` is not of that form
    /// or names an object `schema` lacks.
    pub fn read_target(schema: &Schema, privilege: &str, text: &str) -> Option<Granted> {
        let mut r = Reader {
            toks: tokens(text)?,
            at: 0,
        };
        let out = match privilege {
            "read" | "update" => {
                let (rel, t) = r.target(schema)?;
                let listed = !r.eat("frozen");
                let ids = r.fields(schema, rel)?;
                let fs = GrantFields { ids, listed };
                if privilege == "read" {
                    Granted::Read(t, fs)
                } else {
                    Granted::Update(t, fs)
                }
            }
            "insert" => Granted::Insert(r.target(schema)?.1),
            "delete" => Granted::Delete(r.target(schema)?.1),
            "retag" => {
                let (rel, a) = r.target(schema)?;
                r.expect("->")?;
                let b = r.target(schema)?.1;
                let mut except = Vec::new();
                if r.eat("except") {
                    r.expect("(")?;
                    loop {
                        let x = ctor_id(schema, rel, &r.word()?)?;
                        r.expect("->")?;
                        let y = ctor_id(schema, rel, &r.word()?)?;
                        except.push((x, y));
                        if !r.eat(",") {
                            break;
                        }
                    }
                    r.expect(")")?;
                }
                Granted::Retag(a, b, except)
            }
            "call" => {
                let word = r.word()?;
                let (module, name) = qualified(&word)?;
                let (_, &id) = schema.ids.functions.iter().find(|(f, _)| {
                    let fd = schema.function(**f);
                    fd.name == name && schema.module(fd.module).name == module
                })?;
                Granted::CallFn(id)
            }
            _ => return None,
        };
        (r.at == r.toks.len()).then_some(out)
    }
}

fn target(schema: &Schema, t: &GrantTarget) -> Option<String> {
    let (&rel, _) = schema
        .ids
        .relations
        .iter()
        .find(|(_, x)| **x == t.relation)?;
    let rd = schema.relation(rel);
    let name = format!("{}::{}", schema.module(rd.module).name, rd.name);
    Some(match &t.sel {
        GrantSel::Whole => name,
        GrantSel::Set(s) => {
            let ((_, set), _) = schema.ids.sets.iter().find(|(_, x)| **x == *s)?;
            format!("{name}.{set}")
        }
        GrantSel::Ctors(cs) => {
            let names = cs
                .iter()
                .map(|&c| ctor_name(schema, c))
                .collect::<Option<Vec<_>>>()?;
            match names.as_slice() {
                [one] => format!("{name}.{one}"),
                _ => format!("{name}.({})", names.join(" | ")),
            }
        }
    })
}

fn ctor_name(schema: &Schema, id: u64) -> Option<String> {
    let (&(def, c), _) = schema.ids.ctors.iter().find(|(_, x)| **x == id)?;
    Some(schema.ty(def).ctors()[c as usize].name.clone())
}

fn field_name(schema: &Schema, id: u64) -> Option<String> {
    let ((def, c, name), _) = schema.ids.fields.iter().find(|(_, x)| **x == id)?;
    let ctors = schema.ty(*def).ctors();
    Some(match c {
        Some(c) if ctors.len() > 1 => format!("{}.{name}", ctors[*c as usize].name),
        _ => name.clone(),
    })
}

fn ctor_id(schema: &Schema, rel: RelId, name: &str) -> Option<u64> {
    let def = schema.relation(rel).elem_def;
    let c = schema.ty(def).ctor_index(name)?;
    schema.ids.ctors.get(&(def, c)).copied()
}

fn qualified(word: &str) -> Option<(&str, &str)> {
    word.rsplit_once("::")
}

/// Returns the tokens of `text`: words, which may hold `::`, and the punctuation `.`, `(`,
/// `)`, `|`, `,` and `->`.
fn tokens(text: &str) -> Option<Vec<String>> {
    let mut out = Vec::new();
    let mut chars = text.char_indices().peekable();
    while let Some((i, ch)) = chars.next() {
        match ch {
            c if c.is_whitespace() => {}
            '.' | '(' | ')' | '|' | ',' => out.push(ch.to_string()),
            '-' if chars.next_if(|&(_, c)| c == '>').is_some() => out.push("->".into()),
            c if c.is_alphanumeric() || c == '_' => {
                let mut end = i + c.len_utf8();
                while let Some(&(j, c)) = chars.peek() {
                    if c.is_alphanumeric() || c == '_' || c == ':' {
                        end = j + c.len_utf8();
                        chars.next();
                    } else {
                        break;
                    }
                }
                out.push(text[i..end].to_string());
            }
            _ => return None,
        }
    }
    Some(out)
}

struct Reader {
    toks: Vec<String>,
    at: usize,
}

impl Reader {
    fn eat(&mut self, tok: &str) -> bool {
        let found = self.toks.get(self.at).is_some_and(|t| t == tok);
        if found {
            self.at += 1;
        }
        found
    }

    fn expect(&mut self, tok: &str) -> Option<()> {
        self.eat(tok).then_some(())
    }

    fn word(&mut self) -> Option<String> {
        let t = self.toks.get(self.at)?;
        if !t.starts_with(|c: char| c.is_alphanumeric() || c == '_') {
            return None;
        }
        self.at += 1;
        Some(t.clone())
    }

    fn target(&mut self, schema: &Schema) -> Option<(RelId, GrantTarget)> {
        let word = self.word()?;
        let (module, name) = qualified(&word)?;
        let (&rel, &relation) = schema.ids.relations.iter().find(|(r, _)| {
            let rd = schema.relation(**r);
            rd.name == name && schema.module(rd.module).name == module
        })?;
        if !self.eat(".") {
            let sel = GrantSel::Whole;
            return Some((rel, GrantTarget { relation, sel }));
        }
        let sel = if self.eat("(") {
            let mut cs = Vec::new();
            loop {
                cs.push(ctor_id(schema, rel, &self.word()?)?);
                if !self.eat("|") {
                    break;
                }
            }
            self.expect(")")?;
            cs.sort_unstable();
            GrantSel::Ctors(cs)
        } else {
            let word = self.word()?;
            match ctor_id(schema, rel, &word) {
                Some(c) => GrantSel::Ctors(vec![c]),
                None => GrantSel::Set(*schema.ids.sets.get(&(rel, word))?),
            }
        };
        Some((rel, GrantTarget { relation, sel }))
    }

    fn fields(&mut self, schema: &Schema, rel: RelId) -> Option<Vec<u64>> {
        let def = schema.relation(rel).elem_def;
        self.expect("(")?;
        let mut ids = Vec::new();
        if self.eat(")") {
            return Some(ids);
        }
        loop {
            let first = self.word()?;
            let id = if self.eat(".") {
                let c = schema.ty(def).ctor_index(&first)?;
                schema.ids.fields.get(&(def, Some(c), self.word()?))
            } else if schema.ty(def).ctors().len() == 1 {
                schema
                    .ids
                    .fields
                    .get(&(def, Some(0), first.clone()))
                    .or_else(|| schema.ids.fields.get(&(def, None, first)))
            } else {
                schema.ids.fields.get(&(def, None, first))
            };
            ids.push(*id?);
            if !self.eat(",") {
                break;
            }
        }
        self.expect(")")?;
        ids.sort_unstable();
        Some(ids)
    }
}
