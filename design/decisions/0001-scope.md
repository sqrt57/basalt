# ADR 0001: Scope

Status: Proposed (2026-07-10)

## Context

Three inputs drove this:

- **Motivation**: learning/research. The goal is to understand how a
  database engine works end-to-end (storage, transactions, query
  processing, wire protocol) — not to hit production SLAs or feature-match
  incumbents.
- **Deployment model**: client/server, single-node. A standalone server
  process on one machine, no built-in clustering — closer to early
  Postgres/MySQL than to SQLite (no embedding requirement) or to
  distributed systems like CockroachDB.
- **Resourcing**: one person, spare-time. Scope has to stay small enough
  to actually finish and keep maintaining, not just start.

## Decision

Basalt is a **single-node, client/server relational database engine**,
built as a **solo learning/research project**.

## Consequences

**In scope:**
- A real server process speaking a network protocol to clients.
- A genuine (if minimal) relational storage and query engine — not a toy
  that skips durability or transactions to save time.
- Incremental SQL support, grown as far as remains interesting/learnable.

**Out of scope for now:**
- Distributed/clustered operation: replication, sharding, multi-node
  transactions, HA/failover.
- Embedded/in-process linking mode (no SQLite-style "just a library"
  target).
- Heavy enterprise tooling: elaborate RBAC, monitoring suites, backup
  orchestration. A minimal admin surface is fine; building a full
  enterprise ops story is not a goal.

**Priorities:**
- Clarity and correctness of implementation outrank raw performance or
  feature completeness.
- Every later decision (platform, architecture, SQL surface, tooling)
  should be evaluated against "can one person build and understand this
  in their spare time," not against "what would a commercial engine
  need."
