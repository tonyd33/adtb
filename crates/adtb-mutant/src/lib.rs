//! `mutant!(ID, original, mutated)` marks a temporary mutant site. Without the `active` feature it
//! is `original` through a deprecated identity function, so `clippy -D warnings` refuses any site.
//! With it, the site evaluates `mutated` in a process whose `ADTB_MUTANT` is `ID`, and `original`
//! in any other, and the first such evaluation in a process appends `ID` to the file
//! `ADTB_MUTANT_TRACE` names.

#[cfg(not(feature = "active"))]
#[macro_export]
macro_rules! mutant {
    ($id:ident, $original:expr, $mutated:expr $(,)?) => {
        $crate::site($original)
    };
}

#[cfg(feature = "active")]
#[macro_export]
macro_rules! mutant {
    ($id:ident, $original:expr, $mutated:expr $(,)?) => {
        if $crate::active(stringify!($id)) {
            $mutated
        } else {
            $original
        }
    };
}

#[deprecated = "a mutant site is a temporary edit; remove it before committing"]
pub const fn site<T>(original: T) -> T {
    original
}

/// Returns whether `id` is this process's `ADTB_MUTANT`.
#[cfg(feature = "active")]
pub fn active(id: &str) -> bool {
    use std::io::Write as _;
    static CHOSEN: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    static TRACED: std::sync::Once = std::sync::Once::new();
    let hit = CHOSEN
        .get_or_init(|| std::env::var("ADTB_MUTANT").ok())
        .as_deref()
        == Some(id);
    if hit && let Some(path) = std::env::var_os("ADTB_MUTANT_TRACE") {
        TRACED.call_once(|| {
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .and_then(|mut trace| writeln!(trace, "{id}"))
                .expect("the mutant trace appends");
        });
    }
    hit
}

#[cfg(all(test, not(feature = "active")))]
mod tests {
    extern crate adtb_lang as _;

    #[expect(deprecated, reason = "goes if an inactive site stops warning")]
    const KEPT: u32 = mutant!(M1, 1, 2);

    #[test]
    fn an_inactive_site_is_its_original_in_a_constant() {
        assert_eq!(KEPT, 1);
    }
}
