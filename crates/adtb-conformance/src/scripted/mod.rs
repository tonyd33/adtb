//! Engine scripts: the scripts of a corpus run on the simulator.
//!
//! A script is read as a conformance script is, with these directives:
//!
//! - `seed N`, first: the simulator's seed, 0 without it.
//! - `use NAME`: the directives of `fixtures/NAME.fixture`, in place.
//! - `option query_memory_bytes N`: the open option for each later open; `default` clears it.
//! - `migrate [at V] [refused VALUE]`: the body's migrations, each starting at a `module` line,
//!   appended to the log, after its first V - 1 when `at V`, and the file reopened with it;
//!   `at V` with no body reopens the file with the first V - 1.
//! - `reopen [refused VALUE]`: the file reopened with the same log.
//! - `log`: the body's migrations appended to the log the next open applies.
//! - `bundle HEADER [use NAME] [refused VALUE]`: the bundle file of `HEADER` and the body, or
//!   of `fixtures/NAME.bundle`, registered at the next open; with no HEADER the file
//!   has none. With `refused`, the file is refused with VALUE and not registered. A script opens
//!   the file after each bundle.
//! - `call NAME [ORDINAL [committed] [rowsort]] [error VALUE]`: bind NAME and call ORDINAL with
//!   the value of each body line, a query, as an argument. The rows after `----` are the
//!   elements of a set result in printed order, or the one value of another; `rowsort` compares
//!   them as a multiset. `committed` states that the call is of a transaction and commits, its
//!   rows the transaction's value. With no ORDINAL, the call takes no body and no rows, and the
//!   handle bound is kept: later calls of NAME in the same open go through it.
//! - `advance N(s|m|h|d)`: the simulated clock advanced.
//! - `role NAME`: the role created.
//! - `host NAME...`: host functions supplied at each later open and restore, each of version
//!   `1.0`, deterministic, answering its own name as text. `host none` supplies none from then on.
//! - `grant [error VALUE]`, `revoke [error VALUE]`: each line of the body, one grant or revoke
//!   statement, given in order; only the last may be refused, with VALUE.
//! - `register HEADER [use NAME] [error VALUE]`: the bundle file `bundle` states, registered
//!   now.
//! - `run`, `query [rowsort]`: as in a conformance script.
//! - `command [rowsort]`: the body, a command or other text, one call in process, its one result
//!   compared with the rows as a `query`'s.
//! - `plan`: the body, one explain sent as a `command` is, its plan document compared with the
//!   JSON object after `----`, which states `format`. Each member the object states is compared,
//!   and only these: `format`, `change` and `constraints` as equal; a node's `arms` and `pruned`
//!   as sets, each entry naming its constructor once by `name`, an arm's `child` as a node;
//!   `rows` and `loops` of `actual`; each of a node's `children` against any node below it in
//!   preorder, through children and arms, the first after the one the child before matched. An
//!   empty `children` or `actual` differs. Each node of the plan holds `kind`, `estimate` and the
//!   array `children`, and each arm, pruned constructor and `actual` its members.
//! - `probe plan`: the body, one explain sent as a `command` is; the rows after `----` are the
//!   lines of its `text`, in order. An explain that runs its statement differs.
//! - `error VALUE`: as in a conformance script, the last statement refused with exactly the
//!   reason VALUE in textual syntax.
//! - `adhoc error VALUE`: the body, one ad hoc call in process committing on its own, refused
//!   with exactly VALUE.
//! - `as ROLE` before `run`, `query`, `error`, `call`, `grant`, `revoke` or `register`: the step
//!   acts through a session of ROLE, a server's own path. A `run`, `query` or `error` body is
//!   then one ad hoc call, refused as a whole, and a `call` binds through the session. Without
//!   it, `grant`, `revoke` and `register` act as `owner` and the others in process.
//! - `failed VALUE`, after `as ROLE`: the body is admitted and fails at completion with VALUE;
//!   `error` takes only a refusal at the submit.
//! - `drop role NAME [error VALUE]`: the role dropped.
//! - `loose NAME`: the body, registration NAME with no bundle file, registered at the next open.
//! - `begin [read only] [deferrable] NAME [error VALUE]`, `commit NAME [position P] [error
//!   VALUE]`, `rollback NAME`: interactive transaction NAME; one open at the next open differs.
//!   `$P` in a later body or compared value stands for the position the commit took, in textual
//!   syntax; a second binding of P differs.
//! - `in NAME` before `run`, `query`, `error`, `plan`, `probe plan` or a `call` with an ordinal:
//!   one call of transaction NAME; `committed` states a registered transaction committing with
//!   it.
//! - `assert VALUE` before `run`, `query`, `error`, `call` or `begin`, after any `as`: the call
//!   or transaction runs for principal VALUE. With `in` or `assert` a body is one call.
//! - `fatal VALUE`: the body, one ad hoc call in process, failing with exactly `Fatal { reason:
//!   VALUE }`.
//! - `restore SOURCE to FILE [at POINT] [error VALUE | fatal VALUE]`: the dump or backup at
//!   SOURCE restored to the new file FILE, refused with `Rejected { reason: VALUE }` or `Fatal {
//!   reason: VALUE }` when stated. POINT is `version N` or `$P`; with none, the latest. With no
//!   refusal, the rows after `----` are the grants the restore reports dormant, in its order,
//!   each `dormant { id: N, grantee: G, privilege: P, target: T }`. Once a restore succeeds,
//!   every later open opens FILE. A restore with no database open runs as a process of its own.
//!
//! These name a page by the structure holding it, as PAGE: `heap RELATION CTOR`, the first data
//! page of the heap of constructor CTOR; `directory RELATION CTOR`, the root of that heap's
//! directory; `large RELATION CTOR`, the first page of the store of that heap's large values;
//! `index RELATION CLAUSE`, the root of the index of clause CLAUSE as the schema writes it;
//! `catalog`, the catalog's root; `commit log`, the commit log's root; `status`, the commit log's
//! status page of the newest transaction ids. A page is found in the open database when the step
//! runs.
//!
//! - `mark OBJECT`: the mark of OBJECT set, durably: `database`, `heap RELATION CTOR` or `index
//!   RELATION CLAUSE`.
//! - `damage PAGE`: every later read of the page in this open refused as failing its checksum,
//!   and the damage recorded.
//! - `flip PAGE`: a byte of the page's body inverted on the device once this process closes the
//!   database, so that the next open reads it failing its checksum.
//! - `lose PAGE`: every later write of this process covering the page completes and changes
//!   nothing.
//! - `tear PAGE`: of the next write covering the page, a crash before its force lands only the
//!   first atomic unit.
//!
//!   A `damage` that no read or latch of its page meets before the close differs, as does a
//!   `flip` that no read of its page meets by the end of the script, and a `lose` or `tear`
//!   whose process ends with no write it acts on.
//! - `scribble PAGE`: the page edited as the body's one line states, as one logged change that
//!   keeps its checksum valid: `kind KIND`, its header relabelled to page kind KIND, by name as
//!   `HeapFixed`; `entry N names PAGE`, entry N of a directory page naming that page, its count
//!   raised to hold it; `no map`, a directory root naming no visibility map; `slot N broken`, slot
//!   N of a slotted page in a state no slot holds; `slot N deleter unreserved`, the version in slot
//!   N deleted by the first transaction id past the commit log's reservation; `slot N count C`,
//!   the large value's head in slot N counting C versions. On an index, `entry N again at slot S`:
//!   the index's entry N added again, naming slot S of the page its own names.
//! - `leak PAGE`: a page of the kind and owner of PAGE allocated, zero, that no structure names.
//! - `free PAGE`: the page freed in the allocation bitmap, whatever still names it.
//! - `crash at KIND PATH`: the process crashes at the first device op of KIND on PATH, before
//!   the op completes; the path of a `rename` is its new name. KIND is one of
//!   `open`, `create_new`, `write`, `allocate`, `sync`, `sync_dir`, `remove`, `remove_dir`,
//!   `rename` and `create_dir`.
//! - `crash after N`: the process crashes at the device op after the next N, before it
//!   completes; a timer is no device op.
//!
//!   A crash fires in the steps after it or the close, and a process that ends without it
//!   differs. Steps after the one it fires in, up to the next `migrate`, `reopen` or `probe
//!   files`, do not run, and differ.
//! - `probe marks`: the rows after `----` are the name of each mark set, in memory or in the
//!   file, as `Quarantined` names it, sorted.
//! - `probe damage`: the rows are each damage recorded and not cleared: each page as `PAGE:
//!   DAMAGE` by the page a fault named, `a page leaked beside PAGE` for a `leak`, `the page
//!   entry N of PAGE names` for an index's `scribble`, or `a page no fault names: DAMAGE`,
//!   sorted; then each finding of the log as `log: PROBLEM`; then `the damage list is full` when
//!   it is. DAMAGE is the variant of the engine's damage, its shape named: `Checksum`,
//!   `Unreadable`, `Shape(Blank)` and the other shapes, `Stale`, `LogUnproven` or `Settled`.
//!   PROBLEM is the variant of the log's problem, `Unlogged PAGE` naming its page. Neither names
//!   a log position.
//! - `probe holding verify`: the rows are those of the run of `verify` that holds the database,
//!   as `adtb verify FILE` runs it, printed and sorted, each `page N` written as `probe damage`
//!   names page N.
//! - `probe heap RELATION CTOR [pages]`: the rows are, for each position of the directory of the
//!   heap of CTOR in order, `P: hole`, or `P: KIND format F, N slots` with `, flagged all
//!   visible` when the page header says so and `, mapped all visible` when the visibility map
//!   does; then, without `pages`, a row `P.S: SLOT` for each of its slots: `Unused`, `Dead`,
//!   `Redirect P.S`, or `Normal FLAGS, xI[ to xD][, next P.S]` with the version's flags joined
//!   by `|` or `NONE`, its inserter and deleter, and its successor, `elsewhere` off the heap's
//!   pages. A transaction id `xN` is ranked among the ids the probe names, in id order.
//! - `probe files [PREFIX]`: the database closes first, as before an open, and the rows are the
//!   path of each file on the device beginning with PREFIX, sorted.
//!
//! An instant is compared by rank, not value: `tN` in a row or an expected error, outside a text
//! literal, stands for the instant of rank N, in time order, among the distinct instants the run
//! produces where the script's tokens stand. A script with a token refuses one in a body. In a
//! difference of such a script each instant produced appears as its token. Every instant a
//! script compares is written as a token.
//!
//! `pins N` with no hash in a header or body gains the hash of the first N migrations of the log:
//! of that open's in a bundle file, else of the script's, each `log` step's included.
//!
//! An open refused with VALUE, the open refusal in textual syntax, leaves the file closed. A
//! refusal `MigrationRefused` of version V leaves the log at its first V - 1 migrations. Any
//! other refusal leaves the log as before the step, the migrations of each `log` step since
//! the last open included. Each open is a new simulated process. No open runs the background
//! vacuum. A script holds a directive besides `seed` and `use`; an area holds only scripts and
//! directories.
//!

mod clock;
mod fault;
mod pages;
mod plan;
mod read;
mod role;
mod session;
mod txn;

use std::time::Duration;

use crate::runner::shown;
use crate::{Block, Difference};

#[derive(Clone, Debug)]
enum Head {
    Seed(u64),
    Use(String),
    QueryMemory(Option<u64>),
    Migrate {
        at: Option<usize>,
        refused: Option<String>,
    },
    Reopen {
        refused: Option<String>,
    },
    Role(String),
    DropRole {
        name: String,
        error: Option<String>,
    },
    Log,
    Bundle {
        header: String,
        fixture: Option<String>,
        refused: Option<String>,
    },
    Call {
        name: String,
        ordinal: Option<u32>,
        committed: bool,
        rowsort: bool,
        error: Option<String>,
    },
    Advance(Duration),
    Grant {
        revoke: bool,
        error: Option<String>,
    },
    Register {
        header: String,
        fixture: Option<String>,
        error: Option<String>,
    },
    Host(Vec<String>),
    Loose(String),
    Begin {
        name: String,
        read_only: bool,
        deferrable: bool,
        error: Option<String>,
    },
    Commit {
        name: String,
        position: Option<String>,
        error: Option<String>,
    },
    Rollback(String),
    Run,
    Query {
        rowsort: bool,
    },
    Command {
        rowsort: bool,
    },
    Plan,
    ProbePlan,
    Error(String),
    Adhoc(String),
    Failed(String),
    Fatal(String),
    Mark(fault::Marked),
    Fault {
        fault: fault::Fault,
        page: fault::Page,
    },
    Crash(fault::CrashAt),
    Restore(fault::Restore),
    Probe(fault::Probed),
}

/// One directive, with the file and line it was read from.
#[derive(Clone, Debug)]
struct Step {
    at: String,
    line: usize,
    /// The fixture file and line the step comes from, when a `use` brought it in.
    via: Option<(String, usize)>,
    /// The role an `as` prefix names, the step then running through a session of it.
    role: Option<String>,
    /// The transaction an `in` prefix names, the step then one call of it.
    txn: Option<String>,
    /// The principal an `assert` prefix states, in textual syntax.
    assert: Option<String>,
    head: Head,
    body: String,
    rows: Vec<String>,
}

/// Returns the blocks of the engine script `text`.
pub(crate) fn blocks(text: &str) -> Result<Vec<Block>, Difference> {
    crate::blocks(text, read::starts, read::takes_rows)
}

pub use session::run_file;
