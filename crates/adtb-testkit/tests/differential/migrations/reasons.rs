//! The reason each drawn broken operator is refused for, as the precondition it breaks states
//! it, and the reasons a whole operator whose preconditions read data may be refused for. A `*`
//! in a reason stands for any text without a backtick.

use adtb_testkit::schemas::{Kind, Op, Reach};

/// Returns the reasons a broken operator of `kind` drawn in shape `reach` may be refused for:
/// the precondition its draw breaks, or its names resolving.
fn broken(kind: Kind, reach: Option<Reach>) -> &'static [&'static str] {
    match (kind, reach) {
        (Kind::DropUnique | Kind::DropIndex, Some(Reach::DropUnknownName)) => &["no `zz` in scope"],
        (Kind::DropUnique, Some(Reach::DropAbsentClause)) => &["`*` has no unique (*)"],
        (Kind::DropIndex, Some(Reach::DropAbsentClause)) => &["`*` has no index (*)"],
        (kind, _) => broken_kind(kind),
    }
}

/// Returns the reasons a broken operator of `kind`, whose shape does not decide them, may be
/// refused for: none for a drop of a unique or index, whose every draw has a shape.
fn broken_kind(kind: Kind) -> &'static [&'static str] {
    match kind {
        Kind::AddModule => &["module `app` already exists"],
        Kind::DropModule => &["`drop module app` is refused while the module holds * `*`"],
        Kind::AddType | Kind::RenameType => &["module `app` already declares type `*`"],
        Kind::DropType => &["`drop type *` is refused while field `*` uses it"],
        Kind::AddRelation => &["module `app` already declares relation `*`"],
        Kind::DropRelation => &[
            "`drop relation *` is refused while field `*` references it",
            "module `app` declares no relation `zz`",
        ],
        Kind::RenameRelation => &["module `app` already declares `*`"],
        Kind::AddSet => &[
            "module `app` already declares set `*`",
            "`Zz` is not a constructor of `*`",
        ],
        Kind::DropSet => &[
            "`*` declares no named set `Zz`",
            "`*` is neither a constructor of `*` nor a named set of `*`",
        ],
        Kind::RenameSet => &["`*` already has a constructor or named set `*`"],
        Kind::AddConstructor | Kind::RenameConstructor | Kind::Split => {
            &["`*` is already a constructor of `*`"]
        }
        Kind::RemoveConstructor => &[
            "`remove constructor *` is refused: `*` is not deprecated in `*` by an earlier migration",
        ],
        Kind::Enable => &["`*` is already enabled"],
        Kind::Disable => &[
            "`disable` moves an enabled constructor back to declared; `*` is *",
            "`disable *` is refused: the delete action of field `*` retags into it",
        ],
        Kind::Deprecate => &[
            "`deprecate` applies only to an enabled constructor; `*` is *",
            "`deprecate *` is refused: the delete action of field `*` retags into it",
        ],
        Kind::Merge => &["the merged constructor `*` has no discriminator: *"],
        Kind::Nest => &[
            "the field of `*` holds an existing sum whose constructors have the names and fields of *",
        ],
        Kind::Unnest => &[
            "`*` holds one field of a sum, which unnest lifts",
            "the field of `*` holds no declared sum",
            "the field of `*` holds no declared sum without a header and parameters",
        ],
        Kind::AddField => &["`*` already has a field `*`"],
        Kind::RenameField => &[
            "`*` already has a field `*`",
            "`*` of `*` names `*` of `*` and of `*` as one element",
            "`*` of `*` names `*` of `*` and `*` of `*` as one element",
        ],
        Kind::DropField | Kind::MoveFieldDown | Kind::AddDefault => &["`*` has no field `*`"],
        Kind::MoveFieldUp => &["`*` already has a field `*`; the header cannot take it"],
        Kind::AlterField => &["`on delete` is allowed only on a `Key` field or an `Option` of one"],
        Kind::AddUnique | Kind::AddIndex => &[
            "no `zz` in scope",
            "`*` of `*` is one with `*`, which `*` already holds",
        ],
        Kind::AddCheck | Kind::Grant => &["no `zz` in scope"],
        Kind::DropUnique | Kind::DropIndex => &[],
        Kind::Store => &[
            "no `zz` in scope",
            "a stored view is a function with no parameters",
        ],
        Kind::Rekey => &[
            "key field `zz` is not a header field or a field of the only constructor of `*`",
            "`rekey relation *` is refused: field `*` references `*` and is not carried",
            "field `*` of `*`'s own type is typed `Key *` and is outside the new key; drop it in the same migration",
        ],
        Kind::DropDefault => &[
            "`*` has no default for `zz`",
            "`*` has no default for `*.zz`",
        ],
        Kind::DefineFn => &["`++` concatenates two texts or two byte strings, found I64 and Text"],
        Kind::RedefineFn => &["module `app` declares no fn `*`"],
        Kind::DropFn => &["module `app` declares no fn `*`", "no `*` in scope"],
        Kind::DefineAggregate => &[
            "module `app` already declares aggregate `*`",
            "`*` is declared twice in module `app`",
        ],
        Kind::DropAggregate => &["module `app` declares no aggregate `zz`"],
        Kind::AddConstraint => &["* has no field `zz`"],
        Kind::DropConstraint => &["module `app` declares no constraint `zz`"],
        Kind::DropCheck => &["module `app` declares no check `zz`"],
        Kind::Unstore => &["module `app` declares no store `*`"],
        Kind::AddComputed => &["expected I64, found Text"],
        Kind::RedefineComputed | Kind::DropComputed => &["`*` has no computed field `zz`"],
        Kind::Revoke => &["`revoke * from support` matches no grant `support` holds"],
        Kind::Script => {
            &["`zz` is a parameter of this text, but the call supplies no value for it"]
        }
    }
}

/// Returns the reasons an operator of `kind` replacing constructors that an object with no exact
/// image under it names may be refused for.
fn unimaged(kind: Kind) -> &'static [&'static str] {
    match kind {
        Kind::Split => &[
            "the split is refused: the default of `*` has no image under it",
            "`*` names `*`, which no new constructor keeps; state its replacements after the routing",
            "`*` names `*`, which `*` does not keep under that name",
            "`*` has no image under the split: its products do not hold its elements under one name",
            "check `*` has no image under the split*",
        ],
        Kind::Merge => &[
            "the merge is refused: the default of `*` has no image under it",
            "the merge is refused: `*` of `*` names `*` in its element `*`, which has no image under it",
            "the merge is refused: * `*` selects `*`, some but not all of its constructors",
            "`*` has no image under the merge: the routing does not carry `*` whole",
            "`*` has no image under the merge: its constructors' fields go to different fields",
            "`*` ranges over some but not all of the merged constructors and has no image under the merge",
            "check `*` has no image under the merge; drop or rewrite it in this or an earlier migration",
            "check `*` has no image under the merge: no one field of `*` tells its constructors apart",
        ],
        Kind::Nest => &[
            "the nest is refused: the default of `*` has no image under it",
            "the nest is refused: `*` of `*` names `*` in its element `*`, which has no image under it",
            "the nest is refused: * `*` selects `*`, some but not all of its constructors",
            "`*` has no image under the nest",
            "`*` ranges over some but not all of the nested constructors and has no image under the nest",
            "check `*` has no image under the nest: it ranges over a constructor the nest keeps",
        ],
        Kind::Unnest => &[
            "the unnest is refused: the default of `*` has no image under it",
            "the unnest is refused: `*` of `*` names `*` in its element `*`, which has no image under it",
            "the unnest is refused: * `*` selects `*`, some but not all of its constructors",
            "`*` has no image under the unnest",
        ],
        _ => &[],
    }
}

/// Returns the reasons a whole operator of `kind`, whose preconditions read data, may be
/// refused for: the checks at apply that read rows.
fn read(kind: Kind) -> &'static [&'static str] {
    match kind {
        Kind::AddUnique | Kind::AddCheck | Kind::AddConstraint | Kind::Rekey => &[
            "constraint * is violated by *",
            "rows of * share the key * of `*`",
        ],
        Kind::Disable => &["constructor relation * holds rows; it cannot be disabled"],
        Kind::RemoveConstructor => &["constructor relation * is not empty"],
        Kind::DropRelation | Kind::DropType => &["relation * is not empty"],
        Kind::Script if !super::SCRIPT_COMMITS => &["the engine does not run a `script` yet"],
        _ => &[],
    }
}

/// The reason a split whose routing does not carry whole a field with a default is refused for.
const UNCARRIED: &str = "the split is refused: `*` has the default `*` and its routing does not carry that field whole; drop the default first";

/// Returns whether `text` matches `pattern`, each `*` standing for any text without a backtick.
fn matches(pattern: &str, text: &str) -> bool {
    let Some((head, tail)) = pattern.split_once('*') else {
        return pattern == text;
    };
    let Some(rest) = text.strip_prefix(head) else {
        return false;
    };
    let ends = rest.char_indices().map(|(i, _)| i).chain([rest.len()]);
    for i in ends {
        if rest[..i].contains('`') {
            return false;
        }
        if matches(tail, &rest[i..]) {
            return true;
        }
    }
    false
}

fn stated_in(reasons: &[&str], text: &str) -> bool {
    reasons.iter().any(|r| matches(r, text))
}

/// Returns whether one of `messages` is a reason broken operator `op` is refused for, and each
/// of `own`, the messages at the operator itself, is one; else the reasons it may be refused for.
/// An unimaged operator is refused only for an object with no exact image, and one reaching
/// [`Reach::UncarriedDefault`] only for the default its routing does not carry.
pub(super) fn stated(op: &Op, messages: &[&str], own: &[&str]) -> Result<(), String> {
    let reasons: &[&str] = if op.reach() == Some(Reach::UncarriedDefault) {
        &[UNCARRIED]
    } else if op.unimaged() {
        unimaged(op.kind())
    } else {
        broken(op.kind(), op.reach())
    };
    let any = messages.iter().any(|m| stated_in(reasons, m));
    let each = own.iter().all(|m| stated_in(reasons, m));
    if any && each {
        Ok(())
    } else {
        Err(format!("{reasons:?}"))
    }
}

/// Returns whether `line` is a reason a whole operator of `kind` may be refused for the rows
/// it reads.
pub(super) fn data(kind: Kind, line: &str) -> bool {
    stated_in(read(kind), line)
}

/// The refusal of a revert giving a default back to a field a later migration dropped.
pub(super) const FILLS_NO_FIELD: &str =
    "the inverse of `*` names a field a later migration dropped";

/// The refusal of a revert giving a default back to a field that has another default.
pub(super) const ANOTHER_DEFAULT: &str =
    "the inverse `add default *` does not apply: `*` already has a default";

/// Returns the refusal of a revert whose inverse names a `what` a later migration dropped, `what`
/// a type, an index, a set, a constructor or a relation; a field's is [`FILLS_NO_FIELD`].
pub(super) fn later(what: &str) -> String {
    format!("the inverse of `*` names a {what} a later migration dropped")
}

/// The refusal of a revert whose inverse names, by a name a later migration gave another object,
/// an object it renamed.
pub(super) const RENAMED_APART: &str =
    "in the inverse of `*`, the name `*` stood for objects a later migration renamed apart";

/// The refusals of a revert of a migration a later one names an object of.
pub(super) const DEPENDS: [&str; 2] = [
    "`revert *` is refused while migration * depends on it: it names *",
    "`revert *` is refused while migration * depends on it: it names * `*`",
];

/// The refusal of a revert putting back a delete action retagging into a constructor a later
/// migration gave a field with no default.
pub(super) const RETAG_UNFILLED: &str =
    "field `*` of `*` is missing, and `*` has no default for it";

/// The refusal of a revert of a migration dropping a field it states no fill for.
pub(super) const NEEDS_FILL: &str = "the inverse of `drop field *` needs a fill: `fill * = e`";

/// Returns the refusals of an inverse `add field` or `add constructor` of a revert that a name a
/// later migration took refuses.
pub(super) fn taken(kind: Kind) -> Vec<String> {
    broken_kind(kind)
        .iter()
        .map(|why| format!("the inverse `*` does not apply: {why}"))
        .collect()
}

/// Returns whether `detail` is one of the refusals `predicted` of a revert of an earlier
/// migration.
pub(super) fn reverted(detail: &str, predicted: &[String]) -> bool {
    predicted.iter().any(|p| matches(p, detail))
}
