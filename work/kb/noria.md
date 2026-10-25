# Noria

kind: paper
home: https://www.usenix.org/conference/osdi18/presentation/gjengset

## Model

- Gjengset et al., "Noria: dynamic, partially-stateful data-flow for high-performance web applications", OSDI 2018. App supplies a relational schema and parameterized queries; Noria compiles them to a dataflow that precomputes reads and applies writes incrementally. [V 2026-10-25] https://www.usenix.org/system/files/osdi18-gjengset.pdf
- Partial state: operators keep only queried entries; eviction notices flow forward; missing state is rebuilt by recursive upqueries to upstream state. Joins upquery the other side instead of storing it; upqueries impose indexing obligations Noria satisfies. [V 2026-10-25] same
- Only eventual consistency: an update may appear in different views at different times. Compatible with basic optimistic multi-statement transactions; prototype eviction randomized. [V 2026-10-25] same
- Shares state and computation across related queries; adapts the dataflow to schema and query changes online. [V 2026-10-25] same
- Lobsters example: transactions used only to avoid write-write conflicts on precomputed vote counts and hotness; Noria computes hotness in the dataflow, avoiding the conflict at the cost of eventual consistency. [V 2026-10-25] same
