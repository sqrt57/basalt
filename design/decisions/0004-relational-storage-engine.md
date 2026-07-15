# ADR 0004: Relational Storage Engine

Status: Proposed (2026-07-10)

## Context

Reference points considered (see
[backlog.md item 4](../backlog.md#4-implementation-architecture)):
PostgreSQL, SQLite, MS SQL Server, and — for the alternative rejected
below — InterSystems IRIS, GT.M/YottaDB.

The backlog framed a choice between row store, column store, and a
single universal sparse-array primitive (the IRIS/GT.M pattern, where
relational tables are encoded as subscripts over one underlying tree). A
universal-primitive design was explored in detail — encoding rows as
`(table_id, primary_key, column_id) → value` entries, relying on B+-tree
leaf ordering/linking to keep a row's columns physically adjacent — and
rejected in favor of keeping relational and hierarchical data as
genuinely separate engines with separate APIs, each implementable on its
own (hierarchical engine decided separately in
[0010](0010-hierarchical-storage-engine.md)). This avoids forcing SQL to
be the only way to reach hierarchical data, and avoids re-deriving
relational schema/type/constraint enforcement on top of a storage layer
that has no concept of "column." The cost is two storage engines to
build and maintain instead of one, and no automatic data-model interop
between them at the storage layer — offset by both sharing the same
concurrency/durability substrate ([0008](0008-concurrency-durability.md)),
which is what keeps a transaction spanning both engines possible even
though the engines themselves are otherwise independent.

Query execution over this engine (and the hierarchical one) is decided
in [0009](0009-query-execution.md).

## Decision

The relational storage engine is a **row store** — tuples stored
together, tuple-per-page layout — matching the Postgres/MySQL/SQL Server
family, physically backed by a **B+-tree**.

## Consequences

- Can be designed, built, and maintained independently of the
  hierarchical engine ([0010](0010-hierarchical-storage-engine.md)) — no
  shared internal data structures required between them, only the shared
  concurrency/durability substrate ([0008](0008-concurrency-durability.md))
  that keeps cross-engine transactions possible.
- No built-in interop with hierarchical data at the storage layer; any
  bridging (e.g. exposing hierarchical data through SQL) would be a
  higher-level feature layered on top, not a storage-layer given.
- Needs to expose scan/seek operators through an iterator interface
  common with the hierarchical engine, so the tree-walking executor
  ([0009](0009-query-execution.md)) can drive either engine without
  engine-specific executor code.
- Needs to participate in the shared MVCC/WAL substrate
  ([0008](0008-concurrency-durability.md)) rather than an independent
  concurrency/durability scheme of its own, so transactions touching
  both engines commit atomically.
- Pages are copy-on-write, not updated in place
  ([0008](0008-concurrency-durability.md)): a write allocates new pages
  along the path to the root rather than mutating a live page, so
  "tuple-per-page layout" describes the logical layout within a page,
  not an in-place update strategy.
