# ADR 0010: Hierarchical Storage Engine

Status: Proposed (2026-07-10)

## Context

Reference points considered: InterSystems IRIS, GT.M/YottaDB.

Paired with the relational engine
([0004](0004-relational-storage-engine.md)), kept as a genuinely
separate engine with its own API rather than folding hierarchical data
into a single universal storage primitive shared with relational data —
see [0004](0004-relational-storage-engine.md) for why that unification
was rejected (avoiding SQL as the only entry point to hierarchical data,
avoiding re-deriving relational constraint enforcement over a
column-less substrate). The two engines can be implemented separately;
they only need to share a concurrency/durability substrate
([0008](0008-concurrency-durability.md)) so a transaction can span both.

Unlike InterSystems IRIS, which builds SQL as the sole layer on top of
"globals," Basalt exposes the hierarchical engine through its own
multidimensional-subscript API, distinct from the relational/SQL one, in
the style of GT.M/YottaDB.

Query execution over this engine (and the relational one) is decided in
[0009](0009-query-execution.md).

## Decision

The hierarchical storage engine is a **sparse hierarchical/associative
array** ("globals," GT.M/YottaDB-style), with its own
multidimensional-subscript API, physically backed by a **B+-tree**.

## Consequences

- Can be designed, built, and maintained independently of the relational
  engine ([0004](0004-relational-storage-engine.md)) — no shared
  internal data structures required between them, only the shared
  concurrency/durability substrate ([0008](0008-concurrency-durability.md))
  that keeps cross-engine transactions possible.
- Own API surface (multidimensional subscripts), not funneled through
  SQL — unlike IRIS, SQL is not the only way to reach hierarchical data.
- Needs to expose scan/seek operators through an iterator interface
  common with the relational engine, so the tree-walking executor
  ([0009](0009-query-execution.md)) can drive either engine without
  engine-specific executor code.
- Needs to participate in the shared MVCC/WAL substrate
  ([0008](0008-concurrency-durability.md)) rather than an independent
  concurrency/durability scheme of its own, so transactions touching
  both engines commit atomically.
