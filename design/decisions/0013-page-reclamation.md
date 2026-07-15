# ADR 0013: Page/Version Reclamation

Status: Proposed (2026-07-15)

## Context

[0008](0008-concurrency-durability.md)'s copy-on-write B+-tree never
mutates a live page — a write allocates new pages along the path to the
root, and old pages stay around as long as some reader's snapshot still
reaches them. Once no reader does, those pages are garbage and need
reclaiming, but no scheme for that was designed there (flagged in
[backlog.md](../backlog.md)).

Two structurally different ways to track "still reachable" were
considered: **per-page reference counting** (count, for every page, how
many live snapshots reach it; reclaim at zero) and a **low-water-mark**
over reader snapshot generations (LMDB-style: track only the oldest
active reader's generation; anything superseded before that generation
is reachable by no one). Per-page refcounting is the general solution,
needed when a page can be reachable from a set of snapshots that isn't a
contiguous range — e.g. once concurrent writers ([roadmap.md](../roadmap.md)
stage 4) make the root history branch rather than stay linear. Stage 1
doesn't have that problem: with a single writer, root history is a
strict linear sequence of generations, so a page allocated at commit *G*
and superseded at commit *G′* is reachable from exactly the contiguous
range of generations *[G, G′)* — no separate count is needed, only
whether the oldest live reader's generation has passed *G′*. The
low-water-mark approach was chosen for stage 1 on that basis: it gives
the same answer as refcounting here, for less machinery.

Tracking the low-water mark requires readers to publish which generation
they're pinned to. A **reader table** (in-memory, per-process) serves
that: each open read transaction registers its snapshot generation on
start and deregisters on close; the minimum across open entries is the
low-water mark. Making it durable/crash-resilient was considered and
rejected — readers don't survive a process crash anyway (stage 1 is a
single embedded process, [0001](0001-scope.md)), and losing reclamation
progress across a crash is harmless: pages that were live at crash time
simply don't get reclaimed until the first post-recovery pass notices
they're superseded, which is a delay, not a correctness problem.

A known failure mode of the low-water-mark approach (shared with LMDB,
which uses the same technique) is **starvation**: one long-running
reader pins the low-water mark indefinitely, so no page superseded after
it started can ever be reclaimed, regardless of how many newer readers
come and go. This was weighed against the added complexity of a mitigation
(e.g. reader timeouts, forcibly invalidating stale snapshots) and
accepted as a stage-1 tradeoff — the same one LMDB makes.

## Decision

- **Reclamation scheme**: low-water-mark over reader snapshot
  generations, not per-page reference counting. A page superseded at
  commit *G′* is reclaimable once the oldest active reader's generation
  is ≥ *G′*.
- **Reader table**: in-memory only, one process-local table. Each open
  read transaction registers its snapshot generation when it starts and
  removes its entry when it ends. Not persisted, not crash-resilient.
- **Reclaim trigger**: piggybacks on the fuzzy-checkpoint background
  pass already introduced by [0008](0008-concurrency-durability.md),
  rather than being a separate mechanism — the checkpoint pass computes
  the current low-water mark and frees any page superseded before it.
- **Starvation**: accepted as-is for stage 1. A long-running reader can
  delay reclamation indefinitely; no mitigation (timeout, forced
  invalidation) is planned at this stage.

## Consequences

- No per-page refcount, no vacuum-style sweep — reclamation is a single
  minimum computed over an in-memory reader table, reusing the
  checkpoint pass's cadence rather than adding a new background job.
- Reclamation state is not crash-durable: pages live at crash time are
  reclaimed later than they strictly could be, once the next checkpoint
  after recovery notices they're superseded. Not a correctness issue,
  since redo from the WAL doesn't depend on which pages were already
  freed.
- A single long-running reader can starve reclamation for as long as it
  stays open, growing the data file unboundedly in the worst case. No
  mitigation is designed; revisiting this is deferred, not ruled out.
- This scheme is specific to stage 1's linear (single-writer) root
  history. Stage 4's concurrent writers may make the root history branch
  ([backlog.md](../backlog.md)), which would break the "contiguous
  range" assumption the low-water mark relies on — reclamation may need
  to move to per-page refcounting (or another scheme) at that point.
  Not decided here.
