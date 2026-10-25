# Apache DataFusion

kind: system
home: https://datafusion.apache.org/

## Execution

- Default `datafusion.execution.batch_size` 8192 rows; `target_partitions` defaults to the number of CPU cores; `coalesce_batches` merges small batches after selective filters or joins; `collect_statistics` default true. [V 2026-10-25] https://datafusion.apache.org/user-guide/configs.html
- `UnionExec` concatenates partitions of its inputs without copying (N + M output partitions); sorted input partitions stay sorted. [V 2026-10-25] https://docs.rs/datafusion/latest/datafusion/physical_plan/union/struct.UnionExec.html
- Pull-based async streams of Arrow RecordBatches (`SendableRecordBatchStream`); parallelism by Volcano-style exchange (`RepartitionExec`). Lamb et al., SIGMOD 2024 industry track. [S 2026-10-25] https://docs.rs/datafusion/latest/datafusion/
