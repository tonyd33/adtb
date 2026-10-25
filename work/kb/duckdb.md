# DuckDB

kind: system
home: https://duckdb.org/

## Concurrency

- MVCC plus optimistic concurrency control in one process; concurrent edits of the same row: the second fails with a conflict error. [V 2026-10-25] https://duckdb.org/docs/2.0/connect/concurrency.html
- Multi-process writes go through the Quack remote protocol or DuckLake with PostgreSQL coordination. [V 2026-10-25] https://duckdb.org/docs/2.0/connect/concurrency.html
- MVCC design follows Neumann, Mühlbauer, Kemper, SIGMOD 2015. [U]

## UNION type

- Implemented on top of STRUCT with the tag first; at most 255 members; orders by tag first. [V 2026-10-25] https://duckdb.org/docs/2.0/sql/data_types/union.html
- `union_tag`; `union_extract` returns NULL for an inactive member. [V 2026-10-25] same page
- Every member column spans all rows physically (sparse-union-like). [U]

## Storage format

- Single file; begins with a u64 checksum of the main header, magic "DUCK", u64 version number. Row groups are horizontal partitions, size settable on ATTACH (ROW_GROUP_SIZE). [V 2026-10-25] https://duckdb.org/docs/2.0/internals/storage.html
- Backward compatibility guaranteed from v0.10 onward; forward compatibility best effort; STORAGE_VERSION on ATTACH writes an older format; storage versions numbered up to 69 (v2.0.0). [V 2026-10-25] https://duckdb.org/docs/2.0/internals/storage.html
- Per-segment lightweight compression: constant, RLE, bit packing, frame of reference, dictionary, FSST, ALP, Chimp, Patas, Zstd; DICT_FSST default in v2.0.0. [V 2026-10-25] https://duckdb.org/docs/2.0/internals/storage.html

## Execution

- `STANDARD_VECTOR_SIZE` default 2048. Vector types flat, constant, dictionary (child plus selection vector), sequence (offset plus increment); a unified vector format is a generic view every vector converts to. Struct vectors hold one child per field; UNION vectors use struct-like storage with a tag vector. [V 2026-10-25] https://duckdb.org/docs/2.0/internals/vector.html
- Switched from pull (GetChunk) to push-based execution in 2021, with a morsel-driven pipeline scheduler. [S 2026-10-25] https://15721.courses.cs.cmu.edu/spring2024/slides/20-duckdb.pdf

## Memory

- memory_limit default 80% of RAM; max_temp_directory_size default 90% of free disk; blocking operators spill to database_file_name.tmp (or temp_directory). [V 2026-10-25] https://duckdb.org/docs/2.0/configuration/overview.html https://duckdb.org/docs/2.0/guides/performance/how_to_tune_workloads.html
- Row group size settable per ATTACH (ROW_GROUP_SIZE). [V 2026-10-25] https://duckdb.org/docs/2.0/internals/storage.html

## Configuration

- Settings via SET/RESET or PRAGMA; read with current_setting() or the duckdb_settings() table function. memory_limit default 80% of RAM; threads default the core count. [V 2026-10-25] https://duckdb.org/docs/2.0/configuration/overview.html

## Import and export

- read_parquet takes a file, list, glob or directory; .parquet paths queried directly via replacement scans; projection and filter pushdown; union_by_name; hive_partitioning; virtual columns filename, file_row_number. COPY TO writes Parquet; EXPORT DATABASE writes a database as Parquet files. [V 2026-10-25] https://duckdb.org/docs/2.0/data/parquet/overview.html
- CSV: ignore_errors skips faulty rows; store_rejects writes temporary tables reject_scans (dialect, schema, formats) and reject_errors (line, byte position, column, error type, original line, message); rejects_limit. Error types: CAST, MISSING COLUMNS, TOO MANY COLUMNS, UNQUOTED VALUE, LINE SIZE OVER MAXIMUM, INVALID ENCODING. [V 2026-10-25] https://duckdb.org/docs/2.0/data/csv/reading_faulty_csv_files.html

## Extensions

- Install (download, store) is separate from load (per session); core extensions autoload on first use; built-in, core and community extensions; extension binaries tied to a DuckDB release. [V 2026-10-25] https://duckdb.org/docs/2.0/extensions/overview.html
- CPP-ABI extensions work only with the version they were built for; C_STRUCT extensions use a versioned function-pointer table (duckdb_ext_api_v1) and declare a minimum version. [S 2026-10-25] https://github.com/duckdb/duckdb/pull/14992
