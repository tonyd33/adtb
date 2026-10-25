---
title: DuckDB readers and extensions
---
read_parquet and read_csv are table functions; a file path in FROM is a replacement scan. Projection and filter pushdown into Parquet. CSV faulty rows go to reject_scans and reject_errors tables with line, column, error type and the original line. Extensions are installed then loaded per session; C++ extensions are pinned to one release, C-API ones to a minimum version.
https://duckdb.org/docs/2.0/data/parquet/overview.html
https://duckdb.org/docs/2.0/data/csv/reading_faulty_csv_files.html
https://duckdb.org/docs/2.0/extensions/overview.html
