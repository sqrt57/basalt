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

## Research notes (2026-07-10)

Detail behind the summary above, kept in case macOS is revisited later:

**Cloud Mac access, if ever needed (no physical Mac required):**
- **GitHub Actions macOS runners** — simplest option; billed at roughly
  a **10x per-minute multiplier vs. Linux** (Linux ~$0.006/min, macOS
  ~$0.062/min as of 2026). Fine for occasional/on-demand CI, not for
  running constantly.
- **MacStadium** — dedicated cloud Mac minis, ~$109/mo (M2, 8GB) to
  ~$119/mo (M4, 16GB); also offers an Orka platform for
  Kubernetes-orchestrated macOS VMs. Aimed at continuous CI use.
- **AWS EC2 Mac instances** — ~$1.10/hr (~$792/mo equivalent), but
  Apple's licensing terms force a **24-hour minimum allocation** per
  dedicated host — can't spin one up for a quick 10-minute test run.
- **MacinCloud** — pay-as-you-go remote-desktop access to a Mac; good
  for occasional interactive/manual testing, not CI.
- **Scaleway** — Mac mini bare-metal hosting, ~€99/mo, EU-based
  (relevant if GDPR/EU data residency ever mattered).

**macOS versioning / platform facts:**
- Apple discontinued the standalone **macOS Server app in 2022** — there
  is no separate "Server" SKU anymore, just macOS; any Mac can run
  server-type workloads.
- Apple switched to **year-based version numbering** in 2025. Current
  (mid-2026) release is **macOS 26 "Tahoe."** The next release, **macOS
  27 "Golden Gate"** (shipping September 2026), **drops Intel Mac
  support entirely** — Apple Silicon (ARM64) only from that point on.

**Per-database macOS support, as precedent:**

| DB | macOS support |
|---|---|
| PostgreSQL | First-class native builds (Homebrew, Postgres.app); commonly run locally by devs |
| MySQL / MariaDB | Native builds available, dev-focused |
| SQLite / DuckDB | Fully supported everywhere, including macOS |
| Redis | Supported for dev via Homebrew; Linux recommended for production |
| MongoDB | Community builds for macOS exist, dev-only |
| MS SQL Server | **Not available on macOS at all** — no native build; Mac users run it via a Linux Docker container |
| InterSystems IRIS | **Development platform only** — not supported for production on macOS |

The consistent pattern: macOS is near-universally supported as a
*developer workstation* platform, but essentially never used as a
*production server* target — informing the decision above to exclude it
from Basalt's server/client OS matrix for now.
