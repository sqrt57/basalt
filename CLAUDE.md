# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project status

Basalt (code name) is a database engine, in the design stage overall but
now mid-implementation of [roadmap.md](design/roadmap.md) stage 1 (the
embedded core) per [stage1-plan.md](design/stage1-plan.md)'s chunk list
— chunks 1 (page storage), 2 (copy-on-write B+-tree), and 3 (WAL + redo
recovery) are done, in the `engine` crate. See
[design/README.md](design/README.md) for the design
docs index (proposed architecture, open questions, build order, and the
ADRs); it's the entry point for design questions, not this file.

Store AI-generated design docs under `design/`.

A Cargo workspace at the repo root, one member so far: `crates/engine`
(edition 2024). Standard commands from the repo root:

- `cargo build` / `cargo check`
- `cargo test`
- `cargo clippy --all-targets`

## Working on this repo right now

Design work precedes code, not the other way round. Before writing any
code:

1. Check `design/architecture.md` for whether the relevant area (scope,
   platform, architecture, SQL dialect, wire protocol, client tooling)
   is already proposed, and `design/backlog.md` for whether a specific
   sub-question within it is still open.
2. Record new design decisions as new ADRs under `design/decisions/`,
   then update to match: `design/architecture.md` (proposed-state
   summary), `design/backlog.md` (remove the question if it's now
   resolved, or add new sub-questions the decision raised),
   `design/decisions/README.md` (add the new ADR to the index), and
   `design/roadmap.md` if the decision changes build order/sequencing.

When implementing a `design/stage1-plan.md` chunk: if the chunk's
on-disk/in-memory format isn't already decided, write the ADR and add
inline acceptance criteria to the chunk's entry in `stage1-plan.md`
first (see chunks 1 and 2 for the pattern), sync the docs per step 2
above, then implement and test. Commit as two separate commits — the
decision/docs first, the implementation second — rather than one
combined commit.
