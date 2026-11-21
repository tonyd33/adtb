//! The variables by which `cargo xtask` hands the test of every test binary's heap cap the
//! listing of the test binaries it built.

/// The variable naming the listing of the test binaries `cargo xtask` built.
pub const TEST_BINARIES: &str = "ADTB_TEST_BINARIES";

/// The variable set on every command `cargo xtask` starts.
pub const UNDER_XTASK: &str = "ADTB_UNDER_XTASK";
