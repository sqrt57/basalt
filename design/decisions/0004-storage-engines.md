# ADR 0004: Storage Engines

Status: Proposed (2026-07-10)

## Context

Reference points considered (see
[backlog.md item 4](../backlog.md#4-implementation-architecture)):
PostgreSQL, SQLite, MS SQL Server, InterSystems IRIS, GT.M/YottaDB.

The backlog framed a choice between row store, column store, and a
single universal sparse-array primitive (the IRIS/GT.M pattern, where
relational tables are encoded as subscripts over one underlying tree). A
universal-primitive design was explored in detail — encoding rows as
`(table_id, primary_key, column_id) → value` entries, relying on B+-tree
leaf ordering/linking to keep a row's columns physically adjacent — and
rejected in favor of keeping relational and hierarchical data as
genuinely separate engines with separate APIs. This avoids forcing SQL
to be the only way to reach hierarchical data, and avoids re-deriving
relational schema/type/constraint enforcement on top of a storage layer
that has no concept of "column." The cost is two storage engines to
build and maintain instead of one, and no automatic data-model interop
between them at the storage layer.

Concurrency control and durability across these engines are decided
separately in [0008](0008-concurrency-durability.md); query execution
over them in [0009](0009-query-execution.md).

## Decision

Basalt has **two separate, purpose-built storage engines** rather than a
single universal storage primitive or an arbitrary-plugin architecture:

- **Relational tables**: a row store — tuples stored together, tuple-per-
  page layout — matching the Postgres/MySQL/SQL Server family.
- **Hierarchical tables**: a sparse hierarchical/associative array, in the
  style of GT.M/YottaDB "globals," with its own multidimensional-subscript
  API distinct from the relational one — rather than SQL being the only
  entry point to it (unlike IRIS, which builds SQL as the sole layer on
  top of globals).

Both engines are physically backed by **B+-trees**.

## Consequences

- Two storage engines to design, build, and maintain — more surface area
  than a single universal primitive, but each engine's internals stay
  simpler and closer to well-understood precedent (row store ≈ Postgres/
  MySQL; globals ≈ GT.M/YottaDB).
- No built-in interop between relational and hierarchical data at the
  storage layer; any bridging (e.g. exposing hierarchical data through
  SQL) would be a higher-level feature layered on top, not a storage-
  layer given.
- Both engines' scan/seek operators need to expose a common iterator
  interface so the tree-walking executor ([0009](0009-query-execution.md))
  can drive either engine without engine-specific executor code.
