# Syntax

This part defines the lexical structure of adtb source text and the
grammar of every form a program writes: schema files, migration files,
bundle files, declarations, expressions and queries, writes and
transact blocks, schema operators and grants. It fixes what parses and
how; what a form means is stated by the part that owns its topic, and
each section links there.

## 1. Notation

The grammar is written in EBNF:

| Form | Meaning |
| --- | --- |
| `name = ...` | defines the nonterminal `name` |
| `"x"` | the terminal `x`, written exactly |
| `a b` | `a` followed by `b` |
| `a \| b` | `a` or `b` |
| `[ a ]` | `a`, optional |
| `{ a }` | zero or more repetitions of `a` |
| `( a )` | grouping |
| `(* text *)` | a side condition stated in prose |

Terminals for the characters `|`, `{`, `}`, `(`, `)`, `[` and `]` are
always quoted. Nonterminals written in capitals (`LOWER`, `UPPER`,
`INT`, `TEXT`, ...) are tokens defined in §2.

Examples extend the running schema of the [index](README.md#conventions),
the schema at version 12, after migration 0011 of
[evolution.md](evolution.md#42-add-a-constructor) adds `Apple { sub:
Text }` joining `OAuth` and migration 0012 enables it.
Migration examples state the version they start from.

Rulings: none

## 2. Lexical structure

### 2.1 Source text

Source text is UTF-8. An implementation must refuse a file that is not
valid UTF-8. Text literals are normalised to Unicode NFC when the
value is constructed ([model.md](model.md#35-text)); the rest of the
source is compared as written.

### 2.2 Whitespace and comments

Space, tab, carriage return and line feed separate tokens and are
otherwise insignificant. The grammar is free-form: line breaks and
indentation carry no meaning, and the layout used in examples is
convention.

`--` starts a comment that runs to the end of the line. A comment is
whitespace.

### 2.3 Identifiers

```ebnf
LOWER = lower { ident_char }
UPPER = upper { ident_char }
ident_char = lower | upper | digit | "_"
lower = "a" ... "z"
upper = "A" ... "Z"
digit = "0" ... "9"
```

`LOWER` names values, binders, fields, parameters, relation variables,
functions, queries, transactions, constraints, checks, modules, roles
and type variables. `UPPER` names types, constructors and named
constructor sets. The case of the first character decides the class;
an implementation must refuse a declaration whose name has the wrong
class.

A lone `_` is the wildcard token, not an identifier. Identifiers are
ASCII.

A name from another module is qualified by its module:

```ebnf
QLOWER = [ LOWER "::" ] LOWER
QUPPER = [ LOWER "::" ] UPPER
```

`auth::identities` is the relation variable `identities` of module
`auth`. Module resolution is stated in §3.4.

### 2.4 Keywords

Reserved words cannot be used as identifiers except in field position
(below):

```text
and bundle by check constraint delete else exists fail false fix fn
for from group if import in insert into key let match module not or
order ordered query relation return schema select set skip temp then
top transact true try txn type union update where with
```

The prelude type names `Set`, `Key`, `Group` and the bound names `Eq`,
`Ord`, `Hash` and `Num` are reserved type names: no type or type
parameter may be declared with one of them. The reservation covers type
and bound names only; a constructor or field may have one of these
names.

Contextual words are keywords only where the grammar names them and
are identifiers elsewhere:

```text
add adhoc adopt after aggregate alter analyze apply archive arm asc
assert at audit backup before budget call cancel carry cascade catalog
clear commit compact computed configure conflict constructor
constructors create default deferrable define dek deprecate desc
destroy deterministic disable drop dump durability edition enable encrypt explain
export feed field fill finalize finish generate grant hide host
incremental index init install instant interval joins kek lease limit
local maintain map merge migrations monitor move nest new of offline on
online option password peer pin pins plan position print priority quorum
read read_files read_only redefine register reindex reject rejects
rekey remove rename restore restrict retag retire retry revert revoke
role rotate script session setting split state step store sums
supersede take terminate text through to tzdb uninstall unique unless
unmap unnest unpin unstore upgrade vacuum verify version volatile wasm
write_files
```

In field position any word, reserved or not, is a field name. Field
position is the word after `.`, the word before `:` inside a record
type, record expression or record pattern, the word before `:` in a
named argument (`memberships.key(user: u, team: t)`), a field name
standing alone in a record pattern (`{ key }`), and a name standing
alone in the field list of a `key`, `unique` or `index` clause. `g.key` is the field
`key` of `g`; `{ key: k }` is a record with a field `key`.

### 2.5 Literals

```ebnf
INT      = digit { digit }
DECIMAL  = digit { digit } "." digit { digit }
FLOAT    = digit { digit } [ "." digit { digit } ] ( "e" | "E" ) [ "+" | "-" ] digit { digit }
TEXT     = '"' { text_char | escape } '"'
escape   = "\\" ( '"' | "\\" | "n" | "r" | "t" | "u{" hex { hex } "}" )
DURATION = digit { digit } unit
unit     = "us" | "ms" | "s" | "min" | "h" | "d"
hex      = digit | "a" ... "f" | "A" ... "F"
```

- A `text_char` is any character except `"`, `\` and a line break. A
  `\u{...}` escape names one Unicode scalar value; a surrogate or a
  value above `10FFFF` is refused.
- An integer literal has no sign; `-3` is negation applied to `3`.
  Leading zeros are allowed and do not change the value.
- The type of a numeric literal is the numeric type its context
  expects. With nothing expected, an `INT` is `I64`, a `DECIMAL` is
  `Decimal k` with `k` its number of fractional digits, and a `FLOAT`
  is `F64`. A literal out of range of its type is a
  compile-time error ([typing.md](typing.md#8-literals-and-numeric-types)).
- `true` and `false` are the constructors `True` and `False` of the
  prelude sum `type Bool = False | True`.
- `[e1, e2, ...]` is a set value, allowed where a `Set T` is
  expected; `[]` is the empty set. Equal elements are one element, and
  the order written carries no meaning. Its grammar is `set_literal`
  (§6.1); its printed form is in
  [interop.md](interop.md#11-one-syntax).
- A `DURATION` is a value of type `Duration`; `d` is 86400 seconds.
  Calendar periods have no literal form.
- A `Decimal s` value is written as a numeric literal, which takes its
  scale from its context
  ([typing.md §8](typing.md#8-literals-and-numeric-types)).
- A value with no literal (NaN and the infinities, time values,
  `Period`, `Bytes`) is written as a `T.parse` call on its textual
  form ([interop.md §1.3](interop.md#13-values-with-no-literal)),
  which returns `Option T`: `F64.parse("inf")`,
  `Date.parse("2026-10-25")`, `Bytes.parse("0x00ff")`.

Single quotes are not a token.

### 2.6 Bundle names, versions and hashes

```ebnf
BUNDLE_NAME = FAMILY "-" digit { digit | "." }
FAMILY      = lower { lower | digit | "_" | "-" }
version     = INT "(" HASH ")"
HASH        = hex { hex }        (* exactly 64 hex digits *)
MIGRATION   = digit digit digit digit { digit }
```

A `BUNDLE_NAME` is a token only after `bundle`, `retire bundle`,
`supersede bundle`, the `by` of a `supersede bundle` command,
`grant call` and `revoke call`, and as the first part of a qualified
query name `web-2.4::login_key` (after `grant call`, `revoke call` and
`explain`); `web-2.4` and `ios-4.2` are bundle names there. A bundle
name is its family, then a hyphen, then its version: the digits and
dots after the last hyphen. `web` is the family of `web-2.4`. After
`grant call` and `revoke call`, `FAMILY::q` names query `q` in every
bundle of the family. A
`HASH` is a token only inside the parentheses of a `version`, and is
written in full, 64 hex digits; an implementation must refuse a
shorter one. Examples elide it as `c41e...`. A
version is a position in the operator log shown with the hash of the
log prefix up to that position ([evolution.md](evolution.md#2-versions)).
A `MIGRATION` number names a migration file and the schema version it
produces.

### 2.7 Punctuation and operators

```text
==  !=  <  <=  >  >=  +  -  *  /  %  ++  =  :  ::  ,  .  .<  ->  |
(  )  {  }  [  ]  _
```

`++` concatenates text and bytes. `==` is equality and `!=` inequality. `=` never compares: it binds a
name in `let`, `fix` and declarations. `->` separates a pattern from
its arm, a signature from its result type, a lambda's parameters from
its body, and the source from the target of a retag grant.

### 2.8 Calls

A `(` that follows an identifier with no whitespace between them
starts the argument list of a call: `active()`, `scalar_len(t)`,
`Email("a@b")`, `users.key(3)`. A `(` after whitespace never does:
`fn display_name (i: Identity)` declares, and `top 3 by n (from ...)`
gives `top` its operand. This is the only place whitespace is
significant.

Rulings: syntax/strings/double, syntax/equality/double, syntax/params/names, spec/textual-forms/interop

### 2.9 Canonical token form

The canonical token form of a term is a `Text`: the tokens of the term
after name resolution, each in its canonical spelling, joined by one
space (U+0020). Comments and all other whitespace are dropped.

| Token | Canonical spelling |
| --- | --- |
| Name of a type, constructor, named set, relation variable, function, query, transaction, constraint or check | qualified by its module: `auth::identities`, `auth::Identity`; a prelude name as written |
| Any other identifier, keyword or punctuation | as written |
| `INT` | its digits without leading zeros; `0` for zero |
| `DECIMAL` | the integer part without leading zeros (`0` when it is zero), `.`, then the fractional digits as written |
| `FLOAT` | the integer part without leading zeros, the fractional part as written, `e`, then the exponent without `+` and without leading zeros |
| `DURATION` | its digits without leading zeros, then the unit |
| `TEXT` | `"`, the literal's value in NFC with `"` written `\"`, `\` written `\\`, line feed `\n`, carriage return `\r` and tab `\t`, every other scalar value as itself, then `"` |

```adt
-- given u: Key users
count(i for i in identities where i.user == u)   -- a term in module auth
```

```text
count ( i for i in auth::identities where i . user == u )
```

The canonical token form of `0012` is `12`, of `007.50` is `7.50`,
and of `"a\u{9}b"` is `"a\tb"`.

Rulings: spec/evolution/term-encoding/canonical-tokens

## 3. Files

### 3.1 Kinds of file

```ebnf
schema_file    = module_header { declaration }
migration_file = module_header { operator }
bundle_file    = bundle_header { import } { bundle_item }
module_header  = "module" LOWER
bundle_item    = fn_decl | query_decl | txn_decl | feed_decl | aggregate_decl
adhoc_text     = adhoc_item { adhoc_item }
adhoc_item     = expr | write_expr | transact_block | command
               | grant | revoke | role_stmt | peer_stmt | temp_decl
temp_decl      = "temp" "relation" LOWER ":" app_type [ "key" key_fields [ generated ] ]
```

- A schema file holds declarations. It is the form in which the engine
  prints the schema it folds from the operator log, and the form
  examples use to show a schema. A database is changed only by
  migration files.
- A migration file holds schema operators (§8). Its name is its
  `MIGRATION` number. It applies as one schema change, and its number
  is the schema version it produces
  ([evolution.md](evolution.md#11-migrations)).
- A bundle file holds the registered queries, transactions, functions
  and feeds of one client release (§9).
- Ad hoc text is what a principal holding the `adhoc` privilege sends
  outside any bundle: queries, writes, `transact` blocks, commands
  (§12), temp relation declarations, and, for holders of `schema`,
  role statements, peer mapping statements and capability grants
  (§10). A write needs the write
  privileges it uses ([queries.md §16](queries.md#16-ad-hoc-statements)).
- In ad hoc text a free lowercase name is a parameter, typed by the
  descriptor the caller supplies for it; a free name the caller supplies
  no parameter for is an error. Several items in one ad hoc text form
  one transaction: each item is a statement, the items run in order,
  and their results are returned in order. The first item that
  faults, fails, violates a constraint or is rejected ends the text:
  nothing commits, later items do not run, and the error names that
  item's ordinal, counted from 1
  ([transactions.md](transactions.md)).
- `temp relation NAME : T` declares a relation outside the operator
  log. Declared inside an interactive transaction, it lives until that
  transaction ends; declared in ad hoc text sent alone, it lives until
  that text ends. A session holds none. `T` is a named type or an
  anonymous record type; the relation has no constraint but its key.
  Without a `key` clause its whole row is its key. Its key clause has
  no `generate`, since no type names a temp relation, and its name must
  not be a relation variable of a module the text reads. A second
  declaration of one temp relation name in one transaction is refused. Loads append to it and ad hoc inserts route its rows
  ([interop.md](interop.md#4-loading)).

In examples, `0011 add ...` is shorthand for a migration file `0011`
holding the one operator, and `-- 0011` before a block of operators
marks the file they belong to. Examples leave out the file's `module`
header; the running schema's migrations are in module `auth`.

### 3.2 Item boundaries

Every declaration, operator and bundle item starts with a keyword. A
block item or ad hoc item starts with a keyword or with `(`; an ad hoc
query that starts with neither is the first item of its text or is
parenthesised. An item ends where the next item's starting keyword appears
at a point where the current item cannot continue. No separator is
written between items.

### 3.3 Open forms

The forms `from`, `match`, `fix`, `fn` (lambda), `exists`,
`if` and `reject` are open: their last part is an expression, and it extends as far to
the right as the text allows. `top` and `at` are not open: `top` ends
at its parenthesised operand and `at` at its closing `}`. Text that should end an open form
earlier is parenthesised:

```adt fragment
-- the inner match takes both arms
match a | X -> match b | Y -> 1 | Z -> 2
-- parentheses close it
match a | X -> (match b | Y -> 1 | Z -> 2) | W -> 3
```

A comprehension used as an operand of `union`, or followed by anything
that would otherwise extend its `select` expression, is parenthesised.

### 3.4 Modules

A module comes to exist by the operator `add module NAME` in a
migration; `drop module NAME` is its inverse, refused while the module
holds any object (§8.1).

One module per file: `module NAME` heads every migration file and
every file of a schema source tree; the header is required. Unqualified
names in that file resolve in that module first and then
in the prelude; every other module's names are qualified. A bundle
file names the modules it reads with `import`; an imported module's
names may then be used unqualified when no two imported modules
declare the same name, and an implementation must refuse an ambiguous
unqualified name.

Ad hoc text belongs to no module. An unqualified name in it denotes
the declaration of that name in a user module, or in the prelude when
no user module declares it; a name declared in several user modules is
ambiguous, refused unqualified, and written qualified
([queries.md §16](queries.md#16-ad-hoc-statements)).

The schema's own `Principal`, when it declares one, belongs to the
root namespace, not to the module whose migration declares it: it
resolves as `Principal` from every module and every bundle
([security.md](security.md#11-the-principal-sum)).

Rulings: syntax/module/header, surface/namespace/modules, surface/source/op-log, surface/migration/handwritten, surface/version/counter, spec/adhoc/names/every-module, spec/temp-relations/names/distinct, spec/temp-relations/generated-key/none, spec/temp-relations/redeclare/refused

### 3.5 Nesting

An implementation must accept text that nests 64 levels deep and must
refuse text that nests deeper, as a syntax error. The depth of a point
in the text is the number of enclosing parentheses, brackets, braces,
operands, patterns, types, writes, generators and `where` clauses
around it.

- A chain of one of `and`, `or` and `union` counts as a balanced
  binary tree of its operands, split in half at each level with the
  first half taking the odd operand, and each operand sits as many
  levels inside the chain as its leaf is deep in that tree. A chain of
  `n` operands is at most `⌈log₂ n⌉` levels: `a or b or c or d` is two
  levels over its operands, and a filter of 1000 alternatives is ten.
- Every other operator chain counts one level per operator:
  `a + b + c + d` is three levels over its operands, as is `x.f.g.h`.
- The number of items in a list does not count: the elements of a set
  literal, the fields of a record, the items of a `transact` block and
  the arms of a `match` each sit one level inside their form however
  many there are.

Rulings: spec/nesting-limit/balanced

## 4. Types

```ebnf
type        = sum_type [ "->" type ]
sum_type    = app_type | anon_ctor { "|" anon_ctor }   (* a lone UPPER is an app_type *)
anon_ctor   = UPPER [ record_type ]                    (* never a prelude type name *)
app_type    = type_atom [ record_type ] { type_arg }   (* record_type only after a prelude type name *)
type_arg    = type_atom | INT
type_atom   = QUPPER [ "." ctor_sel ]
            | LOWER
            | "Key" relation_ref
            | "(" type ")"
            | "(" ")"
            | record_type        (* not as a type_arg: parenthesise it *)
record_type = "{" [ field_type { "," field_type } ] "}"
field_type  = LOWER ":" type [ ref_action ]
ref_action  = "on" "delete" ( "restrict" | "cascade" | "clear" | "retag" UPPER [ record_expr ] )
ctor_sel    = UPPER | "(" UPPER { "|" UPPER } ")"
```

- `Set T`, `Option T`, `Result T E` and `Decimal 2` are applications
  of named types to arguments. An `INT` argument is allowed only where
  the type's parameter is a number, as the scale of `Decimal`.
- A type argument list stops before a contextual clause word of the
  enclosing form (`on`, `volatile`, `unique`, `interval`, `index`,
  `default`, `computed`, `fill`, `generate`) and before a name
  immediately followed by `(` (§2.8). In `relation boxes : Box Text
  unique (label)` the type is `Box Text`; a type argument with one of
  these spellings is parenthesised.
- In a type declaration, the type under `new` is a base type
  ([model.md](model.md#4-newtypes)).
- `Identity.Google` is the row type of one constructor and
  `Identity.(Google | Apple)` the row type of a constructor set.
  `Tag Identity` is the tag type of the sum ([queries.md
  §9.2](queries.md#92-the-tag-of-a-sum)); `Tag` is a prelude type
  constructor, so no constructor name is reserved.
- `Key users`, `Key identities.Google`, `Key identities.(Google |
  Apple)` and `Key identities.OAuth` are reference types to a
  constructor set of a relation variable ([model.md](model.md#8-references)).
- A `ref_action` is allowed only on a field whose type is a `Key` type
  or an `Option` of one. `on delete clear` is allowed only on an
  `Option (Key ...)` field and sets it to `None`. The payload of `on
  delete retag C { ... }` may omit key and header fields, which carry
  over from the row. Meaning in
  [writes.md](writes.md#7-delete-actions).
- A record type used as a type argument is parenthesised: `Set ({
  email: Text })`, `Option ({ at: Instant })`, except directly after
  a prelude type name (§11): a prelude type name followed by `{`
  begins a type application whose first argument is that record type,
  so `Set { email: Text }` is `Set ({ email: Text })`. In a type
  position, `C { ... }` for any other `C` is an anonymous constructor,
  and alone it is a one-constructor anonymous sum: `Set (Found { user:
  Key users })`.
- An anonymous constructor may not be named after a prelude type: an
  implementation must refuse `Set { a: I64 } | Other` or `Found { a:
  I64 } | Option`, naming the prelude type.
- A field type carries no default. Defaults and generated values are
  clauses of a relation variable (§5.2).
- `A { ... } | B` in a type position is an anonymous sum and `{ f: T
  }` an anonymous record type. They are written in the signatures of
  functions, queries and transactions, for a result shape no
  declaration names: `Set ({ email: Text })`, `Set (ByEmail { email:
  Text } | ByGoogle { gid: Text })`. In an argument position an
  anonymous sum is parenthesised.
- In a type declaration (§5.1) every field's type is a named type, a
  type variable, a `Key` type, or an application of a named type: an
  implementation must refuse an anonymous record or anonymous sum as
  the type of a declared field.
- `T -> U` is a function type. It is right associative. A function
  type appears only in the signature of a `fn` and as the type of a
  lambda: never as the type of a relation, a field or a set element,
  as a type argument, in a query's result, or as a parameter of a
  registered query ([typing.md](typing.md)).
- `()` is the unit type, the result of a transaction that returns
  nothing.

Rulings: syntax/reference/key, syntax/record/colon, surface/ref/ref-type, surface/query-scoped, spec/type-args/stop/clause-words, spec/anon-sum/one-ctor/one-ctor, spec/anon-sum/prelude-head/type-app

## 5. Declarations

```ebnf
declaration = type_decl | relation_decl | set_decl | fn_decl
            | constraint_decl | check_decl | store_decl | aggregate_decl
```

### 5.1 Types

```ebnf
type_decl = "type" UPPER { LOWER } [ "ordered" ] [ record_type ] "=" type_body
type_body = "new" app_type [ "where" expr ]
          | record_type
          | ctor_def { "|" ctor_def }
ctor_def  = UPPER [ record_type ]
```

```adt
type User = { id: Key users, name: Text }
type Email = new Text where matches(self, "^[^@]+@[^@]+$")
type Status ordered = Draft | Sent { at: Instant } | Paid { at: Instant }
type Identity { id: Key identities, user: Key users }
  = EmailPass { email: Text, hash: Bytes }
  | Google    { gid: Text }
  | Apple     { sub: Text }
type These a b = This { value: a } | That { value: b } | Both { this: a, that: b }
```

- The lowercase names after the type name are its type parameters.
- `ordered` declares that the type is ordered: by constructor
  declaration order, then header fields, then the constructor's
  fields, each left to right ([model.md](model.md#10-order)). `ordered` is
  reserved, so it is never read as a type parameter.
- The record before `=` is the header: fields every constructor has.
  A header is allowed only when the body lists constructors.
- A record body declares a one-constructor type whose constructor has
  the type's name: `type User = { ... }` has constructor `User`.
- `new T` declares a newtype over `T`, a distinct nominal type. A
  value is built by calling the type, `Email("a@b")`, and `unwrap(x)`
  gives the underlying value back. `where e` makes the newtype a
  domain: `e`, with `self` the underlying value, is checked wherever a
  value of the newtype is stored, as a constraint
  ([constraints.md](constraints.md#23-domains)). Building a value never
  faults on its domain.
- Constructors are listed as alternatives in the one declaration; no
  other declaration adds a constructor. A nullary constructor has no
  record.

`unique` and `index` are never written inside a type.

### 5.2 Relation variables

```ebnf
relation_decl   = "relation" QLOWER ":" app_type { relation_clause }
relation_clause = "key" key_fields [ generated ]
                | "volatile"
                | "unique" index_fields [ "in" ctor_scope ]
                | [ "interval" ] "index" index_fields [ "in" ctor_scope ]
                | "default" default_target "=" ( generated | expr )
                | "computed" LOWER ":" type "=" expr
key_fields      = LOWER | "(" LOWER { "," LOWER } ")"
generated       = "generate" LOWER
default_target  = [ UPPER "." ] LOWER
index_fields    = "(" expr { "," expr } ")"
ctor_scope      = UPPER | "(" UPPER { "|" UPPER } ")"
```

```adt
relation users : User key id generate sequence

type Channel { id: Key channels, user: Key users }
  = Mail  { address: Text }
  | Phone { number: Text }
  | Pager { number: Text }
relation channels : Channel
  key id generate uuidv7
  unique (address) in Mail
  unique (number) in Phone
  index (user) in (Phone | Pager)

type Login { id: Key logins, user: Key users on delete cascade }
  = Active  { identity: Key identities on delete retag Revoked {} }
  | Revoked { at: Instant }
relation logins : Login
  key id generate sequence
  default Revoked.at = generate clock
```

- `key` names key fields the type declares; it never adds a field. A
  key field may have any type; a key field of type `Key r`, for its own
  relation variable `r`, is self-identifying
  ([model.md](model.md#7-keys)). A composite key is `key (doc, tag)`.
  `Key r` is a distinct type for each relation variable whatever its
  key fields' types; `r.key(v)` builds one and `value(k)` reads the key
  value back.
- `generate NAME` after the key makes an insert that omits the key
  fill it from the named generator: `sequence`, `uuidv7`
  ([model.md](model.md#73-generated-keys)).
- `in` after `unique` or `index` scopes the clause to one
  constructor, a parenthesised constructor set, or a named set (`in
  OAuth`, without parentheses). Without `in` the clause ranges over
  every constructor.
- An index element is a field, a path into a value-level sum field
  (`status.Paid.ref`), `tag(status)`, or an expression over the row's
  fields ([derived.md](derived.md#52-expression-indexes)).
- `interval index` declares a range index
  ([constraints.md](constraints.md#25-exclusion)).
- `default f = e` gives the value an insert into this relation
  variable takes for field `f` when it omits it; `default C.f = e`
  does so for inserts of constructor `C` only. `e` is an expression
  without free binders, or `generate NAME`; `generate clock` is the
  wall-clock time at which the statement applies
  ([model.md](model.md#73-generated-keys)). A default is
  applied by an insert into, or a retag within, the relation variable
  that declares it, when the new value omits the field.
- `computed f: U = e` declares a computed field of the relation
  variable; inside `e`, `self` is the row
  ([derived.md](derived.md#5-computed-fields)).
- A qualified relation name and the `volatile` clause appear only in
  the engine's declarations of its `catalog` and `sys` relations
  ([catalog.md](catalog.md#13-volatile-relations)); an implementation
  must refuse either in a user declaration.

The clauses may be written on one line: `relation users : User key id
generate sequence`.

### 5.3 Named constructor sets

```ebnf
set_decl = "set" UPPER "of" LOWER "=" UPPER { "|" UPPER }
```

```adt
set OAuth of identities = Google | Apple
```

A named set belongs to one relation variable and is used as
`identities.OAuth`, as `in OAuth` in that relation's clauses, and as a
pattern ([queries.md §6.4](queries.md#64-named-set-patterns)). Its
members are listed; there is no complement form. Two relation
variables of one type may not declare named sets of the same name with
different members; an implementation must refuse the second.

### 5.4 Functions

```ebnf
fn_decl     = "fn" LOWER [ type_params ] params "->" type "=" fn_body
type_params = "[" tparam { "," tparam } "]"
tparam      = LOWER [ ":" UPPER { "+" UPPER } ]
params      = "(" [ param { "," param } ] ")"
param       = LOWER ":" type
fn_body     = expr
            | "wasm" TEXT "export" TEXT
            | "host" TEXT "version" TEXT [ "deterministic" ]
```

```adt
fn display_name (i: Identity) -> Text = match i
  | EmailPass e -> e.email
  | Google g    -> g.gid
  | Apple a     -> a.sub
fn active () -> Set User = from u in users where u.name != "" select u
fn latest [a: Ord] (xs: Set a) -> Option a = max(x for x in xs)
```

- A space separates the name from the parameter list (§2.8). The
  parameter list is required; a view is a function with `()`.
- Parameter types and the result type are required.
- A type parameter's bounds are among `Eq`, `Ord`, `Hash` and `Num`
  ([typing.md](typing.md#7-generic-functions)).
- `wasm` and `host` bodies declare user-defined functions implemented
  outside the language ([interop.md](interop.md#9-user-defined-functions)).

### 5.5 Registered queries

```ebnf
query_decl = "query" LOWER params [ "limit" DURATION ] "->" type "=" expr
```

```adt
query emails (u: Key users) -> Set ({ email: Text }) =
  from i in identities.EmailPass where i.user == u select { email: i.email }
query login_key (u: Key users) -> Set (ByEmail { email: Text } | ByGoogle { gid: Text }) =
  from i in identities.(EmailPass | Google) where i.user == u
  select match i
    | EmailPass e -> ByEmail { email: e.email }
    | Google g    -> ByGoogle { gid: g.gid }
```

A `query` appears only in a bundle file. Its parameters and its result
have no function types (§4); its result may be any other type: `query n_users () -> I64 =
count(u for u in users)`. `limit 30s` after the parameters declares the
query's own statement limit
([catalog.md](catalog.md#83-limits-and-deadlines)); a `txn` takes the
same clause.

### 5.6 Registered transactions

```ebnf
txn_decl = "txn" LOWER params [ "limit" DURATION ] "->" type "=" transact_block
```

```adt
txn sign_up (n: Text, g: Text) -> Key identities =
  transact
    let u = insert users { name: n }
    let i = insert identities.Google { user: key u, gid: g }
    return key i
```

A `txn` appears only in a bundle file. A transaction whose block has no
`return` has result type `()`.

### 5.7 Constraints and checks

```ebnf
constraint_decl = "constraint" LOWER "=" expr
check_decl      = "check" LOWER "on" relation_ref "(" expr ")"
```

```adt
constraint one_google =
  from u in users
  where count(g for g in identities.Google where g.user == key u) > 1
  select u
check email_length on identities.EmailPass (scalar_len(email) <= 254)
```

Every constraint and check is named. Inside a check's parenthesised
expression the fields of the checked constructor set are in scope by
name and `self` is the row. Meaning, admission and timing are in
[constraints.md](constraints.md).

### 5.8 Stored views

```ebnf
store_decl = "store" QLOWER { store_clause }
store_clause = "key" key_fields
             | "unique" index_fields [ "in" ctor_scope ]
             | [ "interval" ] "index" index_fields [ "in" ctor_scope ]
```

```adt
module billing
store open_by_user
  key user
  index (open)
```

`store f` declares that the parameterless function `f` is held and
maintained, with the keys, uniques and indexes its clauses give
([derived.md](derived.md#2-stored-views)). `open_by_user` is the view
derived.md defines.

### 5.9 User-defined aggregates

```ebnf
aggregate_decl = "aggregate" LOWER params "->" type "="
                 "state" record_type "init" expr
                 "step" "(" LOWER "," LOWER ")" "->" expr
                 "merge" "(" LOWER "," LOWER ")" "->" expr
                 "finish" LOWER "->" expr
```

In a migration the declaration is the operator `define aggregate`
(§8.5). Meaning in [interop.md](interop.md#10-user-defined-aggregates).

Rulings: syntax/decl/type-relation, syntax/keys/on-relation, syntax/query-decl/arrow, syntax/params/names, syntax/constraint/named, syntax/reference/key, syntax/record/colon, surface/sum-decl/alternatives, surface/shared-fields/header, surface/extent/separate, surface/ctor-set/named, surface/ctor-set/list, surface/constraint/check, surface/constraint/denial, lang/nullary-fn, lang/params/declared, lang/inference/signatures, lang/sum-order/declared, write/transact/registered-txn, derived/defaults/on-relation, types/keygen/sequence, types/keygen/uuidv7, types/id-type/key-type, model/declared-key, constraint/overlap/range-index, derived/computed/virtual, derived/computed/expr-index, derived/materialize/storage-choice, derived/mv-integrity/keyed, interop/uda/fold-decl, interop/udf/wasm, interop/udf/host-fn, types/generics/bounded

## 6. Expressions

### 6.1 Precedence

From loosest to tightest:

| Level | Forms | Associativity |
| --- | --- | --- |
| 1 | open forms: `from`, `match`, `fix`, `fn`, `exists`, `if`, `reject` | extend right |
| 2 | `union` | left |
| 3 | `or` | left |
| 4 | `and` | left |
| 5 | `not` | prefix |
| 6 | `==` `!=` `<` `<=` `>` `>=` | none |
| 7 | `+` `-` `++` | left |
| 8 | `*` `/` `%` | left |
| 9 | unary `-`, `key`, `try` | prefix |
| 10 | `.f`, `.C`, `.(C \| D)`, `.<r.f`, call | left |

Comparisons do not chain: `a < b < c` is refused.

```ebnf
expr      = union_e
union_e   = or_e { "union" or_e }
or_e      = and_e { "or" and_e }
and_e     = not_e { "and" not_e }
not_e     = "not" not_e | cmp_e
cmp_e     = add_e [ cmp_op add_e ]
cmp_op    = "==" | "!=" | "<" | "<=" | ">" | ">="
add_e     = mul_e { ( "+" | "-" | "++" ) mul_e }
mul_e     = prefix_e { ( "*" | "/" | "%" ) prefix_e }
prefix_e  = ( "-" | "key" | "try" ) prefix_e | postfix_e
postfix_e = primary { "." LOWER | "." ctor_sel | ".<" relation_ref "." LOWER | key_call }
primary   = literal
          | LOWER | QLOWER
          | call [ "pins" version ]
          | type_call
          | ctor_expr
          | record_expr
          | set_literal
          | "(" expr ")"
          | "(" ")"
          | open_form
          | write_seq             (* as an operand inside a transact block, §7 *)
literal   = INT | DECIMAL | FLOAT | TEXT | DURATION | "true" | "false"
set_literal = "[" [ expr { "," expr } ] "]"
open_form = comprehension | match_expr | fix_expr | lambda | exists_expr
          | top_expr | if_expr | at_expr | reject_expr
```

An open form in a position other than last extends over everything
after it; parenthesise it there (§3.3).

### 6.2 Names, calls and relation references

```ebnf
call         = ( QLOWER | QUPPER ) "(" [ call_args ] ")"
call_args    = call_arg { "," call_arg } | expr "for" generators [ "where" expr ]
call_arg     = expr | type          (* a type only where the parameter is a type *)
type_call    = ( QUPPER | "(" type ")" ) "." LOWER "(" [ call_args ] ")"
key_call     = "." "key" "(" ( expr | named_arg { "," named_arg } ) ")"
named_arg    = LOWER ":" expr
relation_ref = QLOWER [ "." ctor_sel ]
```

- A lowercase name is a binder, a parameter, a relation variable, a
  function or `self`.
- A call's parenthesis follows a name directly (§2.8). `active()`
  calls a parameterless function. `x.f(y)` is a call only in the two
  forms below; otherwise it is refused.
- A call of a newtype's name builds a value of the newtype:
  `Email("a@b")` ([model.md](model.md#4-newtypes)).
- `T.f(args)` calls a function the prelude defines for every type `T`:
  `Instant.parse("2026-10-25T00:00:00Z")`, `(Tag Identity).parse(r.kind)`.
  A type that is not a single name is parenthesised.
- `r.key(v)` builds the key of relation variable `r` with key value
  `v`; for a composite key the fields are named: `memberships.key(user:
  u, team: t)`. It is the only way to write a key value; no literal
  checks against a `Key` type ([model.md](model.md#7-keys)).
- A file-scan function takes a row type as an argument and may be
  followed by `pins` and a version: `read_csv("contacts-old.csv",
  Contact) pins 41 (b3e0...)` ([interop.md](interop.md#3-file-scans)).
- A call whose argument is `e for x in q ...` is an aggregate:
  `count(i for i in identities where i.user == key u)`,
  `sum(o.qty for o in orders where o.user == key u)`. `tag(e)` is a
  call of the prelude function `tag`. The prelude's functions are
  listed in [prelude.md](prelude.md).
- `.C`, `.(C | D)` and `.S` (a named set) applied to a set-valued
  expression select a constructor relation or constructor set:
  `identities.Google`, `identities.(Google | Apple)`,
  `identities.OAuth`, `sign_ins().Remote`.
- `.f` is field access, and on a reference it follows the reference:
  `i.user.name`.
- `.<r.f` is a backlink: `u.<identities.user`. The relation reference
  may select constructors: `u.<identities.Google.user`.
- `key e` is the key of row `e`, a row bound by a generator, a
  pattern, or a `let` of a write.

### 6.3 Constructors and records

```ebnf
ctor_expr   = QUPPER [ "." UPPER ] [ record_expr ]
record_expr = "{" [ field_init { "," field_init } ] "}"
            | "{" expr "with" field_init { "," field_init } "}"
field_init  = LOWER ":" expr
```

```adt fragment
Google { user: u, gid: g }
Identity.Google { user: u, gid: g }
None
Some { value: 3 }
{ email: e.email, n: 2 }
Google { g with gid: g2 }
```

- `T.C` qualifies a constructor by its type where the bare name is
  ambiguous ([typing.md](typing.md#2-signatures-and-inference)).
- `C { x with f: v }` is `x` with field `f` replaced, as constructor
  `C`.
- Every field of a record expression is named; there is no punning in
  expressions.

### 6.4 Match and patterns

```ebnf
match_expr = "match" or_e arms
arms       = arm { arm }
arm        = "|" pattern { "|" pattern } [ "if" expr ] "->" expr
pattern    = "_"
           | LOWER
           | QUPPER [ "." UPPER ] [ ctor_payload ]
           | literal_pat
ctor_payload = "_" | LOWER | record_pat
record_pat = "{" [ field_pat { "," field_pat } ] "}"
field_pat  = LOWER [ ":" pattern ]
literal_pat = [ "-" ] ( INT | DECIMAL | FLOAT ) | TEXT
```

```adt
-- given i: Identity
match i
  | EmailPass { email, hash: _ } -> email
  | Google g if g.gid == "root"  -> "root"
  | Google g | Apple g           -> display_name(g)
```

- `|` before `->` separates the alternatives of an or-pattern; `|`
  after an arm's expression starts the next arm. An arm's expression
  is an open position: a nested `match` takes every following arm
  unless parenthesised.
- `C x` binds the constructor's row to `x`; `C _` matches `C` and binds
  nothing; `C` alone is a nullary constructor; `C { ... }` is a record
  pattern. In a record pattern, a field name alone binds that field
  under its own name, `f: p` matches field `f` against `p`, and fields
  not listed are not constrained.
- A pattern naming a named constructor set (`OAuth o`) is written like
  a constructor pattern.
- In an arm of a split or merge operator (§8.4) the match has no
  scrutinee: `by match | ... -> ...`.

The rules of matching, coverage and binder types are in
[queries.md §5](queries.md#5-match) and [§6](queries.md#6-patterns).

### 6.5 Comprehensions

```ebnf
comprehension = from_clause { from_clause | where_clause }
                [ group_clause { from_clause | where_clause } ]
                "select" expr [ order_clause ]
from_clause   = "from" generators
generators    = generator { "," generator }
generator     = LOWER "in" or_e
where_clause  = "where" expr
group_clause  = "group" or_e "by" or_e "into" LOWER
order_clause  = "order" "by" order_keys [ "take" add_e [ "after" add_e ] ]
order_keys    = order_key | "(" order_key { "," order_key } ")"
order_key     = add_e [ "asc" | "desc" ]
```

```adt
-- given cursor: Option (Key users)
from u in users, i in identities
where i.user == key u
select { name: u.name, login: display_name(i) }

from i in identities group i by i.user into g
select { user: g.key, n: count(x for x in g) }

from u in users select u order by u.id take 50 after cursor

from p in posts select p order by (p.created_at desc, key p)
```

The examples of this section and §6.6 use `posts`, `follows` and
`phones` as [queries.md](queries.md) declares them.

An `order_clause` is allowed only on the outermost comprehension of a
registered query's body or of an ad hoc statement ([queries.md §14](queries.md#14-ordering-top-n-and-pagination)).
`take` is allowed only after `order by`. There is no offset form. A
compound ordering is a parenthesised list of keys, each with its own
direction; a `(` directly after `by` always opens that list, so a
single key that starts with a parenthesis is written inside one:
`order by ((a + b) * 2)`.

### 6.6 Other open forms

```ebnf
fix_expr    = "fix" fix_bind { "," fix_bind } "in" expr
fix_bind    = LOWER "=" expr
lambda      = "fn" ( LOWER | params ) "->" expr
exists_expr = "exists" generators [ "where" expr ]
top_expr    = "top" add_e "by" order_keys "(" comprehension ")"
if_expr     = "if" expr "then" expr "else" expr
at_expr     = "at" ( "commit" | "instant" ) add_e [ "pins" version ] "{" expr "}"
reject_expr = "reject" expr
```

```adt fragment
fix reach = (from f in follows where f.src == me select f.dst)
          union (from r in reach, f in follows where f.src == r select f.dst)
in from u in reach select u

fn i -> match i | Google g -> true | EmailPass _ -> false | Apple _ -> false

not exists d in phones where d.owner == key u

top 3 by (p.created_at desc, key p) (from p in posts where p.author == key u select p)

if scalar_len(u.name) > 0 then u.name else "anonymous"

at commit p { from i in identities.Google select i }
```

- In `fix`, a bound expression ends at a `,` that is not part of a
  generator list, or at the keyword `in` that is not part of a
  generator. `fix a = q1, b = q2 in e` binds `a` and `b` together.
- `not exists` is `not` applied to an `exists` form.
- The operand of `top` is a parenthesised comprehension, and its keys
  are written over the comprehension's `select` scope
  ([queries.md §14.3](queries.md#143-top-n)). It follows the keys after
  whitespace, so it is never read as call arguments (§2.8).
- `if c then a else b` means `match c | True -> a | False -> b`.
- `at commit p { q }`, with `p : Position`, and `at instant t { q }`
  read a past state
  ([derived.md](derived.md#6-reading-the-past)).
- `reject e`, with `e : Text`, is allowed only in the source query of
  an insert ([interop.md](interop.md#45-rejects)).

Rulings: syntax/aggregate/comprehension, syntax/ctor-set/paren, syntax/equality/double, syntax/record/colon, lang/family/comprehension, lang/nav/paths, lang/nav/backlinks, lang/patterns/guards, lang/patterns/or-patterns, lang/patterns/record, lang/patterns/literals, lang/patterns/nested, lang/recursion/fix, lang/nested-order/top-n, lang/order/result-only, lang/keyset, lang/group/into, lang/abstraction/higher-order, spec/top/operand/comprehension

## 7. Writes and transact blocks

```ebnf
transact_block  = "transact" { transact_option } { item } [ "return" expr ]
transact_option = "retry" INT
                | "read_only" [ "deferrable" ]
                | "durability" ( "local" | "quorum" )
item            = "let" LOWER "=" ( expr | write_expr )
                | write_expr
write_expr      = insert | update | delete | fail | "skip"
                | write_seq | write_if | write_match
write_seq       = "(" write_expr { "," write_expr } ")"
write_if        = "if" expr "then" write_expr "else" write_expr
insert          = "insert" relation_ref ( "from" or_e | or_e ) [ upsert ]
                  [ "rejects" "into" QLOWER ]
upsert          = "unless" "conflict" "on" ( "key" | index_fields [ "in" ctor_scope ] )
                  "else" write_expr
update          = "update" LOWER [ "in" or_e ] [ "where" expr ] "set" expr
delete          = "delete" LOWER [ "in" or_e ] [ "where" expr ]
fail            = "fail" or_e
write_match     = "match" or_e write_arm { write_arm }
write_arm       = "|" pattern { "|" pattern } [ "if" expr ] "->" write_expr
```

```adt
-- given n: Text, g: Text, u: Key users, k: Key identities, g2: Text
transact
  let u = insert users { name: n }
  let i = insert identities.Google { user: key u, gid: g }
  return key i

insert identities from (
  from x in imports
  select match x
    | Email e -> EmailPass { user: e.user, email: e.addr, hash: e.h }
    | Oauth o -> Google { user: o.user, gid: o.sub })

update g in identities.Google where g.user == u set Google { g with gid: g2 }

update i in identities where key i == k
set match i
  | EmailPass e -> Google { gid: lookup(e.email) }
  | Google g    -> g
  | Apple a     -> a

delete s in logins.Revoked where s.user == u

insert identities.Google { id: k, user: u, gid: g }
unless conflict on key
else match old
  | Google o    -> update o set Google { o with gid: g }
  | EmailPass e -> fail "identity is email"
  | Apple a     -> fail "identity is apple"

insert identities.Google from (from x in imports.Oauth select { user: x.user, gid: x.sub })
unless conflict on (gid) in Google
else update old set Google { old with user: new.user }

transact
  let u = insert users { name: n }
  (insert identities.Google { user: key u, gid: g },
   if scalar_len(n) > 0 then skip else fail "empty name")

transact retry 5 durability quorum
  let i = insert identities.Google { user: u, gid: g }
  return key i
```

The examples extend the running schema with `logins`, `imports` and
`lookup` as [writes.md](writes.md#1-change-sets) declares them:

```adt
type Login { id: Key logins, user: Key users on delete cascade }
  = Active  { identity: Key identities on delete retag Revoked {} }
  | Revoked { at: Instant }
relation logins : Login
  key id generate sequence
  default Revoked.at = generate clock

type Import { id: Key imports } = Email { user: Key users, addr: Text, h: Bytes }
                                | Oauth { user: Key users, sub: Text }
relation imports : Import key id generate sequence

fn lookup (email: Text) -> Text = lower(email)
```

- An `item` is one `let` or one write of a block. "Statement" means a
  whole statement: a block, or one write or query sent alone.
- A write is written inside a `transact` block, a `txn` body, a
  `script` operator, or as ad hoc text (§3.1). Inside a block a write
  may also be an operand, always parenthesised: `key (insert users {
  name: n })`, only where it is evaluated exactly once per statement:
  under no binder, and not in a branch of an `if`, the right operand of
  `and` or `or`, or an arm of a match
  ([writes.md §2.2](writes.md#22-what-an-insert-returns)).
- `skip` is the empty write. `(w1, w2, ...)` combines writes into one
  change set and is allowed wherever one write is, a match arm or an
  `if` branch included. `if c then w1 else w2` is a write.
- `update x set e` and `delete x` without `in` act on the row already
  bound to `x`, which must be a snapshot row, a row of a relation read
  from the statement's snapshot: bound by a query generator, as `old`,
  by a `let` of a row-valued expression over the snapshot, or by a
  match binder over such a row. A row a write produced is not one, so a
  name bound by `let` to a write is refused. With `where p` and no
  `in`, `update x where p set e` is `if p then update x set e else
  skip`, and `delete x where p` is `if p then delete x else skip`.
- `let x = w` takes an insert, an upsert, an update or a delete; over
  any other write it is a type error.
- `update ... set e` gives the new whole value of each row. There is no
  partial-path assignment and no `retag` clause.
- `fail e` takes `e : Text`. `fail` is reserved, so `transact fail
  "x"` is a block whose first item fails, never an option.
- The options are listed in
  [transactions.md](transactions.md#33-options). A block with both
  `read_only` and `durability` is a type error.
- `return` is the last thing in a block.
- In an upsert's `else`, `old` is the conflicting row of the snapshot
  and `new` the row the insert proposed. A `let` of a single-row upsert
  binds one row: the inserted row, or the row with `old`'s key that the
  branch's change set inserts, or else `old`.
  A conflicting row committed after the snapshot raises a
  serialization failure. A conflict only with a row another write of
  the same statement inserts binds no `old`: the statement fails with
  `KeyViolation` when the conflict is on the key, and with `Violated`
  naming the unique when it is on a unique such as `(gid) in Google`
  ([writes.md](writes.md#6-upsert)).
- `rejects into r` sends the records the insert's source rejects to the
  relation `r`; the source is a comprehension with one generator over a
  source relation or an external source
  ([writes.md §2.5](writes.md#25-rejected-rows),
  [interop.md](interop.md#45-rejects)).

The meaning of change sets, insert, update, retag, delete and upsert is
in [writes.md](writes.md); of the block in
[transactions.md](transactions.md).

Rulings: write/transact/block, write/surface/txdata, write/insert/ctor, write/insert/value, write/insert/returns/rows, write/update/ctor-binder, write/update/match-set, write/retag/assign, write/nested/replace, write/nested/rebuild, write/bulk/from-query, write/upsert/unless-conflict, write/upsert/cross-ctor/match-else, write/unit/multi, interop/load/errors/reject-input, spec/writes/bound-row/snapshot-row, spec/writes/let-write/four-forms, spec/writes/nested-insert/once, spec/writes/upsert-binding/change-set, spec/writes/rejects/per-row, spec/writes/rejects-source/one-source, spec/txn/durability/strongest, spec/writes/where-alone/conditional

## 8. Schema operators

Every operator is a verb, an object and clauses. The meaning of every
operator, its inverse and what it refuses is in
[evolution.md §4](evolution.md#4-operators).

The examples of each subsection are independent histories. Each starts
at version 12, the running schema (§1), unless it states another
version. Migration numbers in one subsection follow on from that
starting version.

```ebnf
operator = module_op | type_op | relation_op | ctor_op | field_op
         | key_op | default_op | definition_op | lifecycle_op
         | script_op | extension_op | grant | revoke
```

### 8.1 Modules, types, relations and sets

```ebnf
module_op   = "add" "module" LOWER | "drop" "module" LOWER
type_op     = "add" type_decl | "drop" "type" QUPPER
            | "rename" "type" QUPPER "to" UPPER
relation_op = "add" relation_decl | "drop" "relation" QLOWER
            | "rename" "relation" QLOWER "to" LOWER
            | "add" set_decl | "drop" "set" UPPER "of" QLOWER
            | "rename" "set" UPPER "of" QLOWER "to" UPPER
```

`add type`, `add relation` and `add set` are the declaration of §5
after the verb:

```adt fragment
-- 0013
add type Device = { id: Key devices, owner: Key users on delete restrict, label: Text }
add relation devices : Device key id generate sequence
add set Hosted of identities = EmailPass | Google
-- 0014
rename type Device to Gadget
rename relation devices to gadgets
rename set Hosted of identities to Managed
```

### 8.2 Constructors

```ebnf
ctor_op    = "add" "constructor" ctor_path [ record_type ] [ ( "before" | "after" ) UPPER ]
             { add_clause }
           | "remove" "constructor" ctor_path
           | "rename" "constructor" ctor_path "to" UPPER
           | "split" "constructor" ctor_path "into" ctor_def { "|" ctor_def } "by" "match" arms
             { split_clause }
           | "merge" "constructor" QUPPER "." ctor_sel "into" ctor_def [ "by" "match" arms ]
             { arm_clause }
           | "nest" "constructor" QUPPER "." ctor_sel "into" ctor_def { arm_clause }
           | "unnest" "constructor" ctor_path { arm_clause }
split_clause = "unique" index_fields "in" ctor_scope
             | arm_clause
             | [ "interval" ] "index" index_fields "in" ctor_scope
ctor_path  = QUPPER "." UPPER
add_clause = "joins" relation_ref { "," relation_ref }
           | arm_clause
arm_clause = "arm" QLOWER [ "." LOWER ] ":" pattern "->" expr
```

```adt fragment
-- from version 10
-- 0011
add constructor Identity.Apple { sub: Text }
  joins identities.OAuth
  arm display_name: Apple a -> a.sub
-- 0012
rename constructor Identity.EmailPass to Password

-- from version 12
-- 0013
add type Provider = Google | Apple
merge constructor Identity.(Google | Apple) into Social { provider: Provider, sub: Text }
  by match
    | Google g -> Social { provider: Google, sub: g.gid }
    | Apple a  -> Social { provider: Apple, sub: a.sub }
  arm display_name: Social s -> s.sub
-- 0014
split constructor Identity.Social into Google { gid: Text } | Apple { sub: Text }
  by match
    | Social { provider: Google, sub } -> Google { gid: sub }
    | Social { provider: Apple, sub }  -> Apple { sub: sub }
  unique (gid) in Google
  unique (sub) in Apple
  arm display_name: Google g -> g.gid
  arm display_name: Apple a -> a.sub

-- from version 12
-- 0013
add type SocialLogin = Google { gid: Text } | Apple { sub: Text }
nest constructor Identity.(Google | Apple) into Social { login: SocialLogin }
  arm display_name: Social s -> match s.login | Google g -> g.gid | Apple a -> a.sub
-- 0014
unnest constructor Identity.Social
  arm display_name: Google g -> g.gid
  arm display_name: Apple a -> a.sub
```

- Operators act on a type, named by `Type.Constructor`.
- `before C` or `after C` places the new constructor in the type's
  declaration order; without either it goes last
  ([evolution.md §4.2](evolution.md#42-add-a-constructor)).
- `joins` names a named set of a relation variable the new
  constructor joins. `arm name: P -> e` gives the new constructor's arm
  of the match in the schema function, match-scoped constraint or
  computed field (`r.f`) named. A `merge`, `nest`, `split` or `unnest`
  takes, for every such dependent whose match names the constructors it
  replaces, an `arm` clause for each constructor it produces, and is
  refused at the recheck without them
  ([evolution.md §4.7](evolution.md#47-merge-constructors)).
- The constructor of a `merge` has the field types of any declared
  constructor: its discriminator is the field whose named nullary sum
  has as constructors exactly the merged constructors' names
  (`provider: Provider`).
- In a `split` or `merge`, the `match` after `by` has no scrutinee: it
  matches a row of the constructor being split or merged. Its arms, and
  the arms of `nest` and `unnest`, may omit key and header fields,
  which carry over from the row.
- The `unique` and `index` clauses after a split's routing state the
  replacements for uniques and indexes that name a field no new
  constructor keeps. The routing's last arm ends at the first of them.

### 8.3 Fields

```ebnf
field_op     = "add" "field" field_path ":" type "fill" fill
             | "drop" "field" field_path
             | "rename" "field" field_path "to" LOWER
             | "move" "field" field_path "to" field_target [ "fill" fill ]
field_path   = QUPPER [ "." UPPER ] "." LOWER
field_target = QUPPER [ "." ctor_sel ]
fill         = "match" arms | expr
```

```adt
-- from version 12
-- 0013
add field Identity.EmailPass.verified: Bool fill false
-- 0014
move field Identity.EmailPass.verified to Identity fill match
  | Google g -> true
  | Apple a  -> true
-- 0015
move field Identity.verified to Identity.(EmailPass | Google)
-- 0016
rename field Identity.EmailPass.hash to pw_hash
-- 0017
drop field Identity.EmailPass.pw_hash
```

`Type.f` is a header field; `Type.C.f` a field of one constructor.
`fill` gives the field's value for existing rows; in a `move` to the
header it gives the value for the constructors that lack the field, as
a `match` with no scrutinee over their rows. A fill is not a default
for later inserts.

### 8.4 Keys, uniques, indexes and defaults

```ebnf
key_op     = ( "add" | "drop" ) ( "unique" | [ "interval" ] "index" ) index_fields "in" relation_ref
           | "rekey" "relation" QLOWER "key" key_fields [ "carry" field_path { "," field_path } ]
default_op = "add" "default" QLOWER [ "." UPPER ] "." LOWER "=" ( generated | expr )
           | "drop" "default" QLOWER [ "." UPPER ] "." LOWER
```

```adt
-- from version 12
add unique (user) in identities.Google
-- with:
--   type Price = { sku: Text, during: Span Instant, amount: I64 }
--   relation prices : Price key (sku)
--   type Order { id: Key orders } = Line { price: Key prices, qty: I64 } | Note { text: Text }
--   relation orders : Order key id generate sequence
rekey relation prices key (sku, during)
  carry Order.Line.price
add field Identity.EmailPass.verified: Bool fill false
add default identities.EmailPass.verified = false
drop default identities.EmailPass.verified
```

In a relation declaration the scope after `in` is a constructor set of
that relation; in an operator it names the relation as well. The
fields after `carry` are named by type, constructor and field.

### 8.5 Definitions, lifecycle, scripts and extensions

```ebnf
definition_op = "define" fn_decl | "redefine" fn_decl | "drop" "fn" QLOWER
              | "add" constraint_decl | "drop" "constraint" QLOWER
              | "add" check_decl | "drop" "check" QLOWER
              | store_decl | "unstore" QLOWER
              | "add" "computed" QLOWER "." LOWER ":" type "=" expr
              | "redefine" "computed" QLOWER "." LOWER ":" type "=" expr
              | "drop" "computed" QLOWER "." LOWER
              | "define" aggregate_decl | "drop" "aggregate" QLOWER
lifecycle_op  = "enable" ctor_target
              | "disable" ctor_target
              | "deprecate" ctor_target
              | "revert" MIGRATION { "fill" field_path "=" fill }
ctor_target   = relation_ref | ctor_path
script_op     = "script" write_expr
extension_op  = "install" LOWER TEXT "from" TEXT | "uninstall" LOWER
              | "adopt" "tzdb" TEXT "from" TEXT
```

```adt fragment
-- from version 10
-- 0011
add constructor Identity.Apple { sub: Text }
  joins identities.OAuth
  arm display_name: Apple a -> a.sub
-- 0012
enable identities.Apple
-- 0013
add constructor Identity.Corp { domain: Text }
  arm display_name: Corp c -> c.domain
enable identities.Corp
script
  update i in identities.Google where ends_with(i.gid, "@corp")
  set Corp { i with domain: i.gid }
-- 0014
deprecate identities.Apple
-- 0015
adopt tzdb "2026b" from "tzdata-2026b.tzif"
-- 0016
define fn google_users () -> Set User =
  from u in users where exists g in u.<identities.Google.user select u
store google_users
  key id
-- 0017
revert 0016
-- 0018
add constructor Identity.Saml { idp: Text }
  arm display_name: Saml s -> s.idp
enable identities.Saml
-- 0019
disable identities.Saml
```

- In a migration, `store` is written without `add`.
- `enable r.C` opens constructor `C` of relation `r` to writes;
  `enable T.C`, for a constructor of a value-level sum `T`, lets a
  value of `C` be stored in any field: `enable Provider.Okta`.
  `disable` returns an enabled constructor to declared, and is refused
  while any row of `r.C`, or any stored value of `T.C`, exists. The
  inverse of `enable` is `disable` when the constructor was declared
  and `deprecate` when it was deprecated. `deprecate` applies only to an
  enabled constructor
  ([evolution.md](evolution.md#4-operators)).
- `revert n` takes one `fill T.C.f = e` per inverse that adds a field
  the log holds no values for; `e` is read as an `add field` fill is
  ([evolution.md §5.4](evolution.md#54-revert)).
- A `script` holds one write ([writes.md](writes.md#1-change-sets)).
- `install geo "1.2" from "geo-1.2.adtx"` adds an extension package;
  `uninstall geo` removes it ([interop.md §9.6](interop.md#96-extensions)).
- `adopt tzdb` adopts a release of the zone rules
  ([catalog.md](catalog.md#31-zone-rules)).
- `retire bundle` and `supersede bundle` are commands (§12), never
  operators: they are not written in a migration.

Rulings: syntax/operator/verb, evolution/smo, evolution/add, evolution/remove, evolution/rename, evolution/split, evolution/merge, evolution/move-field, evolution/nest, evolution/split-routing/match-route, evolution/merge-tag/discriminator, evolution/hoist/default, derived/defaults/backfill/separate, derived/materialize/storage-choice, derived/computed/virtual, interop/ext-pkg/catalog-module, evolution/in-log, evolution/log-ops, evolution/revert, evolution/scripted, evolution/types, surface/ctor-set/growth/declare, constraint/scope/new-ctor/arm-in-add, spec/evolution/ctor-order/placed, spec/evolution/enable-inverse/prior-state, spec/evolution/revert-fill/per-field

## 9. Bundle files

```ebnf
bundle_header = "bundle" BUNDLE_NAME "pins" version [ "catalog" version ]
                "edition" TEXT { header_clause }
header_clause = "lease" DURATION
              | "priority" LOWER
              | "budget" record_expr
              | "hide" "new" "constructors"
import        = "import" LOWER
              | "import" "fn" QLOWER
feed_decl     = "feed" LOWER "of" relation_ref
```

```adt
-- bundles/admin.adb
bundle admin-1.0 pins 12 (c41e...) edition "2026-10" lease 90d
import auth
import fn display_name
fn google_users () -> Set User =
  from u in users where exists g in u.<identities.Google.user select u
query list_google_users () -> Set User = from u in google_users() select u
feed oauth_changes of identities.OAuth

bundle monitor-1.3 pins 9 (5d0e...) catalog 4 (e07b...) edition "2026-10"
bundle batch-1.2 pins 9 (5d0e...) edition "2026-10" priority background budget { memory: 0.25, workers: 2 }
```

- `pins` names the schema version the bundle reads and that version's
  content hash, written in full (examples elide it as `c41e...`).
- `catalog` names the catalog version a bundle that reads catalog
  relations pins ([evolution.md](evolution.md#10-engine-releases)).
- `edition` names the spec edition the bundle is written in; it is
  required ([README.md](README.md#editions)).
- `lease 90d` gives the bundle's lease. `priority` and `budget` give
  its scheduling class and resource shares
  ([catalog.md](catalog.md#82-bundle-quotas)).
- `hide new constructors` makes the bundle never block `enable r.C`: its
  reads of a whole relation, and references whose type admits a
  constructor it does not know, are translated to its pinned
  constructors, so rows of newer constructors are invisible to it
  ([applications.md](applications.md)). It still blocks `enable T.C`
  when it reads a field whose type reaches `T`.
- There is no owner clause: the bundle's owner is the role that
  registers it ([security.md](security.md#3-grants)).
- `import auth` imports a module's names; `import fn display_name`
  imports a schema function so the bundle may call it.
- Bundle ordinals go to `query`, `txn`, `import fn` and `feed`
  declarations, in the order the file writes them, from 1. A
  bundle-local `fn` and a module import take none.
- A bundle holds `fn`, `query`, `txn`, `feed` and `aggregate`
  declarations. `feed
  NAME of r.S` declares a change stream of the constructor relations
  of `r.S` ([interop.md](interop.md#52-change-streams)).

Registration, pinning, leases and retirement are in
[applications.md](applications.md).

Rulings: syntax/module/header, surface/registration/unit/bundle, surface/reader-tracking/bundles, surface/reader-tracking/dormant/lease, surface/both, lang/defs-home/bundle, lang/registered/named

## 10. Grants and roles

```ebnf
grant     = "grant" privilege { "," privilege } "to" LOWER [ "with" "grant" "option" ]
revoke    = "revoke" privilege { "," privilege } "from" LOWER
privilege = "read" relation_ref [ field_list ]
          | "insert" relation_ref
          | "update" relation_ref [ field_list ]
          | "delete" relation_ref
          | "retag" relation_ref "->" relation_ref
          | "call" ( BUNDLE_NAME | FAMILY ) [ "::" LOWER ]
          | "call" "fn" QLOWER
          | "adhoc" | "assert" | "schema" | "monitor" | "cancel"
          | "maintain" | "configure" | "read_files" | "write_files"
field_list = "(" LOWER { "," LOWER } ")"
role_stmt = "create" "role" LOWER
          | "alter" "role" LOWER "password" prefix_e
          | "drop" "role" LOWER
peer_stmt = "map" "peer" TEXT "on" LOWER "to" LOWER
          | "unmap" "peer" INT "on" LOWER
```

Grant operators, in a migration:

```adt
-- 0013
grant read identities.(EmailPass | Google) to support
grant read identities.(EmailPass | Google) (user) to support
grant read identities.EmailPass (user, email) to support
grant update billing::invoices.Open (amount) to web_owner
grant retag billing::invoices.Open -> billing::invoices.Paid to clerk
grant call fn display_name to support
```

Ad hoc statements:

```adt
-- given p: Text
grant call web-2.4::login_key to support
grant call web-2.4 to web
grant call web::login_key to app
grant monitor, cancel to oncall

create role web
alter role web password p
drop role web

map peer "deploy" on unix to web
unmap peer 2 on unix
```

The examples use `billing::invoices` as [derived.md](derived.md) declares it.

- The privileges are exactly the sixteen listed. A field list names
  only header fields or fields every constructor of the set declares;
  a field of one constructor is granted per constructor. A read grant
  includes the key field without listing it.
- `call` names one registered query or transaction of a bundle, or with
  no `::` every one of them. A family name (`web::login_key`) covers
  that query in every bundle of the family whose query of that name
  has the same signature, compared by type ids. A bundle call grant
  covers only the bundle's registered queries and transactions;
  `call fn` names a schema function a role may call directly, and a
  direct call needs it.
- Relation and constructor-set grants (`read`, `insert`, `update`,
  `delete`, `retag`) and `grant call fn` are operators: they are
  written in a migration and are part of the log, except grants on
  `catalog` and `sys` relations. Roles, passwords, peer mappings,
  grants on `catalog` and `sys` relations, capability grants (`adhoc`, `assert`, `schema`, `monitor`, `cancel`,
  `maintain`, `configure`, `read_files`, `write_files`) and bundle call
  grants are deployment state: they are ad hoc statements for holders
  of `schema` ([evolution.md](evolution.md#4-operators)). Each kind is
  refused in the other place.
- A grant with no field list covers the fields that exist when it is
  granted. A grant on a whole relation also covers the fields of the
  constructors the relation gains later; any other field added later
  needs a new grant.
- `with grant option` on a relation grant lets the grantee extend the
  privilege only through bundles it registers, which run with its
  rights; it never lets the grantee write grant operators.
- `map peer "pattern" on L to R` appends a rule, with the next
  ordinal of listener `L`, mapping a peer that matches the pattern to
  role `R`; in the pattern `*` matches any run of characters. `unmap
  peer n on L` removes the rule with ordinal `n`. The rules are rows of
  `catalog::peer_mappings`, keyed by listener and ordinal
  ([security.md](security.md#22-sources)).
- In `alter role R password p`, `p` is a `Text` parameter of the
  call, never written in the text.

Meaning in [security.md](security.md#3-grants).

Rulings: syntax/ctor-set/paren, security/grain/ctor-set, security/fields/field-list, security/transition/transition, security/rights/definer, spec/password-param/text

## 11. The prelude

The prelude is a module every file reads without qualification. Its
sums used by this part are:

```adt
type Bool ordered = False | True
type Option a ordered = None | Some { value: a }
type Result a e = Ok { value: a } | Err { error: e }
```

The full prelude, its types and its functions, is listed in
[prelude.md](prelude.md). Every prelude type name (`Bool`, `Option`,
`Result`, `Tag`, `Span`, `Uuid`, `Rounding`, `Overflow`, `Fault`,
`Error` and the other sums prelude.md lists) and every prelude function
name (`count`, `scalar_len`, `covers`, `div`, `uuid`, ...) is reserved
as a type or function name: an implementation must refuse a schema or
bundle declaration of a type or function with one of these names,
except a schema's own `Principal`, which only the schema's first
migration may declare
([prelude.md](prelude.md#26-error-audit-sums-and-principal)).

Rulings: model/prelude-bool, model/user-params

## 12. Commands

```ebnf
command = "explain" [ "analyze" ] ( query_ref "(" [ call_args ] ")" | expr | write_expr | transact_block )
        | "pin" "plan" prefix_e | "unpin" "plan" prefix_e
        | "cancel" "call" prefix_e
        | "terminate" "session" prefix_e
        | "vacuum" QLOWER
        | "verify" [ QLOWER ]
        | "reindex" QLOWER [ "index" index_fields "in" ctor_scope ] [ "online" | "offline" ]
        | "compact"
        | "finalize" "upgrade"
        | "retire" "bundle" BUNDLE_NAME
        | "supersede" "bundle" BUNDLE_NAME "by" BUNDLE_NAME
        | "set" "setting" UPPER "=" expr
        | "export" or_e "to" TEXT [ "sums" "text" ]
        | "register" "bundle" TEXT
        | "apply" "migrations" TEXT { TEXT }
        | "print" "schema"
        | "archive" "audit" "through" add_e "to" TEXT
        | "rotate" "key" ( "kek" TEXT | "dek" )
        | "destroy" "key" "dek" INT
        | "dump" "to" TEXT [ "encrypt" prefix_e ]
        | "restore" "from" TEXT "to" TEXT [ restore_point ]
        | "backup" "to" TEXT [ "incremental" ]
restore_point = "before" "version" INT
              | "at" "instant" add_e
              | "at" "position" add_e
query_ref = BUNDLE_NAME "::" LOWER
```

```adt
-- given u: Key users, k: Text
explain from i in identities where i.user == u select i
explain analyze web-2.4::login_key(users.key(3))
retire bundle web-2.3
supersede bundle ios-4.1 by ios-4.2
set setting VacuumBudget = { io: 0.2, cpu: 0.5 }
export (from i in identities.Google select i) to "google.parquet"
cancel call sys::calls.key(812)
register bundle "bundles/web.adb"
apply migrations "migrations/0013" "migrations/0014"
print schema
archive audit through 991204 to "audit-2026-10.arrow"
rotate key kek "kms://vault/adt-app/v3"
destroy key dek 7
dump to "app.dump" encrypt k
restore from "s3://backups/app" to "restored.adt" before version 12
backup to "s3://backups/app" incremental
```

- A command is ad hoc text (§3.1). It never appears in a migration, a
  bundle, a `transact` block, a function or a constraint.
- `explain web-2.4::login_key(args)` explains a registered query at the
  bundle's pinned version, with the given arguments.
- `set setting` changes a setting and needs `configure`.
- `supersede bundle B by B2` marks bundle `B` superseded by `B2`,
  which must be a live bundle of the same family. It is given by `B`'s
  owner or the schema owner ([applications.md](applications.md#44-supersession)).
- `register bundle` registers a bundle file under the rules of
  [applications.md §4.1](applications.md#41-registration); `apply
  migrations` applies migration files in order and needs `schema`;
  `print schema` returns the folded schema as a schema file.
- `dump` writes plaintext unless `encrypt` gives a key: a `Text`
  passphrase, a parameter.
  It needs `maintain` and `read` on every relation it dumps, and writes
  the security section only for a holder of `schema`.
  `restore` writes a new database file; it never replaces an open one.
  In `restore ... at position p`, `p` is a `Position`, and in `at
  instant t`, `t` is an `Instant`.
- The CLI (`$ adt ...`) and the in-process API wrap these commands.
- `export` needs `write_files`, and its path must lie within the
  export roots configured at open.

The commands' effects and privileges are in
[catalog.md](catalog.md#9-operational-commands) and
[interop.md](interop.md#51-export).

Rulings: spec/dump-key/passphrase
