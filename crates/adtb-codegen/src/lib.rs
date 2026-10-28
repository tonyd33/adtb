//! Rust code generation from a schema's migration log and bundle files: the log, each bundle's
//! file, error type and typed callables, and the host types of the schema, over the in-process
//! API of `adtb`.

mod emit;
mod names;

use std::fmt::{self, Write};
use std::path::Path;

use adtb_lang::signature::{self, CallKind, Signature};
use adtb_lang::types::Ty;

use emit::Module;
use names::{camel, lower_type_name};

/// The edition whose error sums generated bundle error types follow.
pub const EDITION: &str = "2026-10";

/// Why code generation failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    pub message: String,
}

impl Error {
    pub(crate) fn new(message: impl Into<String>) -> Error {
        Error {
            message: message.into(),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

/// A bundle file and the Rust module its generated items go in.
#[derive(Clone, Copy, Debug)]
pub struct BundleSource<'a> {
    pub module: &'a str,
    pub text: &'a str,
}

/// Returns the Rust source of module `schema`, holding `LOG` and the host types of the schema
/// `log` builds, and of one module per bundle of `bundles`, holding `BUNDLE`, `Error` and one
/// value per callable.
///
/// Fail when the log does not replay, a bundle does not check at the version it pins or names
/// another edition than [`EDITION`], a signature holds a type with no host type, or two items
/// of one module take one Rust name.
pub fn generate(log: &[&str], bundles: &[BundleSource<'_>]) -> Result<String, Error> {
    let source = signature::replay(log.iter().copied()).map_err(|e| {
        Error::new(format!(
            "migration {} does not apply: {}",
            e.migration, e.message
        ))
    })?;
    let latest = source.schema().map_err(|ds| {
        let ms: Vec<&str> = ds.iter().map(|d| d.diag.message.as_str()).collect();
        Error::new(ms.join("\n"))
    })?;
    let mut schema = Module::new(&latest);
    schema.declare("LOG", "the log")?;
    schema.user_items()?;
    let mut out = String::from("pub mod schema {\n    pub const LOG: &[&str] = &[\n");
    for m in log {
        writeln!(out, "        {m:?},").expect("a String takes writes");
    }
    out.push_str("    ];\n\n");
    out.push_str(&indent(&schema.out));
    out.push_str("}\n");
    for b in bundles {
        let text = bundle(log, &schema, b)?;
        write!(out, "\npub mod {} {{\n{}}}\n", b.module, indent(&text))
            .expect("a String takes writes");
    }
    Ok(out)
}

/// Returns the text of every `.adt` file of `dir`, in file name order: a log as a schema
/// directory keeps it.
pub fn read_log(dir: &Path) -> std::io::Result<Vec<String>> {
    let mut paths = Vec::new();
    for e in std::fs::read_dir(dir)? {
        let p = e?.path();
        if p.extension().is_some_and(|x| x == "adt") {
            paths.push(p);
        }
    }
    paths.sort();
    paths.iter().map(std::fs::read_to_string).collect()
}

/// Returns the items of the module of bundle `b`: re-exporting `schema` when it pins the
/// version `log` ends at, else with a module `types` of the types at the version it pins.
fn bundle(log: &[&str], schema: &Module<'_>, b: &BundleSource<'_>) -> Result<String, Error> {
    let failed = |m: String| Error::new(format!("bundle module `{}`: {m}", b.module));
    let header = adtb_lang::parse::parse_bundle(b.text, adtb_lang::parse::Options::default())
        .ok()
        .map_err(|ds| failed(ds.iter().map(|d| d.message.as_str()).collect()))?
        .header;
    if header.edition != EDITION {
        return Err(failed(format!(
            "edition \"{}\" is not the edition \"{EDITION}\" this generator emits errors of",
            header.edition
        )));
    }
    let pins = header.pins.number;
    let at = usize::try_from(pins)
        .ok()
        .filter(|&n| n <= log.len())
        .ok_or_else(|| {
            failed(format!(
                "it pins version {pins}, and the log holds {} migrations",
                log.len()
            ))
        })?;
    let source = signature::replay(log[..at].iter().copied())
        .map_err(|e| failed(format!("migration {}: {}", e.migration, e.message)))?;
    let checked = signature::bundle(&source, b.text).map_err(failed)?;
    let name = format!("{}-{}", header.name.family, header.name.version);
    let mut out = String::new();
    let mut m = if at == log.len() {
        out.push_str("pub use super::schema::*;\n\n");
        Module::after(&checked.schema, schema)
    } else {
        let mut types = Module::new(&checked.schema);
        types.user_items().map_err(|e| failed(e.message))?;
        write!(
            out,
            "pub use self::types::*;\n\npub mod types {{\n{}}}\n\n",
            indent(&types.out)
        )
        .expect("a String takes writes");
        Module::after(&checked.schema, &types)
    };
    for (n, what) in [
        ("BUNDLE", "the bundle value"),
        ("Bundle", "the bundle type"),
        ("Error", "the bundle's error type"),
    ] {
        m.declare(n, what).map_err(|e| failed(e.message))?;
    }
    let mut calls = String::new();
    for c in &checked.calls {
        callable(&mut m, c, &mut calls).map_err(|e| failed(e.message))?;
    }
    write!(
        out,
        "pub const BUNDLE: Bundle = Bundle;\n\n\
         #[derive(Clone, Copy, Debug, PartialEq, Eq)]\npub struct Bundle;\n\n\
         impl ::adtb::BundleDef for Bundle {{\n    type Error = Error;\n    \
         const NAME: &'static str = {name:?};\n    const TEXT: &'static str = {:?};\n    \
         const PINS: u64 = {pins};\n}}\n\n{ERROR}\n",
        b.text
    )
    .expect("a String takes writes");
    out.push_str(&m.out);
    out.push_str(&calls);
    Ok(out)
}

const ERROR: &str = "#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Retry { reason: ::adtb::RetryReason },
    Rejected { reason: ::adtb::RejectReason },
    Fatal { reason: ::adtb::FatalReason },
}

impl From<::adtb::Error> for Error {
    fn from(e: ::adtb::Error) -> Error {
        match e {
            ::adtb::Error::Retry { reason } => Error::Retry { reason },
            ::adtb::Error::Rejected { reason } => Error::Rejected { reason },
            ::adtb::Error::Fatal { reason } => Error::Fatal { reason },
        }
    }
}
";

/// Emit the value of callable `c` into `out`, and the anonymous types of its signature into `m`.
fn callable(m: &mut Module<'_>, c: &Signature, out: &mut String) -> Result<(), Error> {
    let base = camel(&c.name);
    m.declare(&c.name, &format!("callable `{}`", c.name))?;
    let mut args = Vec::new();
    for (p, ty) in &c.params {
        args.push(m.host(ty, &format!("{base}{}Arg", camel(p)), &[])?);
    }
    let args = match args.as_slice() {
        [one] => format!("({one},)"),
        all => format!("({})", all.join(", ")),
    };
    let (output, in_txn, convert) = match (c.kind, &c.result) {
        (CallKind::Txn, ty) => {
            let t = m.host(ty, &format!("{base}Value"), &[])?;
            (
                format!("::adtb::Committed<{t}>"),
                format!("::adtb::Pending<{t}>"),
                ("committed", "pending"),
            )
        }
        (_, Ty::Set(e) | Ty::NonEmpty(e)) => {
            let t = format!(
                "::adtb::Rows<{}, Error>",
                m.host(e, &format!("{base}Row"), &[])?
            );
            (t.clone(), t, ("rows", "rows_in_txn"))
        }
        (_, ty) => {
            let t = m.host(ty, &format!("{base}Value"), &[])?;
            (t.clone(), t, ("value", "value_in_txn"))
        }
    };
    if lower_type_name(&c.name) {
        out.push_str(
            "#[expect(non_camel_case_types, reason = \"a callable takes the name its bundle \
             file declares\")]\n",
        );
    }
    write!(
        out,
        "#[derive(Clone, Copy, Debug, PartialEq, Eq)]\npub struct {name};\n\n\
         impl ::adtb::Callable for {name} {{\n    type Bundle = Bundle;\n    type Args = {args};\n    \
         type Output = {output};\n    type InTxn = {in_txn};\n    const ORDINAL: u32 = {ordinal};\n\n    \
         fn output(c: ::adtb::Called) -> Result<Self::Output, ::adtb::Error> {{\n        \
         ::adtb::typed::{out_fn}(c)\n    }}\n\n    \
         fn in_txn(c: ::adtb::InTxn) -> Result<Self::InTxn, ::adtb::Error> {{\n        \
         ::adtb::typed::{in_fn}(c)\n    }}\n}}\n\n",
        name = names::ident(&c.name),
        ordinal = c.ordinal,
        out_fn = convert.0,
        in_fn = convert.1,
    )
    .expect("a String takes writes");
    Ok(())
}

/// Returns `text` indented one level, blank lines left blank.
fn indent(text: &str) -> String {
    let mut out = String::new();
    for line in text.lines() {
        if !line.is_empty() {
            out.push_str("    ");
            out.push_str(line);
        }
        out.push('\n');
    }
    out
}
