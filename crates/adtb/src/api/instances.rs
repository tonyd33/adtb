use std::hint::black_box;
use std::path::Path;

use adtb_sys::os::OsRuntime;

use super::{
    AdHocTransaction, Database, Load, Log, OpenError, OpenOptions, Registered, RoleAuthority,
    RoleSession, Task,
};

/// Compile the public API at the host's runtime in this crate. Dependents at an opt level that
/// shares generics link these instances.
///
/// Each pointer must reach `black_box`: an unused pointer is optimised away and compiles nothing.
#[used]
static ROOTS: fn() = roots;

fn open_at_path(path: &Path, log: &Log, options: &OpenOptions) -> Result<Database, OpenError> {
    Database::open(path, log, options)
}

fn roots() {
    black_box(open_at_path as fn(_, _, _) -> _);
    database();
    transaction();
    registered();
    authority();
    task();
}

fn database() {
    type D = Database<OsRuntime>;
    black_box(D::open_in as fn(_, _, _, _) -> _);
    black_box(D::schema as fn(_) -> _);
    black_box(D::query_text as fn(_, _, _, _) -> _);
    black_box(D::verify as fn(_, _) -> _);
    black_box(D::verify_holding as fn(_) -> _);
    black_box(D::damage_kept_store as fn(_));
    black_box(D::write_path as fn(_) -> _);
    black_box(D::explain_checks as fn(_, _) -> _);
    black_box(D::close as fn(_) -> _);
    black_box(D::clone as fn(_) -> _);
    black_box(D::begin as fn(_, _) -> _);
    black_box(D::load as fn(_, _) -> _);
    black_box(D::registered as fn(_, _) -> _);
    black_box(D::role_authority as fn(_) -> _);
}

fn transaction() {
    type T<'a> = AdHocTransaction<'a, OsRuntime>;
    black_box(T::id as fn(_) -> _);
    black_box(T::query_text as fn(_, _, _, _) -> _);
    black_box(T::call as fn(_, _, _, _, _) -> _);
    black_box(T::commit as fn(_, _) -> _);
    black_box(T::rollback as fn(_) -> _);
    black_box(Load::<OsRuntime>::commit as fn(_) -> _);
}

fn registered() {
    type G = Registered<OsRuntime>;
    black_box(G::name as fn(_) -> _);
    black_box(G::pins as fn(_) -> _);
    black_box(G::ordinal as fn(_, _) -> _);
    black_box(G::call as fn(_, _, _, _) -> _);
    black_box(G::call_arrow as fn(_, _, _, _) -> _);
    black_box(G::clone as fn(_) -> _);
    black_box(G::submit as fn(_, _, _, _, _) -> _);
}

fn authority() {
    type A = RoleAuthority<OsRuntime>;
    type S = RoleSession<OsRuntime>;
    black_box(A::session as fn(_, _) -> _);
    black_box(A::create_role as fn(_, _) -> _);
    black_box(A::drop_role as fn(_, _) -> _);
    black_box(A::grant_capability as fn(_, _, _, _) -> _);
    black_box(A::grant_capability_as as fn(_, _, _, _, _) -> _);
    black_box(A::revoke_capability as fn(_, _, _) -> _);
    black_box(A::revoke as fn(_, _, _) -> _);
    black_box(A::require as fn(_, _, _, _) -> _);
    black_box(A::register as fn(_, _, _) -> _);
    black_box(A::principal as fn(_, _, _) -> _);
    black_box(S::role_name as fn(_) -> _);
    black_box(S::submit_text as fn(_, _, _, _) -> _);
}

fn task() {
    type K = Task<OsRuntime>;
    black_box(K::observe as fn(_, _) -> _);
    black_box(K::supply as fn(_, _));
    black_box(K::drained as fn(_));
    black_box(K::cancel as fn(_));
    black_box(K::expire_idle as fn(_, _));
    black_box(K::held as fn(_) -> _);
}
