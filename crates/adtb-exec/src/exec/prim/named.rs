use adtb_lang::kernel::Prim;
use adtb_lang::schema::Schema;
use adtb_lang::types::Ty;

use super::super::Error;

/// Returns the refusal of primitive `p` on operands of types `args`.
pub(super) fn unsupported_on(schema: &Schema, p: Prim, args: &[Ty]) -> Error {
    let shown: Vec<String> = args.iter().map(|t| schema.show_ty(t)).collect();
    Error::Unsupported(format!("`{}` on {}", named(p), shown.join(" and ")))
}

/// Returns the name of primitive `p`: its operator, or the prelude function that calls it.
pub(super) fn named(p: Prim) -> &'static str {
    match p {
        Prim::Add => "+",
        Prim::Sub | Prim::Neg => "-",
        Prim::Mul => "*",
        Prim::Div => "/",
        Prim::Rem => "%",
        Prim::Concat => "++",
        Prim::Eq => "==",
        Prim::Ne => "!=",
        Prim::Lt => "<",
        Prim::Le => "<=",
        Prim::Gt => ">",
        Prim::Ge => ">=",
        Prim::CheckedAdd => "checked_add",
        Prim::CheckedSub => "checked_sub",
        Prim::CheckedMul => "checked_mul",
        Prim::CheckedDiv => "checked_div",
        Prim::CheckedRem => "checked_rem",
        Prim::CheckedNeg => "checked_neg",
        Prim::DecimalDiv => "div",
        Prim::Round => "round",
        Prim::Rescale => "rescale",
        Prim::Abs => "abs",
        Prim::Convert { checked: false } => "a conversion",
        Prim::Convert { checked: true } => "a checked conversion",
        Prim::Wrap => "a newtype's constructor",
        Prim::Unwrap => "value",
        Prim::KeyValue => "a key's value",
        Prim::ParseUuid => "uuid",
        Prim::ToText => "text",
        Prim::BytesLen => "bytes_len",
        Prim::ScalarLen => "scalar_len",
        Prim::GraphemeLen => "grapheme_len",
        Prim::StartsWith => "starts_with",
        Prim::EndsWith => "ends_with",
        Prim::Contains => "contains",
        Prim::Matches => "matches",
        Prim::Lower => "lower",
        Prim::Upper => "upper",
        Prim::Fold => "fold",
        Prim::Collate => "collate",
        Prim::Utf8 => "utf8",
        Prim::Bytes => "bytes",
        Prim::Micros => "micros",
        Prim::Millis => "millis",
        Prim::Seconds => "seconds",
        Prim::Minutes => "minutes",
        Prim::Hours => "hours",
        Prim::Days => "days",
        Prim::Weeks => "weeks",
        Prim::Months => "months",
        Prim::Years => "years",
        Prim::AddPeriod => "add_period",
        Prim::Zoned => "zoned",
        Prim::InZone => "in_zone",
        Prim::ToInstant => "to_instant",
        Prim::Span => "span",
        Prim::Start => "start",
        Prim::End => "end",
        Prim::Overlaps => "overlaps",
        Prim::Covers => "covers",
        Prim::Parse => "parse",
        Prim::Print => "print",
        Prim::ReadFile { .. } => "a file read",
    }
}
