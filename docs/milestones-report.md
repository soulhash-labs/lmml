# lmml v2 Milestones Report

Date: 2026-06-01

Scope: This report compares `docs/lmml-plan.md` against the current repository state after milestones 1-13. It includes what is implemented, what is partial or stubbed, and whether the runtime is production ready.

## Executive Verdict

lmml is not production ready yet.

The v2 architecture has been mostly scaffolded and many core library paths are real code, not pure placeholders: detection, compat flag assembly, build streaming, state round-tripping, GGUF parsing, HF search/download, server lifecycle, settings editing, logging, snapshots, and CI all exist and pass tests.

However, the runtime is still an integration-stage MVP. The code compiles and the unit/snapshot test suite is healthy, but several workflows are incomplete at the TUI/action layer, some planned persistence behavior is missing, and the full real-world chain has not been proven end-to-end against actual `llama.cpp`, real model downloads, and a real `llama-server`.

The most important structural concern: the workspace still contains a legacy root binary package (`lmml` under `src/`) alongside the new v2 `crates/lmml-tui` binary. Running `cargo run` targets the root package by default, not necessarily the v2 architecture. The plan says `lmml-tui` is the only binary crate; the repository does not yet match that.

## Current Verification Status

Last verified locally:

- `cargo fmt --all -- --check` passes.
- `cargo test --workspace` passes.
- `cargo clippy --workspace -- -D warnings` passes.
- TUI snapshot tests exist and pass with 16 committed snapshots.
- CI exists at `.github/workflows/ci.yml`.

This proves the current code is internally consistent. It does not prove production readiness.

## Milestone-by-Milestone Status

| # | Plan Expected | Current Status | Critic's View |
|---|---|---|---|
| 1 | `lmml-detect` with full CUDA arch matrix, C++17 probe, disk check, unit tests | Mostly done | Strong library milestone. Full arch mapping, compatibility checks, C++17 probe, RAM/disk/tool checks, tests. Needs broader real-host validation. |
| 2 | `lmml-compat` with flag detection + argv assembly, unit tests | Mostly done | Good centralization of llama.cpp flag knowledge. Parser is still help-text heuristic based, so fixture coverage should expand with real upstream outputs. |
| 3 | `lmml-build` with sccache, fingerprint, update check, streaming | Partial | Build streaming, clone/pull, cmake args, sccache flags, verification exist. Fingerprint exists but is not fully used/persisted to decide rebuilds. Build cancellation is not real. Pin-to-tag/track-main settings are not fully integrated. |
| 4 | `lmml-state` with full schema, XDG paths, round-trip tests | Mostly done | Schema and tests exist. But app does not save state on every significant change or graceful exit. Build metadata updates are incomplete. |
| 5 | TUI skeleton: event loop, Action dispatch, tabs, footer, help overlay | Done | Skeleton exists and is testable. Event loop supports background tasks. Some actions remain only status-message stubs. |
| 6 | Detect tab: badges, CUDA arch list, install hints, first-run onboarding | Mostly done | Detect UI renders badges, cached state, hints, onboarding. First-run flow is still simple, not the full guided modal sequence from the plan. |
| 7 | Build tab: streaming log, sccache badge, update check, clean build | Partial | Build log and controls exist. Update check exists. Clean build exists. Actual cancellation, update-and-rebuild semantics, and persisted fingerprint/update state need more work. |
| 8 | `lmml-models`: GGUF parse, VRAM fit, registry scan + Models tab | Mostly done | Real GGUF header parse, scan, VRAM label, aliases in registry. Quant detection is basic. TUI alias management is not implemented. |
| 9 | HF search + resumable download + progress bar in Models tab | Partial to mostly done | HF API search and Range-based resume exist. TUI displays progress. Search input is not a proper editable UI; `/` searches the current/default query. Download test coverage is not an actual HTTP resume integration test. |
| 10 | `lmml-server`: health polling, port conflict, argv via compat + Server tab | Partial to mostly done | Real managed process path, port check, compat argv, health polling, logs, stop on quit. Missing stronger integration tests with server stubs. Runtime monitor is basic. |
| 11 | Settings tab: all fields editable, unsupported flag warnings | Mostly done | Modal editor, toggles, save, probe, warnings exist. Needs better field validation, UX polish, and robust extra-args parsing/quoting. |
| 12 | Observability: tracing spans, rolling log, panic hook | Mostly done | `tracing` added across major paths. Stable `lmml.log`, rolling logs, panic restore hook exist. Retention/log strategy should be tested in an integration-style way. |
| 13 | Full snapshot test suite + CI | Partial to mostly done | CI and 16 snapshots exist. This is not a "full" suite by the plan's matrix; it covers representative states but not every meaningful combination. |
| 14 | Model alias support (symlinks + external paths) | Not complete | Library-level alias support exists. TUI `AddModelAlias` is only a status message. No modal/path input, persistence update, or alias deletion UX. |

## Implemented Real Code

The following are real implementations, not just stubs:

- Hardware/toolchain detection with command runner abstraction.
- CUDA compute capability mapping through `sm_100a`.
- CUDA toolkit vs GPU architecture compatibility checks.
- C++17 compiler probe and disk space checks.
- llama.cpp capability probing and compatibility-based argv assembly.
- Build runner that clones/pulls, configures CMake, builds, streams output, verifies binaries.
- State schema with XDG config/data path handling and round-trip tests.
- TUI event loop with background task messages.
- Detect/Build/Models/Server/Settings tab rendering.
- GGUF metadata header parser.
- Local model registry scan and VRAM fit estimates.
- HF model search parsing and direct resolve URL creation.
- Resumable download path using `.part` files and HTTP `Range`.
- Server manager that owns the child process and checks readiness.
- Settings modal editor and unsupported flag warnings.
- Structured file logging and panic terminal restoration.
- Snapshot tests and CI.

## Stubs, Partial Actions, and Missing Runtime Wiring

These are the main areas where the UI or runtime still does not do what the plan implies:

| Area | Current Behavior | Needed Completion |
|---|---|---|
| Root binary / package layout | Workspace still has legacy root `src/` package plus v2 crates. | Decide the real product binary. Either make `lmml-tui` the shipped `lmml` binary or remove/merge legacy root code. |
| Add model alias | `Action::AddModelAlias` only changes status text. | Add modal/path input, call `ModelRegistry::add_alias`, persist aliases to `lmml-state`, rescan models. |
| Delete model | `Action::DeleteModel` only changes status text. | Add confirmation dialog, call `ModelRegistry::delete`, update list/state, test destructive flow. |
| Cancel build | `Action::CancelBuild` only logs a cancellation request. | Keep build task/process handles and terminate subprocesses safely. |
| Build fingerprint | Fingerprint helpers exist, but rebuild decisions are mainly binary-existence based and not persisted. | Persist commit/cmake hash/backend/arch/sccache/last_built and use them to skip or trigger rebuilds. |
| Update and rebuild | Event loop treats `UpdateAndRebuild` like a normal build path. Existing build code pulls on existing repos. | Implement explicit update flow, track-main vs tag mode, user-visible update status, and rebuild reason. |
| Save on exit | Settings save works, but graceful exit does not save all significant runtime changes. | Save app state on quit and after build/model/server significant events. |
| HF search input | `/` searches current/default query. No real query editor. | Add modal/search input, filters, result navigation polish. |
| Download integration test | Resume helpers are tested, but no real HTTP Range server integration test. | Add local mock HTTP server or test harness for 200/206/resume/error cases. |
| Server integration tests | Unit tests cover URL/readiness helper, not full child process lifecycle. | Add stub `llama-server` process tests: ready, timeout, crash, port conflict. |
| First-run onboarding | Basic modal exists. | Implement guided flow: detect, choose backend, model dir, starter model, server port. |
| Settings validation | Basic parse errors only. | Add ranges, port validation, host validation, extra args quoting, API key masking. |
| Snapshot coverage | 16 representative snapshots. | Add full matrix from plan: per tab × complete state set. |

## Production Readiness Assessment

### What is production-ish

- The Rust workspace is healthy under tests, clippy, and fmt.
- Core crate boundaries match the v2 design.
- The code is structured enough to continue milestone work without a rewrite.
- Several critical "works on my machine" mitigations are implemented in library form: C++17 probe, disk check, CUDA arch compatibility, sccache injection, health polling, and flag compatibility.

### What is not production ready

- The shipped binary story is confused by the legacy root package.
- The main user flows have not been proven end-to-end:
  - clean first launch,
  - detect,
  - build llama.cpp,
  - download a real GGUF,
  - start real llama-server,
  - quit and restore state.
- Several TUI actions still do not perform their named operation.
- State persistence is incomplete for runtime changes.
- Build cancellation and robust process cleanup are incomplete.
- Model alias and deletion UX are not complete.
- Tests are strong for units and snapshots, but weak for real subprocess/network integration.

Production-ready runtime status: **No.** It is a functional architecture implementation and testable MVP foundation, but it should be treated as pre-production until the gaps above are closed and a real E2E smoke test passes on a fresh machine.

## Critic's View

The work so far is valuable, but the milestone labels are optimistic. Several milestones are "implemented enough to compile and demonstrate architecture," not fully complete against the plan's user-facing standard.

The biggest risk is not one specific bug; it is that implementation depth varies by layer:

- Library crates are generally stronger.
- TUI rendering exists and is snapshot-tested.
- TUI action wiring is uneven.
- Cross-crate persistence and full workflow integration are underbuilt.

The plan describes a turnkey product. The current repo is closer to a well-structured alpha.

## Recommended Next Work

Priority order:

1. Resolve binary/package layout: make the v2 binary the actual `lmml` product.
2. Implement Milestone 14 fully: alias modal, persistence, scan integration.
3. Complete destructive/action flows: delete model, cancel build, clean build confirmation.
4. Complete persistence: save on quit and after build/model/server state changes.
5. Complete build fingerprint/update semantics.
6. Add integration smoke tests for server lifecycle and download resume.
7. Add an E2E "fresh install" script/test path using stubs for git/cmake/llama-server.
8. Expand snapshot matrix only after workflow behavior is fully wired.

## Bottom Line

Milestones 1-13 are committed and mostly aligned with the plan at the structural level. Milestone 14 is not complete. The codebase is in good shape for continued development, but calling the runtime production ready would be inaccurate. The next phase should focus less on adding new surface area and more on closing action wiring, persistence, integration tests, and the binary packaging mismatch.
