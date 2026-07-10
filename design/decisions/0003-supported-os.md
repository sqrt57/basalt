# ADR 0003: Supported Operating Systems

Status: Decided (2026-07-10)

## Decision

Basalt's **server** targets **Windows and Linux**. Client tooling
(CLI/admin/dev clients, and any third-party SQL client connecting over
the wire protocol) targets the **same two platforms** — no separate
client-only OS support beyond that for now.

macOS is explicitly not a target, for either server or client, at this
stage.

## Context

Rust ([0002](0002-implementation-platform.md)) makes cross-platform
support cheap in principle, so this decision is really about which
platforms are worth the CI/testing burden for a solo project
([0001](0001-scope.md)), not about language capability.

Research into the options:

- **Dev machine is Windows** — the primary environment this gets built
  and tested on day to day.
- **Linux is the realistic production target** for a client/server DB
  (matches Postgres/MySQL/SQL Server precedent), and is cheap to test
  locally via WSL or a Linux VM — no cloud infrastructure required.
- **macOS was considered and rejected for now.** Precedent from other
  engines is mixed and instructive: Postgres/MySQL/SQLite/DuckDB support
  macOS natively as a *developer workstation* platform, but essentially
  nobody deploys production database servers on macOS — Apple doesn't
  sell server hardware anymore, and even IRIS treats macOS as dev-only.
  SQL Server doesn't support macOS at all (Mac users run it via a Linux
  container). Testing on macOS would also require either a physical Mac
  or a cloud Mac (GitHub Actions macOS runners at a 10x per-minute cost
  vs. Linux, or a dedicated rental like MacStadium/Scaleway) — real
  recurring cost/friction for a hobby project, for a platform that
  wouldn't be a production target anyway.
- Client and server platforms were kept in sync rather than split,
  because there's no concrete need yet (no client exists) to justify the
  extra combination.

## Consequences

- CI/testing needs to cover Windows and Linux only. Linux testing can run
  locally (WSL) as well as in CI, keeping cost near zero.
- macOS can be revisited later if a concrete reason shows up (e.g.
  someone other than the primary developer wants to run a client on a
  Mac) — this is not a permanent exclusion, just not worth building for
  today.
- No Apple Silicon/Intel split to worry about since macOS is out of
  scope; Windows/Linux are both effectively x86_64 first, with ARM64
  support (e.g. Windows on ARM, Linux on ARM servers) a possible later
  addition rather than a day-one requirement.
