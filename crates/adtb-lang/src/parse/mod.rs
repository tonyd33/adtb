mod adhoc;
mod decl;
mod expr;
mod op;
mod ty;
mod write;

use std::collections::HashSet;

use crate::ast::*;
use crate::lex::{self, Kw, Tok, Token};
use crate::span::{Diag, Span};

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, Default)]
pub struct Options {
    /// Accept the elisions documentation examples use: `c41e...` for a hash,
    /// `{..}` for a record's fields, `...` for an expression.
    pub elisions: bool,
}

/// A parse failure already reported as a diagnostic.
#[derive(Debug)]
pub(crate) struct Fail;

pub(crate) type PResult<T> = Result<T, Fail>;

/// The greatest height a parsed tree may reach, counting each operand, operator of a chain,
/// postfix, pattern, type, write, generator and `where` clause on the path to a leaf.
pub const MAX_DEPTH: u32 = 64;

/// A production the parser tries and backtracks from.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Attempt {
    Write,
    TypeCall,
}

pub(crate) struct Parser<'s> {
    src: &'s str,
    toks: Vec<Token>,
    pos: usize,
    diags: Vec<Diag>,
    opts: Options,
    depth: u32,
    peak: u32,
    failed: HashSet<(usize, u32, Attempt)>,
}

#[derive(Debug)]
pub struct Parsed<T> {
    pub value: Option<T>,
    pub diags: Vec<Diag>,
}

impl<T> Parsed<T> {
    pub fn ok(self) -> Result<T, Vec<Diag>> {
        match self.value {
            Some(v) if self.diags.is_empty() => Ok(v),
            _ => Err(self.diags),
        }
    }
}

fn run<'s, T>(
    src: &'s str,
    opts: Options,
    f: impl FnOnce(&mut Parser<'s>) -> PResult<T>,
) -> Parsed<T> {
    let (toks, diags) = lex::lex(src);
    let mut p = Parser {
        src,
        toks,
        pos: 0,
        diags,
        opts,
        depth: 0,
        peak: 0,
        failed: HashSet::new(),
    };
    let value = f(&mut p).ok();
    let value = match value {
        Some(v) if p.at(Tok::Eof) => Some(v),
        Some(_) => {
            p.error_here("expected the end of the text");
            None
        }
        None => None,
    };
    Parsed {
        value,
        diags: p.diags,
    }
}

pub fn parse_schema(src: &str, opts: Options) -> Parsed<SchemaFile> {
    run(src, opts, |p| {
        let module = p.module_header()?;
        let decls = p.items(Parser::decl)?;
        Ok(SchemaFile { module, decls })
    })
}

pub fn parse_migration(src: &str, opts: Options) -> Parsed<MigrationFile> {
    run(src, opts, |p| {
        let module = p.module_header()?;
        let operators = p.items(Parser::operator)?;
        Ok(MigrationFile { module, operators })
    })
}

pub fn parse_bundle(src: &str, opts: Options) -> Parsed<BundleFile> {
    run(src, opts, |p| {
        let header = p.bundle_header()?;
        let mut imports = Vec::new();
        while p.at_kw(Kw::Import) {
            imports.push(p.import()?);
        }
        let items = p.items(Parser::bundle_item)?;
        Ok(BundleFile {
            header,
            imports,
            items,
        })
    })
}

pub fn parse_adhoc(src: &str, opts: Options) -> Parsed<Vec<AdhocItem>> {
    run(src, opts, |p| {
        let mut first = true;
        let items = p.items(|p| {
            let start = p.tok();
            let item = p.adhoc_item()?;
            if matches!(item, AdhocItem::Expr(_)) && !first && !p.starts_with_keyword(start) {
                return Err(p.error_at(
                    start.span,
                    "a query that starts with neither a keyword nor `(` must be the first item or be parenthesised",
                ));
            }
            first = false;
            Ok(item)
        })?;
        if items.is_empty() {
            p.error_here("expected a statement");
            return Err(Fail);
        }
        Ok(items)
    })
}

pub fn parse_example(src: &str, opts: Options) -> Parsed<Vec<ExampleItem>> {
    run(src, opts, |p| {
        p.items(|p| {
            let start = p.tok();
            let item = p.example_item()?;
            let bare = matches!(item, ExampleItem::Adhoc(AdhocItem::Expr(_)));
            if bare && !p.first_on_line(start) && !p.starts_with_keyword(start) {
                return Err(p.error_at(
                    start.span,
                    "an example query that starts with neither a keyword nor `(` starts its own line",
                ));
            }
            Ok(item)
        })
    })
}

pub fn parse_expr(src: &str, opts: Options) -> Parsed<Expr> {
    run(src, opts, Parser::expr)
}

pub fn parse_type(src: &str, opts: Options) -> Parsed<Type> {
    run(src, opts, Parser::ty)
}

/// Words that may start a top-level item, used to resynchronise after an error.
const ITEM_WORDS: &[&str] = &[
    "add",
    "drop",
    "rename",
    "remove",
    "split",
    "merge",
    "nest",
    "unnest",
    "move",
    "rekey",
    "define",
    "redefine",
    "store",
    "unstore",
    "enable",
    "disable",
    "deprecate",
    "revert",
    "script",
    "install",
    "uninstall",
    "adopt",
    "grant",
    "revoke",
    "feed",
    "aggregate",
    "explain",
    "create",
    "alter",
    "map",
    "unmap",
];

impl<'s> Parser<'s> {
    fn tok(&self) -> Token {
        self.toks[self.pos]
    }

    fn peek(&self, n: usize) -> Token {
        self.toks[(self.pos + n).min(self.toks.len() - 1)]
    }

    fn at(&self, tok: Tok) -> bool {
        self.tok().tok == tok
    }

    fn at_kw(&self, kw: Kw) -> bool {
        self.at(Tok::Kw(kw))
    }

    fn text_of(&self, t: Token) -> &'s str {
        &self.src[t.span.range()]
    }

    fn is_word(&self, t: Token, w: &str) -> bool {
        t.tok == Tok::Lower && self.text_of(t) == w
    }

    fn at_word(&self, w: &str) -> bool {
        self.is_word(self.tok(), w)
    }

    fn peek_word(&self, n: usize, w: &str) -> bool {
        self.is_word(self.peek(n), w)
    }

    fn bump(&mut self) -> Token {
        let t = self.tok();
        if t.tok != Tok::Eof {
            self.pos += 1;
        }
        t
    }

    fn eat(&mut self, tok: Tok) -> bool {
        if self.at(tok) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn eat_kw(&mut self, kw: Kw) -> bool {
        self.eat(Tok::Kw(kw))
    }

    fn eat_word(&mut self, w: &str) -> bool {
        if self.at_word(w) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn prev_span(&self) -> Span {
        if self.pos == 0 {
            Span::default()
        } else {
            self.toks[self.pos - 1].span
        }
    }

    fn span_from(&self, start: Span) -> Span {
        start.to(self.prev_span())
    }

    fn error_at(&mut self, span: Span, msg: impl Into<String>) -> Fail {
        self.diags.push(Diag::new(span, msg));
        Fail
    }

    fn error_here(&mut self, msg: impl Into<String>) -> Fail {
        let t = self.tok();
        let found = match t.tok {
            Tok::Eof => "the end of the text".to_string(),
            _ => format!("`{}`", self.text_of(t)),
        };
        let msg = format!("{}, found {found}", msg.into());
        self.error_at(t.span, msg)
    }

    fn expect(&mut self, tok: Tok, what: &str) -> PResult<Token> {
        if self.at(tok) {
            Ok(self.bump())
        } else {
            Err(self.error_here(format!("expected {what}")))
        }
    }

    fn expect_kw(&mut self, kw: Kw) -> PResult<Token> {
        let what = format!("`{}`", format!("{kw:?}").to_lowercase());
        self.expect(Tok::Kw(kw), &what)
    }

    fn expect_word(&mut self, w: &str) -> PResult<Token> {
        if self.at_word(w) {
            Ok(self.bump())
        } else {
            Err(self.error_here(format!("expected `{w}`")))
        }
    }

    /// Runs `f` as production `what`, rewinding the position and dropping its diagnostics when
    /// it fails, and skipping it where it failed before at the same position and depth.
    fn attempt<T>(&mut self, what: Attempt, f: impl FnOnce(&mut Self) -> PResult<T>) -> Option<T> {
        let key = (self.pos, self.depth, what);
        if self.failed.contains(&key) {
            return None;
        }
        let (pos, peak) = (self.pos, self.peak);
        let n = self.diags.len();
        match f(self) {
            Ok(v) => Some(v),
            Err(Fail) => {
                self.pos = pos;
                self.peak = peak;
                self.diags.truncate(n);
                self.failed.insert(key);
                None
            }
        }
    }

    /// Runs `f` and restores the nesting depth it started at.
    fn scoped<T>(&mut self, f: impl FnOnce(&mut Self) -> PResult<T>) -> PResult<T> {
        let base = self.depth;
        let r = f(self);
        self.depth = base;
        r
    }

    /// Runs `f` one level deeper.
    fn nested<T>(&mut self, f: impl FnOnce(&mut Self) -> PResult<T>) -> PResult<T> {
        self.scoped(|p| {
            p.deeper()?;
            f(p)
        })
    }

    /// Adds a level of nesting until the end of the enclosing [`Parser::scoped`].
    fn deeper(&mut self) -> PResult<()> {
        self.depth += 1;
        self.reach(self.depth)
    }

    /// Notes that a leaf lies at `depth`, refusing the text when that is too deep.
    fn reach(&mut self, depth: u32) -> PResult<()> {
        self.peak = self.peak.max(depth);
        if depth > MAX_DEPTH {
            return Err(
                self.error_here(format!("the text nests more than {MAX_DEPTH} levels deep"))
            );
        }
        Ok(())
    }

    /// Parses a left-nested chain: `first`, then `link` while `more` holds, each link taking
    /// the chain so far.
    ///
    /// Postconditions:
    /// - The chain's height is the greatest height of its parts plus one per link, and the
    ///   nesting depth is unchanged.
    fn chain<T>(
        &mut self,
        first: impl FnOnce(&mut Self) -> PResult<T>,
        mut more: impl FnMut(&mut Self) -> bool,
        mut link: impl FnMut(&mut Self, T) -> PResult<T>,
    ) -> PResult<T> {
        let base = self.depth;
        let outer = self.peak;
        self.peak = base;
        let mut acc = first(self)?;
        let mut height = self.peak - base;
        while more(self) {
            self.peak = base;
            acc = link(self, acc)?;
            height = height.max(self.peak - base) + 1;
            self.reach(base + height)?;
        }
        self.peak = outer.max(base + height);
        Ok(acc)
    }

    fn ident(&mut self, t: Token) -> Ident {
        Ident {
            text: self.text_of(t).to_string(),
            span: t.span,
        }
    }

    fn lower(&mut self, what: &str) -> PResult<Ident> {
        let t = self.expect(Tok::Lower, what)?;
        Ok(self.ident(t))
    }

    fn upper(&mut self, what: &str) -> PResult<Ident> {
        let t = self.expect(Tok::Upper, what)?;
        Ok(self.ident(t))
    }

    fn at_field_word(&self) -> bool {
        matches!(self.tok().tok, Tok::Lower | Tok::Kw(_))
    }

    /// A word in field position, where reserved words are names too.
    fn field_word(&mut self) -> PResult<Ident> {
        if self.at_field_word() {
            let t = self.bump();
            Ok(self.ident(t))
        } else {
            Err(self.error_here("expected a field name"))
        }
    }

    fn qlower(&mut self, what: &str) -> PResult<QName> {
        let first = self.lower(what)?;
        if self.at(Tok::ColonColon) && self.peek(1).tok == Tok::Lower {
            self.bump();
            let name = self.lower(what)?;
            Ok(QName {
                module: Some(first),
                name,
            })
        } else {
            Ok(QName {
                module: None,
                name: first,
            })
        }
    }

    fn at_qupper(&self) -> bool {
        self.at(Tok::Upper)
            || (self.at(Tok::Lower)
                && self.peek(1).tok == Tok::ColonColon
                && self.peek(2).tok == Tok::Upper)
    }

    fn qupper(&mut self, what: &str) -> PResult<QName> {
        if self.at(Tok::Lower) && self.peek(1).tok == Tok::ColonColon {
            let module = self.lower(what)?;
            self.bump();
            let name = self.upper(what)?;
            Ok(QName {
                module: Some(module),
                name,
            })
        } else {
            let name = self.upper(what)?;
            Ok(QName { module: None, name })
        }
    }

    fn ctor_sel(&mut self) -> PResult<CtorSel> {
        if self.eat(Tok::LParen) {
            let mut names = vec![self.upper("a constructor name")?];
            while self.eat(Tok::Bar) {
                names.push(self.upper("a constructor name")?);
            }
            self.expect(Tok::RParen, "`)`")?;
            Ok(CtorSel::Many(names))
        } else {
            Ok(CtorSel::One(self.upper("a constructor name")?))
        }
    }

    fn at_ctor_sel_after_dot(&self) -> bool {
        self.at(Tok::Dot)
            && (self.peek(1).tok == Tok::Upper
                || (self.peek(1).tok == Tok::LParen && self.peek(2).tok == Tok::Upper))
    }

    fn relation_ref(&mut self) -> PResult<RelationRef> {
        let start = self.tok().span;
        let relation = self.qlower("a relation name")?;
        let sel = if self.at_ctor_sel_after_dot() {
            self.bump();
            Some(self.ctor_sel()?)
        } else {
            None
        };
        Ok(RelationRef {
            relation,
            sel,
            span: self.span_from(start),
        })
    }

    fn ctor_scope(&mut self) -> PResult<CtorScope> {
        Ok(match self.ctor_sel()? {
            CtorSel::One(c) => CtorScope::One(c),
            CtorSel::Many(cs) => CtorScope::Many(cs),
        })
    }

    fn ctor_path(&mut self) -> PResult<CtorPath> {
        let ty = self.qupper("a type name")?;
        self.expect(Tok::Dot, "`.`")?;
        let ctor = self.upper("a constructor name")?;
        Ok(CtorPath { ty, ctor })
    }

    fn text_lit(&mut self) -> PResult<String> {
        let t = self.expect(Tok::Text, "a text literal")?;
        Ok(lex::decode_text(self.text_of(t)))
    }

    fn int_lit(&mut self) -> PResult<u64> {
        let t = self.expect(Tok::Int, "an integer")?;
        let text = self.text_of(t);
        text.parse()
            .map_err(|_| self.error_at(t.span, "integer out of range"))
    }

    /// Skips the tokens covering source bytes up to `end`, which must end on a token boundary.
    fn skip_to_byte(&mut self, end: usize) -> PResult<()> {
        while self.tok().tok != Tok::Eof && (self.tok().span.start as usize) < end {
            if self.tok().span.end as usize > end {
                return Err(self.error_here("malformed name"));
            }
            self.bump();
        }
        Ok(())
    }

    /// Scans a run of bundle-name characters at the current token: `web-2.4`, `web`.
    fn scan_bundle_run(&self) -> Option<(usize, usize)> {
        let t = self.tok();
        if t.tok != Tok::Lower {
            return None;
        }
        let bytes = self.src.as_bytes();
        let start = t.span.start as usize;
        let mut end = start;
        while end < bytes.len()
            && (bytes[end].is_ascii_lowercase()
                || bytes[end].is_ascii_digit()
                || matches!(bytes[end], b'_' | b'-' | b'.'))
        {
            end += 1;
        }
        while end > start && bytes[end - 1] == b'.' {
            end -= 1;
        }
        Some((start, end))
    }

    fn split_bundle_name(run: &str) -> Option<(&str, &str)> {
        let dash = run.rfind('-')?;
        let (family, version) = (&run[..dash], &run[dash + 1..]);
        let version_ok = version.starts_with(|c: char| c.is_ascii_digit())
            && version.chars().all(|c| c.is_ascii_digit() || c == '.');
        let family_ok =
            family.starts_with(|c: char| c.is_ascii_lowercase()) && !family.contains('.');
        (version_ok && family_ok).then_some((family, version))
    }

    fn at_bundle_name(&self) -> bool {
        self.scan_bundle_run()
            .is_some_and(|(s, e)| Self::split_bundle_name(&self.src[s..e]).is_some())
    }

    fn bundle_name(&mut self) -> PResult<BundleName> {
        let Some((start, end)) = self.scan_bundle_run() else {
            return Err(self.error_here("expected a bundle name"));
        };
        let run = &self.src[start..end];
        let Some((family, version)) = Self::split_bundle_name(run) else {
            return Err(self.error_here("expected a bundle name such as `web-2.4`"));
        };
        let name = BundleName {
            family: family.to_string(),
            version: version.to_string(),
            span: Span::new(start, end),
        };
        self.skip_to_byte(end)?;
        Ok(name)
    }

    /// A bundle name or a bare family name, as after `grant call`.
    fn call_target(&mut self) -> PResult<CallTarget> {
        if self.at_bundle_name() {
            return Ok(CallTarget::Bundle(self.bundle_name()?));
        }
        let Some((start, end)) = self.scan_bundle_run() else {
            return Err(self.error_here("expected a bundle or family name"));
        };
        let family = self.src[start..end].to_string();
        if family.contains('.') {
            return Err(self.error_here("expected a bundle or family name"));
        }
        self.skip_to_byte(end)?;
        Ok(CallTarget::Family(family, Span::new(start, end)))
    }

    fn version(&mut self) -> PResult<Version> {
        let start = self.tok().span;
        let number = self.int_lit()?;
        let open = self.expect(Tok::LParen, "`(` and a hash")?;
        let bytes = self.src.as_bytes();
        let hstart = open.span.end as usize;
        let mut hend = hstart;
        while hend < bytes.len() && bytes[hend].is_ascii_hexdigit() {
            hend += 1;
        }
        let digits = &self.src[hstart..hend];
        let mut stop = hend;
        let elided = self.opts.elisions && self.src[hend..].starts_with("...");
        if elided {
            stop += 3;
        }
        if !elided && digits.len() != 64 {
            let span = Span::new(hstart, hend);
            return Err(self.error_at(span, "a version hash is exactly 64 hex digits"));
        }
        self.skip_to_byte(stop)?;
        self.expect(Tok::RParen, "`)`")?;
        Ok(Version {
            number,
            hash: (!elided).then(|| digits.to_string()),
            span: self.span_from(start),
        })
    }

    fn at_elision(&self) -> bool {
        self.opts.elisions
            && self.at(Tok::Dot)
            && self.peek(1).tok == Tok::Dot
            && self.peek(2).tok == Tok::Dot
    }

    fn module_header(&mut self) -> PResult<Ident> {
        self.expect_kw(Kw::Module)?;
        self.lower("a module name")
    }

    fn starts_with_keyword(&self, t: Token) -> bool {
        matches!(t.tok, Tok::Kw(_) | Tok::LParen)
    }

    fn first_on_line(&self, t: Token) -> bool {
        let before = &self.src[..t.span.start as usize];
        before.rfind('\n').map_or(before.trim().is_empty(), |i| {
            before[i + 1..].trim().is_empty()
        })
    }

    fn at_item_start(&self) -> bool {
        let t = self.tok();
        let kw_start = matches!(
            t.tok,
            Tok::Kw(
                Kw::Type
                    | Kw::Relation
                    | Kw::Set
                    | Kw::Fn
                    | Kw::Constraint
                    | Kw::Check
                    | Kw::Query
                    | Kw::Txn
                    | Kw::Bundle
                    | Kw::Import
                    | Kw::Module
                    | Kw::Transact
                    | Kw::Insert
                    | Kw::Update
                    | Kw::Delete
                    | Kw::From
                    | Kw::Temp
            )
        );
        let word_start = t.tok == Tok::Lower && ITEM_WORDS.contains(&self.text_of(t));
        (kw_start || word_start) && self.first_on_line(t)
    }

    /// Parses items until the end of the text, skipping to the next line that starts an item after an error.
    fn items<T>(&mut self, mut item: impl FnMut(&mut Self) -> PResult<T>) -> PResult<Vec<T>> {
        let mut out = Vec::new();
        let mut failed = false;
        while !self.at(Tok::Eof) {
            let before = self.pos;
            match item(self) {
                Ok(v) => out.push(v),
                Err(Fail) => {
                    failed = true;
                    if self.pos == before {
                        self.bump();
                    }
                    while !self.at(Tok::Eof) && !self.at_item_start() {
                        self.bump();
                    }
                }
            }
        }
        if failed { Err(Fail) } else { Ok(out) }
    }

    fn example_item(&mut self) -> PResult<ExampleItem> {
        let t = self.tok();
        match t.tok {
            Tok::Kw(Kw::Module) => Ok(ExampleItem::Module(self.module_header()?)),
            Tok::Kw(Kw::Bundle) => Ok(ExampleItem::BundleHeader(self.bundle_header()?)),
            Tok::Kw(Kw::Import) => Ok(ExampleItem::Import(self.import()?)),
            Tok::Kw(Kw::Query | Kw::Txn) => Ok(ExampleItem::BundleItem(self.bundle_item()?)),
            Tok::Kw(Kw::Fn) if self.at_fn_decl() => Ok(ExampleItem::Decl(self.decl()?)),
            Tok::Kw(Kw::Set) if !self.peek_word(1, "setting") => {
                Ok(ExampleItem::Decl(self.decl()?))
            }
            Tok::Kw(Kw::Type | Kw::Relation | Kw::Constraint | Kw::Check) => {
                Ok(ExampleItem::Decl(self.decl()?))
            }
            _ if self.at_word("feed") && self.peek(1).tok == Tok::Lower => {
                Ok(ExampleItem::BundleItem(self.bundle_item()?))
            }
            _ if self.at_word("aggregate") && self.peek(1).tok == Tok::Lower => {
                Ok(ExampleItem::Decl(self.decl()?))
            }
            _ if self.at_word("store") && self.peek(1).tok == Tok::Lower => {
                Ok(ExampleItem::Decl(self.decl()?))
            }
            _ if self.at_operator() => Ok(ExampleItem::Operator(self.operator()?)),
            _ => Ok(ExampleItem::Adhoc(self.adhoc_item()?)),
        }
    }
}
