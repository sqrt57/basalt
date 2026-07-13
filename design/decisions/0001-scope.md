# ADR 0001: Scope

Status: Proposed (2026-07-10), revised (2026-07-13)

## Context

Three inputs drove this:

- **Motivation**: learning/research. The goal is to understand how a
  database engine works end-to-end (storage, transactions, query
  processing, wire protocol) — not to hit production SLAs or feature-match
  incumbents.
- **Deployment model**: single-node, in-process/embedded first — a
  linkable engine core usable directly from a host process, closer to
  SQLite/DuckDB than to a server-only engine. Client/server (a
  standalone server process on one machine, no built-in clustering) is
  a committed second stage layered on that same core — still
  single-node, not a distributed system like CockroachDB.
- **Resourcing**: one person, spare-time. Scope has to stay small enough
  to actually finish and keep maintaining, not just start.

## Decision

Basalt is a **single-node relational database engine**, built as a
**solo learning/research project**. It ships first as an **in-process/
embedded library** — no server process required; a **client/server
mode**, wrapping the same engine core, is a committed second stage, not
the first-stage goal (see Consequences).

## Consequences

**In scope:**
- A genuine (if minimal) relational storage and query engine — not a toy
  that skips durability or transactions to save time — built from the
  start as an independently linkable crate
  ([0004](0004-relational-storage-engine.md),
  [0010](0010-hierarchical-storage-engine.md)), separate from any
  network/wire-protocol layer.
- An in-process/embedded build mode exposing that engine core directly:
  no server process required to use Basalt.
- Incremental SQL support, grown as far as remains interesting/learnable.

**Out of scope for now:**
- Distributed/clustered operation: replication, sharding, multi-node
  transactions, HA/failover.
- Heavy enterprise tooling: elaborate RBAC, monitoring suites, backup
  orchestration. A minimal admin surface is fine; building a full
  enterprise ops story is not a goal.

**Committed second stage:**
- Client/server mode: a standalone server process speaking a network
  protocol ([0007](0007-wire-protocol.md)) to clients, wrapping the same
  engine core the embedded library exposes. The engine/network split
  above exists specifically so this wrapping is straightforward once the
  embedded core is solid — not a first-stage goal, but not optional
  either.

**Priorities:**
- Clarity and correctness of implementation outrank raw performance or
  feature completeness.
- Every later decision (platform, architecture, SQL surface, tooling)
  should be evaluated against "can one person build and understand this
  in their spare time," not against "what would a commercial engine
  need."
