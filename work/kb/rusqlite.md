# rusqlite

kind: system
home: https://github.com/rusqlite/rusqlite

## Native SQLite interface

- Published API documentation identifies version 0.41.1 and exposes connections, prepared statements, typed parameter/result conversion and libsqlite3_sys as ffi. [V 2026-10-31] https://docs.rs/rusqlite/0.41.1/rusqlite/
- The project documents libsqlite3-sys discovering an installed SQLite library with pkg-config by default, or accepting SQLITE3_LIB_DIR and SQLITE3_INCLUDE_DIR. The bundled feature instead compiles embedded SQLite source; the README currently describes that source as 3.54.0 for rusqlite 0.41.0/libsqlite3-sys 0.39.0. This statement does not establish the bundled version in every later crate release. [V 2026-10-31] https://github.com/rusqlite/rusqlite

## Exact 0.41.1 package verification

- Cargo's published rusqlite 0.41.1 manifest with default-features=false resolves libsqlite3-sys 0.39.1. Adding only the bundled feature activates rusqlite's modern_sqlite and libsqlite3-sys's bundled/bundled_bindings/cc features. Verified from downloaded crates.io package manifests and resolved metadata; wrapper defaults are off but dependency defaults still activate pkg-config/vcpkg metadata dependencies. [V 2026-10-31] https://docs.rs/crate/rusqlite/0.41.1/source/Cargo.toml ; https://docs.rs/crate/libsqlite3-sys/0.39.1/source/Cargo.toml
- The libsqlite3-sys 0.39.1 packaged sqlite3.h identifies SQLite 3.54.1 and source ID `2026-10-16 14:45:20 555e07ef33f69a0957d24d8e555b702464a131d6f1607f4c3c1e4551e8e6ca76`. Its packaged sqlite3.c SHA3-256 is `e2b869d3003b33c942b7dd7063604fbc53553e1b66609570df39b9b7b558ee81`, matching SQLite's official 3.54.1 release hash. [V 2026-10-31] https://docs.rs/crate/libsqlite3-sys/0.39.1/source/sqlite3/sqlite3.h ; https://sqlite.org/changes.html
- The 0.39.1 build script selects packaged sqlite3/sqlite3.c for a normal bundled build, but LIBSQLITE3_SYS_USE_PKG_CONFIG set to a nonzero value overrides that selection and uses linked discovery. LIBSQLITE3_FLAGS supplies additional C defines/undefines. [V 2026-10-31] https://docs.rs/crate/libsqlite3-sys/0.39.1/source/build.rs
