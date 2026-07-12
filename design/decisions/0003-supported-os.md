# ADR 0003: Supported Operating Systems

Status: Proposed (2026-07-10), revised (2026-07-11)

## Context

Rust ([0002](0002-implementation-platform.md)) makes cross-platform
support cheap in principle, so this decision is really about which
platforms are worth the CI/testing burden for a solo project
([0001](0001-scope.md)), not about language capability.

Research into the options:

- **Dev machine is Windows** — the primary environment this gets built
  and tested on day to day, hence day-one priority.
- **Linux is the realistic production target** for a client/server DB
  (matches Postgres/MySQL/SQL Server precedent), and is cheap to test
  locally via WSL or a Linux VM — no cloud infrastructure required. Still
  staged after Windows rather than day one, to keep initial platform
  surface to one target while the core is taking shape.
- **FreeBSD, staged after Linux**, specifically for **ZFS** — checksummed
  copy-on-write storage with native snapshots is directly relevant to a
  database engine's durability/backup story. Worth a dedicated target
  once Windows/Linux support is established, rather than day one.
- **Other BSDs and illumos are "maybe sometime," not committed.**
  illumos (OmniOS/SmartOS) shares FreeBSD's ZFS motivation and adds
  DTrace, arguably unmatched for profiling a storage engine in
  production — but has a much smaller community/ecosystem today.
  DragonflyBSD, OpenBSD, and NetBSD have no specific driver (no unique
  filesystem/tracing story pulling them in) and are listed only as
  possibilities, not plans.
- **macOS was considered and moved to the same "maybe sometime" tier as
  the other BSDs/illumos**, rather than being explicitly excluded.
  Precedent from other engines is mixed and instructive:
  Postgres/MySQL/SQLite/DuckDB support
  macOS natively as a *developer workstation* platform, but essentially
  nobody deploys production database servers on macOS — Apple doesn't
  sell server hardware anymore, and even IRIS treats macOS as dev-only.
  SQL Server doesn't support macOS at all (Mac users run it via a Linux
  container). Testing on macOS would also require either a physical Mac
  or a cloud Mac (GitHub Actions macOS runners at a 10x per-minute cost
  vs. Linux, or a dedicated rental like MacStadium/Scaleway) — real
  recurring cost/friction for a hobby project, for a platform that
  wouldn't be a production target anyway.
- Client tooling was later split from a flat "matches the server" rule
  once the client took shape as three distinct interface forms
  ([0006](0006-admin-dev-client.md)): CLI REPL and TUI are cheap to port
  (no GUI toolkit dependency) so they can reasonably chase every
  platform the server supports, while GUI portability is a real,
  separate cost that shouldn't gate on niche server platforms — hence
  GUI's narrower Windows+Linux requirement with best-effort elsewhere.

## Decision

**Server** OS support is staged by priority rather than a flat list:

1. **Windows — day one.** Primary development and first-supported
   platform.
2. **Linux — next.** Added once Windows support is solid; still the
   realistic production-deployment target for a client/server DB.
3. **FreeBSD — much later.** Motivated specifically by ZFS.
4. **Maybe/sometime, no concrete commitment:** other BSDs (DragonflyBSD,
   OpenBSD, NetBSD), illumos (OmniOS/SmartOS — the original home of ZFS
   and DTrace, which FreeBSD later ported), and macOS. None of these are
   ruled out, but nothing beyond FreeBSD is currently planned for them;
   they'd only get picked up if a concrete reason shows up.

**Client tooling** ([0006](0006-admin-dev-client.md): CLI REPL, TUI,
GUI) no longer tracks the server list uniformly — it now splits by
interface form:

- **CLI REPL and TUI strive to match the server's list exactly**,
  platform for platform, at whatever stage the server reaches it:
  Windows + Linux, then FreeBSD, then the maybe-sometime tier if the
  server ever picks one of those up. These are lightweight enough
  (no rendering toolkit) that there's little reason for them to lag
  a platform the server already supports.
- **GUI is required only on Windows + Linux.** Support for FreeBSD or
  anything in the maybe-sometime tier is best-effort/if-possible, not a
  commitment — GUI toolkit portability to less-common platforms can be
  genuinely harder and isn't worth blocking on.

## Consequences

- CI/testing rolls out in the same priority order as support: Windows
  first, Linux added once Windows is solid, FreeBSD added later still.
  Linux testing can run locally (WSL) as well as in CI, keeping cost near
  zero once it's added.
- CLI REPL and TUI CI should track the server's platform matrix 1:1 —
  whenever a new server platform is added, add CLI/TUI CI for it too,
  since there's no separate reason for them to lag.
- GUI CI only needs to cover Windows + Linux as a hard requirement.
  Building/testing the GUI on FreeBSD or the maybe-sometime tier is
  opportunistic — nice if the toolkit happens to support it cheaply, but
  not blocking and not scheduled.
- DragonflyBSD, OpenBSD, NetBSD, illumos, and macOS require no CI/testing
  investment for now — they stay in the "maybe sometime" tier until a
  concrete reason promotes one of them (e.g. someone wants to run a
  client on a Mac, or the ZFS/DTrace story on illumos becomes compelling
  enough to prioritize over other work).
- No Apple Silicon/Intel split to worry about yet since macOS isn't an
  active target; Windows/Linux/FreeBSD are all effectively x86_64 first,
  with ARM64 support (e.g. Windows on ARM, Linux/FreeBSD on ARM servers)
  a possible later addition rather than a day-one requirement.

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
