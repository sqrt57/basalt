# Basalt — Open Design Questions

Basalt is a database engine (code name) in the design stage. This
document tracks **open** design/implementation questions — the original
top-level architectural backlog (scope, implementation platform,
supported OSes, storage/execution architecture, SQL support level,
admin/dev client, wire protocol) is now fully decided. See
[architecture.md](architecture.md) for the current-state summary and
`decisions/` for the full ADRs.

What remains here are sub-questions that surfaced while resolving those
ADRs but weren't settled at the time, plus possibilities that were
raised and set aside without a commitment either way.

## Open Implementation Questions

- **TUI vs. GUI build order** — CLI ships first
  ([0006](decisions/0006-admin-dev-client.md)), but the relative order
  of TUI and GUI after that is undecided.
- **Order after roadmap stage 4** — the hierarchical storage engine
  ([0010](decisions/0010-hierarchical-storage-engine.md)) and TUI/GUI/
  Web ([0006](decisions/0006-admin-dev-client.md)) are both unstaged
  past [roadmap.md](roadmap.md) stage 4; their relative order isn't
  decided.
- **Stage-4 concurrent-writer mechanism** — stage 1's single evolving
  root pointer ([0008](decisions/0008-concurrency-durability.md))
  doesn't support concurrent writers as-is; reconciling independent
  copy-on-write changes into one next root (or moving away from a
  single root pointer) isn't decided. If root history stops being
  linear, stage 1's low-water-mark page reclamation
  ([0013](decisions/0013-page-reclamation.md)) stops working too and
  would need to move to per-page refcounting or another scheme — also
  not decided.
- **Web console frontend stack** — expected to reuse the Tauri GUI's
  frontend ([0006](decisions/0006-admin-dev-client.md)), but the actual
  framework/stack choice hasn't been made.
- **Native wire protocol design** — text vs. binary framing,
  request/response shape, auth, streaming results, etc. Flagged as
  unresolved in [0007](decisions/0007-wire-protocol.md).
- **Supporting client crates** — `clap` (CLI flag parsing) and a
  table-rendering crate (`comfy-table` vs. `tabled`) are likely
  defaults, not firmly confirmed
  ([0006](decisions/0006-admin-dev-client.md)).
- **Serializable isolation implementation strategy** — stage 1's
  single-writer restriction may make Serializable reachable for free
  (no concurrent writers means no write skew, the anomaly that
  otherwise separates Snapshot from Serializable); whether that's
  actually true, and worth implementing opportunistically at stage 1,
  isn't confirmed. Once stage 4 brings concurrent writers online
  ([roadmap.md](roadmap.md),
  [0008](decisions/0008-concurrency-durability.md)), that free ride
  ends and true Serializable needs real conflict detection on top of
  the shared MVCC substrate (e.g. Postgres-style SSI/predicate locking
  vs. an alternative). Not decided ([0005](decisions/0005-sql-support.md)).
- **Read Uncommitted implementation strategy** — relevant from stage 1
  onward, since MVCC (and the Snapshot-level consistency it gives by
  default) already exists there: how dirty reads would actually be
  surfaced under this versioned storage model when weaker visibility is
  explicitly requested. Not decided
  ([0005](decisions/0005-sql-support.md)).

- **Embedded config surface beyond DB file location** — isolation
  level, memory/cache limits, sync/fsync policy, read-only mode, and
  anything else the embedded builder/options should expose. Only file
  location (path prefix) is decided so far
  ([0011](decisions/0011-embedded-config.md)). Deliberately left open,
  not blocking stage 1 — to be settled during stage-1 implementation
  itself rather than pre-decided here.
- **Minimum/maximum allowed page size bounds** — page size is
  per-database and stored in the preamble
  ([0015](decisions/0015-page-storage-format.md)), but what values are
  actually valid (power-of-two only? a floor/ceiling?) isn't decided.
  Doesn't block starting stage-1 chunk 1.
- **Total page count storage** — derived from data-file length at open
  time, or stored explicitly in page 0 alongside the free-list head
  ([0015](decisions/0015-page-storage-format.md))? Not decided; doesn't
  block starting stage-1 chunk 1.
- **Server's own config file location** — default path next to the
  executable, a CLI flag pointing at it, or something else. Not decided
  ([0012](decisions/0012-server-config.md)).
- **CLI flag / env var overrides for server config** — whether values
  from the TOML file ([0012](decisions/0012-server-config.md)) can be
  overridden by CLI flags or env vars, and their precedence if so. Not
  decided.
- **B+-tree underflow/merge policy on delete** — chunk 2's tree
  deliberately never rebalances or merges underfull nodes
  ([0016](decisions/0016-btree-node-format.md)), so heavy delete
  traffic leaves the tree sparser over time with no fix yet. Whether to
  add merging later, fold it into chunk 4's reclamation work, or accept
  permanent sparseness isn't decided.
- **Oversized key/value handling** — chunk 2's tree has no overflow
  pages; an entry too large for one page is a reported error
  ([0016](decisions/0016-btree-node-format.md)). Whether the row store
  (chunk 5+) needs an overflow-chain mechanism for large values, or can
  get away with a hard size ceiling, isn't decided.
- **Checkpoint-accelerated redo start** — chunk 3's recovery always
  scans the WAL from the start ([0017](decisions/0017-wal-format.md));
  using the latest checkpoint's dirty-page table to start later needs a
  durable pointer to that checkpoint's own LSN somewhere recovery can
  find without itself scanning the whole log. Not designed yet.
- **Automatic checkpoint triggering** — chunk 3 only checkpoints on an
  explicit call or clean close, no background size/timeout trigger
  ([0017](decisions/0017-wal-format.md)), since stage 1 has no
  background-task mechanism yet. Revisit once one exists.
- **WAL diff compression quality** — chunk 3's diff algorithm is a
  single common-prefix/common-suffix trim, which over-logs routine
  inserts that touch disjoint regions of a node
  ([0017](decisions/0017-wal-format.md)). The record format supports
  multiple ranges for a smarter algorithm later; not designed yet.
- **Publish-time crate/package naming** — `basalt`, `basalt-cli`, and
  `basalt-tui` are already taken on crates.io (an unrelated Vulkan UI
  framework and its sub-crates); `basalt-proto`, `basalt-server`,
  `basalt-client-core`, and `basalt-web-ui` are free as of this writing.
  The one crate that exists so far was renamed `basalt-engine` → `engine`
  to sidestep the collision risk on that one ([0014](decisions/0014-crate-layout.md)
  update note); whether to drop the `basalt-*` prefix from the rest too,
  pick a different prefix, or accept `basalt` internally and publish
  externally under another name isn't decided, and doesn't block
  implementation.

## Deferred, Not Yet Scheduled

Raised as possibilities but with no committed timeline — not ruled out,
just not planned:

- **Postgres wire-protocol compatibility** — an optional possibility,
  not a committed direction ([0007](decisions/0007-wire-protocol.md):
  native protocol is the actual target; Postgres compatibility may be
  revisited later but isn't assumed).
- MySQL wire protocol.
- Standardized ODBC / JDBC / ADO.NET *provider* layers (e.g. a
  `DbConnection`-based ADO.NET provider, not just a plain native .NET
  client library) — no longer assumed to arrive "for free" via Postgres
  compatibility, since that's now optional rather than committed
  ([0007](decisions/0007-wire-protocol.md)). Distinct from the native
  Rust/C/.NET client libraries already committed to in
  [0007](decisions/0007-wire-protocol.md).
- Native client libraries for languages beyond the initial Rust/C/.NET
  set (Python, Node, Go, Java, etc.) — unscheduled
  ([0007](decisions/0007-wire-protocol.md)).
- A general REST/HTTP + JSON data API (distinct from the narrow HTTP
  endpoint that serves the bundled web console — see
  [0006](decisions/0006-admin-dev-client.md) /
  [0007](decisions/0007-wire-protocol.md)).
- A standard replication/CDC hook (logical-decoding-style).
- OS support beyond the committed FreeBSD tier — DragonflyBSD, OpenBSD,
  NetBSD, illumos, macOS ("maybe sometime," see
  [0003](decisions/0003-supported-os.md)).
