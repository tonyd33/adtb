//! The reason each drawn broken operator is refused for, as the precondition it breaks states
//! it, and the reasons a whole operator whose preconditions read data may be refused for. A `*`
//! in a reason stands for any text without a backtick.

use adtb_testkit::schemas::Kind;

/// Returns the reasons a broken operator of `kind` may be refused for: the precondition its
/// draw breaks, or its names resolving.
fn broken(kind: Kind) -> &'static [&'static str] {
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
        Kind::AddField | Kind::RenameField => &["`*` already has a field `*`"],
        Kind::DropField | Kind::MoveFieldDown | Kind::AddDefault => &["`*` has no field `*`"],
        Kind::MoveFieldUp => &["`*` already has a field `*`; the header cannot take it"],
        Kind::AlterField => &["`on delete` is allowed only on a `Key` field or an `Option` of one"],
        Kind::AddUnique => &["no `zz` in scope", "`*` is declared twice"],
        Kind::AddIndex => &["no `zz` in scope", "`*` declares `index *` twice"],
        Kind::AddCheck | Kind::Grant => &["no `zz` in scope"],
        Kind::Store => &[
            "no `zz` in scope",
            "a stored view is a function with no parameters",
        ],
        Kind::DropUnique => &["`*` has no unique (zz)"],
        Kind::DropIndex => &["`*` has no index (zz)"],
        Kind::Rekey => &[
            "key field `zz` is not a header field or a field of the only constructor of `*`",
            "`rekey relation *` is refused: field `*` references `*` and is not carried",
            "field `*` of `*`'s own type is typed `Key *` and is outside the new key; drop it in the same migration",
        ],
        Kind::DropDefault => &["`*` has no default for `zz`"],
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

/// Returns whether one of `messages` is a reason a broken operator of `kind` is refused for, and
/// each of `own`, the messages at the operator itself, is one; else the reasons it may be
/// refused for. An operator `unimaged` is refused only for an object with no exact image.
pub(super) fn stated(
    (kind, unimaged_shape): (Kind, bool),
    messages: &[&str],
    own: &[&str],
) -> Result<(), String> {
    let reasons = if unimaged_shape {
        unimaged(kind)
    } else {
        broken(kind)
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
