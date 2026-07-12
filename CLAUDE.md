# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project status

Basalt (code name) is a database engine in the pre-implementation design
stage — no source code, build system, or test suite yet, just licensing
and design material. See [design/README.md](design/README.md) for the
design docs index (currently proposed architecture, open questions,
build order, and the ADRs); it's the entry point, not this file.

Store AI-generated design docs under `design/`.

There are no build/lint/test commands to document yet. When
implementation begins, this file should be updated with the actual
Cargo commands and the real crate/module architecture once code exists
— do not invent these before they exist.

## Working on this repo right now

Most work at this stage is design discussion, not code. Before writing
any code:

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
