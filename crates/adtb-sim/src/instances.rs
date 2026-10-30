use std::hint::black_box;

use adtb::{AdHocTransaction, Database, Load, Registered, RoleAuthority, RoleSession, Task};

use crate::SimRuntime;

/// Compile the engine at [`SimRuntime`] in this crate. Dependents at an opt level that shares
/// generics link these instances.
///
/// Each pointer must reach `black_box`: an unused pointer is optimised away and compiles nothing.
#[used]
static ROOTS: fn() = roots;

fn roots() {
    type D = Database<SimRuntime>;
    type T<'a> = AdHocTransaction<'a, SimRuntime>;
    type G = Registered<SimRuntime>;
    type A = RoleAuthority<SimRuntime>;
    black_box(D::open_in as fn(_, _, _, _) -> _);
    black_box(D::query_text as fn(_, _, _, _) -> _);
    black_box(D::verify as fn(_, _) -> _);
    black_box(D::close as fn(_) -> _);
    black_box(D::begin as fn(_, _) -> _);
    black_box(D::load as fn(_, _) -> _);
    black_box(D::registered as fn(_, _) -> _);
    black_box(T::query_text as fn(_, _, _, _) -> _);
    black_box(T::call as fn(_, _, _, _, _) -> _);
    black_box(T::commit as fn(_, _) -> _);
    black_box(T::rollback as fn(_) -> _);
    black_box(Load::<SimRuntime>::commit as fn(_) -> _);
    black_box(G::call as fn(_, _, _, _) -> _);
    black_box(G::submit as fn(_, _, _, _, _) -> _);
    black_box(A::session as fn(_, _) -> _);
    black_box(A::register as fn(_, _, _) -> _);
    black_box(RoleSession::<SimRuntime>::submit_text as fn(_, _, _, _) -> _);
    black_box(Task::<SimRuntime>::observe as fn(_, _) -> _);
}
