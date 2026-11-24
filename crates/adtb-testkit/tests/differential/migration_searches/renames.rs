//! Every rename rewrites exactly the tokens that resolve to its object: each token of every
//! declaration is resolved by the checker before and after the rename, and only a token resolving
//! to the renamed object changes, to its new name, still resolving to it.

use std::sync::Mutex;

use super::*;

/// The kinds the search schedules first in a migration.
const RENAMES: [Kind; 5] = [
    Kind::RenameType,
    Kind::RenameRelation,
    Kind::RenameSet,
    Kind::RenameConstructor,
    Kind::RenameField,
];

/// The module that declares the twins of `app`'s sums.
const TWIN: &str = "wm";

/// Returns a migration of module `app` declaring, for each relation of `schema`, functions and
/// checks naming the relation, its type, constructors, fields and set qualified and
/// unqualified, in strings and comments, and under binders of the same spelling.
fn witnesses(schema: &Schema) -> String {
    let mut out = String::from("module app\n");
    for (r, _) in app_relations(schema) {
        let rel = schema.relation(r);
        let t = elem(schema, r);
        let ty = schema.ty(t);
        let name = &rel.name;
        let tn = &ty.name;
        let mut words = vec![name.clone(), tn.clone()];
        for c in ty.ctors() {
            words.push(c.name.clone());
            words.extend(c.fields.iter().map(|f| f.name.clone()));
        }
        words.extend(ty.header().iter().map(|f| f.name.clone()));
        words.extend(rel.sets.iter().map(|s| s.name.clone()));
        let words = words.join(" ");
        out.push_str(&format!(
            "define fn wq_{name} () -> Set ({{ a: Key {name}, s: Text }}) =\n  -- {words}\n  \
             from x in {name} select {{ a: key x, s: \"{words}\" }}\n"
        ));
        out.push_str(&format!(
            "define fn wa_{name} ({name}: I64) -> Set ({{ a: Key app::{name} }}) =\n  \
             from x in app::{name} where {name} == {name} select {{ a: key x }}\n"
        ));
        out.push_str(&format!(
            "define fn wt_{name} (x: {tn}, y: app::{tn}) -> I64 = 1\n"
        ));
        out.push_str(&format!(
            "define fn wr_{name} () -> Set ({{ a: Key {name} }}) =\n  \
             from {b} in {name} from x in {name} select {{ a: key x }}\n",
            b = schemas::BOUND
        ));
        for s in &rel.sets {
            out.push_str(&format!(
                "define fn ws_{name} () -> Set ({{ a: Key {name} }}) =\n  \
                 from x in {name}.{} select {{ a: key x }}\n",
                s.name
            ));
        }
        let sum = is_sum(schema, t);
        for (i, c) in ty.ctors().iter().enumerate() {
            let from = if sum {
                format!("{name}.{}", c.name)
            } else {
                name.clone()
            };
            let fields = if sum { &c.fields } else { ty.header() };
            if let Some(f) = fields.first() {
                let f = &f.name;
                out.push_str(&format!(
                    "define fn wc_{name}_{i} () -> Set ({{ a: Key {name} }}) =\n  \
                     from {f} in {from} where {f}.{f} == {f}.{f} select {{ a: key {f} }}\n"
                ));
            }
            let int = fields
                .iter()
                .find(|f| schema.show_ty(&f.ty) == "I64")
                .map(|f| &f.name);
            if let Some(g) = int {
                let b = schemas::BOUND;
                out.push_str(&format!(
                    "add check wk_{name}_{i} on {from} (match {g} | {b} -> {b} == {g})\n"
                ));
            }
        }
    }
    out
}

/// Returns a migration adding module [`TWIN`], declaring a sum of the name and constructors of
/// each sum of module `app` of `schema`, and a function of a selection of its first constructor.
fn twins(schema: &Schema) -> String {
    let mut out = format!("module {TWIN}\nadd module {TWIN}\n");
    for (r, _) in app_relations(schema) {
        let t = elem(schema, r);
        if !is_sum(schema, t) {
            continue;
        }
        let ty = schema.ty(t);
        let cs: Vec<&str> = ty.ctors().iter().map(|c| c.name.as_str()).collect();
        out.push_str(&format!("add type {} = {}\n", ty.name, cs.join(" | ")));
        out.push_str(&format!(
            "define fn wf_{0} (x: {0}.{1}) -> I64 = 1\n",
            ty.name, cs[0]
        ));
    }
    out
}

/// The object a rename names, by the space and catalog id it resolves to, with its old and new
/// names.
struct Target {
    id: (Space, u64),
    from: String,
    to: String,
}

/// Returns the type of module `app` named `name` in `schema`.
fn app_type(schema: &Schema, name: &str) -> Option<(TypeId, u64)> {
    let app = schema.module_named("app")?;
    schema
        .ids()
        .type_ids()
        .find(|(t, _)| schema.ty(*t).name == name && schema.ty(*t).module == app)
        .map(|(t, id)| (t, u64::from(id)))
}

/// Returns the relation of module `app` named `name` in `schema`.
fn app_relation(schema: &Schema, name: &str) -> Option<(RelId, u64)> {
    app_relations(schema)
        .into_iter()
        .find(|(r, _)| schema.relation(*r).name == name)
}

/// Returns the object rename `text` names in `schema`, as the generator states it.
fn target(schema: &Schema, text: &str) -> Option<Target> {
    let words: Vec<&str> = text.split_whitespace().collect();
    let to = (*words.last()?).to_owned();
    let ctor_of = |t: TypeId, c: &str| {
        let i = schema.ty(t).ctors().iter().position(|d| d.name == c)? as u32;
        schema
            .ids()
            .type_ctor_ids(t)
            .find(|(r, _)| r.index() == i)
            .map(|(_, id)| u64::from(id))
    };
    let (id, from) = match words.as_slice() {
        ["rename", "type", t, "to", _] => {
            ((Space::Shared, app_type(schema, t)?.1), (*t).to_owned())
        }
        ["rename", "relation", r, "to", _] => {
            ((Space::Shared, app_relation(schema, r)?.1), (*r).to_owned())
        }
        ["rename", "set", s, "of", r, "to", _] => {
            let (rel, _) = app_relation(schema, r)?;
            let id = schema.ids().set_id(rel, s)?;
            ((Space::Set, u64::from(id)), (*s).to_owned())
        }
        ["rename", "constructor", path, "to", _] => {
            let (t, c) = path.split_once('.')?;
            (
                (Space::Shared, ctor_of(app_type(schema, t)?.0, c)?),
                c.to_owned(),
            )
        }
        ["rename", "field", path, "to", _] => {
            let parts: Vec<&str> = path.split('.').collect();
            let (t, c, f) = match parts.as_slice() {
                [t, c, f] => (*t, Some(*c), *f),
                [t, f] => (*t, None, *f),
                _ => return None,
            };
            let (def, _) = app_type(schema, t)?;
            let ci = match c {
                Some(c) => Some(schema.ty(def).ctors().iter().position(|d| d.name == c)? as u32),
                None => None,
            };
            let id = schema
                .ids()
                .type_field_ids(def)
                .find(|(r, _)| field_name(schema, *r) == (ci, f.to_owned()))
                .map(|(_, id)| u64::from(id))?;
            ((Space::Field, id), f.to_owned())
        }
        _ => return None,
    };
    Some(Target { id, from, to })
}

/// Returns, by start and end, the ids each name of `names` resolves to.
fn resolved_at(
    names: &adtb_lang::canon::Names,
    len: usize,
) -> BTreeMap<(usize, usize), BTreeSet<(Space, u64)>> {
    names
        .ids_within(0..len)
        .map(|(r, ids, _)| ((r.start, r.end), ids.clone()))
        .collect()
}

/// The ids each declaring name of a source's files resolves to, by file and the token's start
/// and end.
type Declared = BTreeMap<usize, BTreeMap<(usize, usize), BTreeSet<(Space, u64)>>>;

/// Returns the declaring name of each type, constructor, field, relation and named set of `s`,
/// each the first token spelling its name within the span of its declaration.
fn declared(s: &Source) -> Result<Declared, String> {
    let compiled = s.check().map_err(|e| format!("{e:?}"))?;
    let schema = compiled.schema();
    let ids = schema.ids();
    let mut sites: Vec<(FileId, adtb_lang::span::Span, String, (Space, u64))> = Vec::new();
    for (t, id) in ids.type_ids() {
        let ty = schema.ty(t);
        sites.push((
            ty.file,
            ty.span,
            ty.name.clone(),
            (Space::Shared, u64::from(id)),
        ));
    }
    for (c, id) in ids.ctor_ids() {
        let ty = schema.ty(c.def());
        let Some(ctor) = ty.ctors().get(c.index() as usize) else {
            continue;
        };
        sites.push((
            ty.file,
            ctor.span,
            ctor.name.clone(),
            (Space::Shared, u64::from(id)),
        ));
    }
    for (f, id) in ids.field_ids() {
        let ty = schema.ty(f.def());
        let def = match f.ctor() {
            Some(c) => ty
                .ctors()
                .get(c as usize)
                .and_then(|c| c.fields.get(f.pos() as usize)),
            None => ty.header().get(f.pos() as usize),
        };
        let Some(def) = def else {
            continue;
        };
        sites.push((
            ty.file,
            def.span,
            def.name.clone(),
            (Space::Field, u64::from(id)),
        ));
    }
    for (r, id) in ids.relation_ids() {
        let rel = schema.relation(r);
        sites.push((
            rel.file,
            rel.span,
            rel.name.clone(),
            (Space::Shared, u64::from(id)),
        ));
    }
    for (r, name, id) in ids.set_ids() {
        if let Some(set) = schema.relation(r).sets.iter().find(|x| x.name == name) {
            let (file, span) = set.declared_at();
            sites.push((file, span, name.to_owned(), (Space::Set, u64::from(id))));
        }
    }
    let mut out = Declared::new();
    for (file, span, name, id) in sites {
        let i = file.0 as usize;
        if i >= s.decls.len() {
            continue;
        }
        let text = file_text(s, i);
        let within = span.range();
        let site = tokens(&text)
            .into_iter()
            .map(|(_, r)| r)
            .find(|r| within.start <= r.start && r.end <= within.end && text[r.clone()] == name);
        if let Some(r) = site {
            out.entry(i)
                .or_default()
                .entry((r.start, r.end))
                .or_default()
                .insert(id);
        }
    }
    Ok(out)
}

/// Returns each token of the declarations of `before` the rename `t` that left `after` rewrites
/// other than to its new name where it resolves to the renamed object, and keeps where it does
/// not, counting in `reached` the kinds of token it judged.
fn misrewritten(
    before: &Source,
    after: &Source,
    t: &Target,
    reached: &mut BTreeMap<&'static str, usize>,
) -> Vec<String> {
    let mut out = Vec::new();
    let (nb, na) = match (before.names(), after.names()) {
        (Ok(b), Ok(a)) => (b, a),
        (b, a) => {
            return vec![format!(
                "the names do not resolve: {:?} {:?}",
                b.err(),
                a.err()
            )];
        }
    };
    let (db, da) = match (declared(before), declared(after)) {
        (Ok(b), Ok(a)) => (b, a),
        (b, a) => {
            return vec![format!(
                "the sources do not check: {:?} {:?}",
                b.err(),
                a.err()
            )];
        }
    };
    if before.decls.len() != after.decls.len() {
        return vec![format!(
            "{} declarations before the rename, {} after",
            before.decls.len(),
            after.decls.len()
        )];
    }
    let mut count = |what| *reached.entry(what).or_default() += 1;
    let word = |s: &str| {
        s.split(|c: char| !c.is_alphanumeric() && c != '_')
            .any(|w| w == t.from)
    };
    for i in 0..before.decls.len() {
        let (tb, ta) = (file_text(before, i), file_text(after, i));
        let (kb, ka) = (tokens(&tb), tokens(&ta));
        let at = format!(
            "[{}] {}",
            before.decls[i].module,
            tb.lines().next().unwrap_or_default()
        );
        if kb.len() != ka.len() || before.decls[i].module != after.decls[i].module {
            out.push(format!(
                "{at}: {} tokens before the rename, {} after:\n{tb}\n{ta}",
                kb.len(),
                ka.len()
            ));
            continue;
        }
        let (mut rb, mut ra) = (resolved_at(&nb[i], tb.len()), resolved_at(&na[i], ta.len()));
        for (to, from) in [(&mut rb, &db), (&mut ra, &da)] {
            for (r, ids) in from.get(&i).into_iter().flatten() {
                to.entry(*r).or_default().extend(ids);
            }
        }
        let none = BTreeSet::new();
        let unresolved = |k: usize| {
            rb.get(&(kb[k].1.start, kb[k].1.end))
                .is_none_or(BTreeSet::is_empty)
        };
        let binds = |k: usize| binding(&kb, k);
        let mut ends = (0, 0);
        for (k, ((tok, b), (_, a))) in kb.iter().zip(&ka).enumerate() {
            let (gb, ga) = (&tb[ends.0..b.start], &ta[ends.1..a.start]);
            if gb != ga {
                out.push(format!(
                    "{at}: the text between tokens changes: `{gb}` / `{ga}`"
                ));
            } else if gb.contains("--") && word(gb) {
                count("a comment spelling the name kept");
            }
            ends = (b.end, a.end);
            let (sb, sa) = (&tb[b.clone()], &ta[a.clone()]);
            let (ib, ia) = (
                rb.get(&(b.start, b.end)).unwrap_or(&none),
                ra.get(&(a.start, a.end)).unwrap_or(&none),
            );
            let ours = ib.contains(&t.id);
            match (sb == sa, ours) {
                (true, true) => out.push(format!("{at}: `{sb}` of the renamed object is kept")),
                (true, false) if ib != ia => out.push(format!(
                    "{at}: `{sb}` resolves to {ib:?} before the rename and {ia:?} after: `{}`",
                    &ta[a.start.saturating_sub(40)..(a.end + 20).min(ta.len())]
                )),
                (true, false) => {
                    if *tok == Tok::Text && word(sb) {
                        count("a string spelling the name kept");
                    } else if sb == t.from && ib.is_empty() {
                        let bound = binds(k)
                            || (0..k)
                                .any(|j| &tb[kb[j].1.clone()] == sb && unresolved(j) && binds(j));
                        if selected(&kb, k) {
                            out.push(format!(
                                "{at}: `{sb}` after `.` or `::` resolves to nothing"
                            ));
                        } else if labels(&tb, &kb, k) {
                            count("a record label spelling the name kept");
                        } else if bound {
                            count("a binder spelling the name kept");
                        } else {
                            let near = &tb[b.start.saturating_sub(40)..(b.end + 20).min(tb.len())];
                            out.push(format!(
                                "{at}: `{sb}` resolves to nothing and nothing before it binds it: `{near}`"
                            ));
                        }
                    } else if sb == t.from && before.decls[i].module == TWIN {
                        count("an object of the name in another module kept");
                    } else if sb == t.from {
                        count("another object of the name kept");
                    }
                }
                (false, false) => {
                    out.push(format!("{at}: `{sb}`, resolving to {ib:?}, becomes `{sa}`"));
                }
                (false, true) if sa != t.to || ia != ib => out.push(format!(
                    "{at}: `{sb}` becomes `{sa}` resolving to {ia:?}, not `{}` resolving to {ib:?}",
                    t.to
                )),
                (false, true) => {
                    let qualified = k > 0 && kb[k - 1].0 == Tok::ColonColon;
                    count(if qualified {
                        "a qualified name rewritten"
                    } else {
                        "an unqualified name rewritten"
                    });
                }
            }
        }
        let (gb, ga) = (&tb[ends.0..], &ta[ends.1..]);
        if gb != ga {
            out.push(format!(
                "{at}: the text after the last token changes: `{gb}` / `{ga}`"
            ));
        }
    }
    out
}

/// Returns whether token `k` of `toks` follows `.` or `::`, where only a schema name stands.
fn selected(toks: &[(Tok, Range<usize>)], k: usize) -> bool {
    k > 0 && matches!(toks[k - 1].0, Tok::Dot | Tok::ColonColon)
}

/// Returns whether token `k` of `toks` labels a field of an anonymous record: it stands before
/// `:` right inside `{` or after `,`, and no constructor names that `{`, or it follows `key` in a
/// declaration whose first word is `store`.
fn labels(text: &str, toks: &[(Tok, Range<usize>)], k: usize) -> bool {
    let stored = text
        .trim_start()
        .lines()
        .nth(1)
        .is_some_and(|l| l.starts_with("store "));
    if stored && k > 0 && toks[k - 1].0 == Tok::Kw(Kw::Key) {
        return true;
    }
    let label = k > 0
        && matches!(toks[k - 1].0, Tok::LBrace | Tok::Comma)
        && toks.get(k + 1).is_some_and(|t| t.0 == Tok::Colon);
    let mut depth = 0;
    for j in (0..k).rev() {
        match toks[j].0 {
            Tok::RBrace => depth += 1,
            Tok::LBrace if depth > 0 => depth -= 1,
            Tok::LBrace => return label && (j == 0 || toks[j - 1].0 != Tok::Upper),
            _ => {}
        }
    }
    false
}

/// Returns whether token `k` of `toks` stands where a name is bound: after `from`, `let`, `|`,
/// `(` or `,`, or before `->`, `in` or `=`.
fn binding(toks: &[(Tok, Range<usize>)], k: usize) -> bool {
    let before = k.checked_sub(1).map(|j| toks[j].0);
    let after = toks.get(k + 1).map(|t| t.0);
    matches!(
        before,
        Some(Tok::Kw(Kw::From | Kw::Let) | Tok::Bar | Tok::LParen | Tok::Comma)
    ) || matches!(after, Some(Tok::Arrow | Tok::Kw(Kw::In) | Tok::Eq))
}

/// Returns the refusal of each capture the rename `op` makes in `s`: a `rename field` to
/// [`schemas::BOUND`] in a witness check whose `match` binds that name beside the field, and a
/// `rename relation` to it in the witness function whose `from` binds that name before naming
/// the relation again.
fn captures(s: &Source, op: &str) -> Vec<String> {
    let b = schemas::BOUND;
    if let Some(old) = op
        .strip_prefix("rename relation ")
        .and_then(|r| r.strip_suffix(&format!(" to {b}")))
    {
        return s
            .decls
            .iter()
            .filter(|d| {
                d.text
                    .contains(&format!("from {b} in {old} from x in {old} "))
            })
            .filter_map(|d| d.text.strip_prefix("fn ")?.split_whitespace().next())
            .map(|n| format!("`{b}` in fn `{n}` would name the local binder `{b}`"))
            .collect();
    }
    let Some(old) = op
        .strip_prefix("rename field ")
        .and_then(|r| r.strip_suffix(&format!(" to {b}")))
        .and_then(|path| path.rsplit('.').next())
    else {
        return Vec::new();
    };
    s.decls
        .iter()
        .filter(|d| d.text.contains(&format!("| {b} -> {b} == {old})")))
        .filter_map(|d| d.text.strip_prefix("check ")?.split_whitespace().next())
        .map(|n| format!("`{b}` in check `{n}` would name the local binder `{b}`"))
        .collect()
}

/// Returns the source `text` of module `app` leaves `s`, or why the checker refuses it.
fn migrated(s: &Source, text: &str) -> Result<Source, String> {
    let file = parse_migration(text, Options::default())
        .ok()
        .map_err(|ds| format!("{ds:?}"))?;
    s.migrate(&file, text).map_err(|f| match f {
        adtb_lang::evolve::Failed::Refused(r) => {
            let ops = r.operators.iter().map(|d| d.message.clone());
            let recheck = r.recheck.iter().map(|d| d.diag.message.clone());
            ops.chain(recheck).collect::<Vec<_>>().join("\n")
        }
        adtb_lang::evolve::Failed::Broken(d) => format!("the model fails: {}", d.message),
    })
}

/// Run the rename search over the case of `seed`.
fn search(
    seed: u64,
    at: &mut Option<Failed>,
    reached: &Mutex<BTreeMap<&'static str, usize>>,
) -> Option<Failed> {
    let failed = |k, log: &[String], text: &str, finding: String| {
        Some(Failed {
            seed,
            k,
            log: log.to_vec(),
            text: text.to_owned(),
            finding,
        })
    };
    let mut walk = match Walk::new(seed) {
        Ok(w) => w,
        Err(e) => return failed(0, &[], "", e),
    };
    let Ok(compiled) = walk.source.check() else {
        return failed(0, &walk.log, "", "the schema does not check".into());
    };
    let schema = compiled.schema();
    for text in [witnesses(schema), twins(schema)] {
        match walk.apply(&text) {
            Ok(Some(_)) => {}
            Ok(None) => {
                let m = parse_migration(&text, Options::default()).ok().ok();
                let why = m.map(|f| walk.source.migrate(&f, &text).err());
                return failed(
                    0,
                    &walk.log,
                    &text,
                    format!("the witnesses are refused: {why:?}"),
                );
            }
            Err(e) => return failed(0, &walk.log, &text, e),
        }
    }
    let mut seen = BTreeMap::new();
    for k in 0..migrations() {
        let kind = scheduled(&RENAMES, seed, k);
        let Some(m) = walk.draw(k, kind) else {
            continue;
        };
        let text = m.text();
        *at = failed(k, &walk.log, &text, String::new());
        let mut before = walk.source.clone();
        let mut prefix = String::from("module app\n");
        for o in m.ops() {
            prefix.push_str(o.text());
            prefix.push('\n');
            let renames = RENAMES.contains(&o.kind());
            let after = match migrated(&walk.source, &prefix) {
                Ok(a) => a,
                Err(why) if captures(&before, o.text()).contains(&why) => {
                    let by = if o.kind() == Kind::RenameRelation {
                        "a capture by a relation's name refused"
                    } else {
                        "a capture refused"
                    };
                    *seen.entry(by).or_default() += 1;
                    break;
                }
                Err(why) if renames => {
                    return failed(
                        k,
                        &walk.log,
                        &text,
                        format!("`{}` is refused: {why}", o.text()),
                    );
                }
                Err(_) => break,
            };
            if renames {
                let Ok(compiled) = before.check() else {
                    break;
                };
                let schema = compiled.schema();
                let Some(t) = target(schema, o.text()) else {
                    return failed(k, &walk.log, &text, format!("`{}` names nothing", o.text()));
                };
                let wrong = misrewritten(&before, &after, &t, &mut seen);
                if !wrong.is_empty() {
                    return failed(
                        k,
                        &walk.log,
                        &text,
                        format!("`{}`:\n{}", o.text(), wrong.join("\n")),
                    );
                }
                *seen.entry("a rename judged").or_default() += 1;
            }
            before = after;
        }
        match walk.apply(&text) {
            Ok(Some(_)) => walk.case = m.applied(seed ^ k, 0),
            Ok(None) => {}
            Err(e) => return failed(k, &walk.log, &text, e),
        }
    }
    let mut r = reached
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    for (what, n) in seen {
        *r.entry(what).or_default() += n;
    }
    None
}

#[test]
fn a_rename_rewrites_exactly_the_tokens_that_resolve_to_its_object() {
    let reached = Mutex::new(BTreeMap::new());
    let failures = each_seed(|seed, at| search(seed, at, &reached));
    report("renames", &failures);
    floors(
        reached,
        &[
            "a rename judged",
            "an unqualified name rewritten",
            "a qualified name rewritten",
            "a string spelling the name kept",
            "a comment spelling the name kept",
            "a binder spelling the name kept",
            "another object of the name kept",
            "an object of the name in another module kept",
            "a capture refused",
            "a capture by a relation's name refused",
        ],
    );
}
