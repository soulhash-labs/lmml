# lmml — Final Packaging & Distribution Prompt for Claude Code

Read `CLAUDE.md` and `docs/lmml-plan.md` in full before touching any code.

The runtime is integration-tested and wired. The only remaining gap between
"developer build" and "LAN curl install and 100% running" is packaging,
distribution, and clean-machine validation. Work through every item below in
strict order. Do not move to the next item until the current one is complete,
tested, and committed.

---

## 1. Cut a clean release commit first

Before touching packaging:

- Run `cargo fmt --all -- --check` — must pass.
- Run `cargo clippy --workspace -- -D warnings` — must pass.
- Run `cargo test --workspace` — must pass.
- Commit all uncommitted changes with the message:
  `chore: pre-release cleanup — all checks pass`

This is the baseline the installer will be built from.

---

## 2. Build a release binary and verify it is self-contained

```sh
cargo build --release -p lmml-tui
```

The output binary is `target/release/lmml`.

Verify it is self-contained (no missing shared libraries the target machine
might not have):

```sh
# Linux
ldd target/release/lmml

# macOS
otool -L target/release/lmml
```

If the binary links against anything beyond libc, libm, libdl, libpthread,
and optionally libcuda/libcudart (expected), document it. If it links against
something unexpected, fix the build to statically link or vendor it.

---

## 3. Create the release packaging script

Create `scripts/package-release.sh`:

```
scripts/
  package-release.sh      # builds + packages a versioned tarball
  install.sh              # the file users download and run
  uninstall.sh            # clean removal
```

### `scripts/package-release.sh`

This script:
1. Reads the version from `crates/lmml-tui/Cargo.toml` (the `version` field).
2. Builds `--release` for the current platform.
3. Creates a versioned tarball:
   `lmml-<version>-<target-triple>.tar.gz`
   containing:
   - `lmml` binary
   - `README.md`
   - `LICENSE` (create one if absent — MIT is fine)
   - `scripts/install.sh`
   - `scripts/uninstall.sh`
4. Writes a `SHA256SUMS` file alongside the tarball.
5. Prints the tarball path and checksum.

Target triples to support:
- `x86_64-unknown-linux-gnu`
- `aarch64-unknown-linux-gnu`
- `x86_64-apple-darwin`
- `aarch64-apple-darwin`

---

## 4. Write `scripts/install.sh` — the LAN curl installer

This is the file a user on a LAN machine runs with:

```sh
curl -fsSL http://<lan-host>/lmml/install.sh | sh
```

or downloads and runs directly:

```sh
sh install.sh
```

The script must do the following in order, with no silent failures:

### 4a. Detect the OS and architecture

Support:
- Linux x86_64
- Linux aarch64
- macOS x86_64
- macOS arm64 (Apple Silicon)

Exit with a clear error on anything else:
```
✗ Unsupported platform: <uname output>
  lmml supports Linux x86_64/aarch64 and macOS x86_64/arm64.
```

### 4b. Check hard prerequisites before installing

For each of the following, check if it is present and meets the minimum
version. If any hard prerequisite is missing, print the install command
and exit — do not proceed:

| Tool | Minimum | Install hint |
|---|---|---|
| `gcc` or `clang` | any | `apt install build-essential` / `xcode-select --install` |
| `cmake` | 3.21 | `apt install cmake` / `brew install cmake` |
| `git` | 2.28 | `apt install git` / `brew install git` |

Optional (warn but continue if missing):
- `nvcc` (CUDA) — warn: "CUDA not found; will build CPU-only"
- `nvidia-smi` — warn: "GPU not detected; will build CPU-only"

### 4c. Download the correct tarball

If run from a LAN server (BASE_URL environment variable is set):
```sh
BASE_URL=${BASE_URL:-https://github.com/YOUR_ORG/lmml/releases/latest}
```

Download:
- `lmml-<version>-<target-triple>.tar.gz`
- `SHA256SUMS`

Verify the SHA256 checksum before extracting. If the checksum fails, delete
the downloaded file and exit with:
```
✗ Checksum verification failed. The download may be corrupt.
  Try again or download manually from: <url>
```

### 4d. Install the binary

Default install location: `$HOME/.local/bin/lmml`

If the user runs with `sudo` or sets `PREFIX=/usr/local`:
install to `$PREFIX/bin/lmml`.

After installing, check if the install location is on `$PATH`. If not, print:
```
⚠  $HOME/.local/bin is not on your PATH.
   Add this to your shell profile:
     export PATH="$HOME/.local/bin:$PATH"
   Then restart your terminal or run: source ~/.bashrc
```

### 4e. Run `lmml doctor` as the final step

After install, run the newly installed binary in doctor mode (see item 5).
Print the output. If doctor exits non-zero, print:
```
⚠  lmml installed but preflight checks found issues.
   Run `lmml doctor` to see what needs fixing before first use.
```
and exit 0 (installation succeeded; issues are informational).

### 4f. Print a success summary

```
✓ lmml <version> installed to ~/.local/bin/lmml

  Get started:
    lmml doctor       — check your system
    lmml              — launch the TUI
```

---

## 5. Implement `lmml doctor`

Add a `doctor` subcommand to the `lmml-tui` binary (use `clap` for argument
parsing if not already present).

`lmml doctor` runs outside the TUI, prints to stdout, and exits 0 if all
hard prerequisites pass or 1 if any hard prerequisite fails.

Output format:

```
lmml doctor — system preflight check
─────────────────────────────────────
  ✓  gcc 15.2.0
  ✓  cmake 4.2.3
  ✓  git 2.53.0
  ✓  CUDA 12.4  ·  RTX 3090  ·  sm_86
  ✓  disk: 44 GB available
  ✗  nvcc not found
     → sudo apt install nvidia-cuda-toolkit

  1 issue found. Run `lmml` to proceed in CPU-only mode,
  or fix the issues above for GPU acceleration.
```

`lmml doctor` must reuse `lmml-detect::SystemProfile::detect()` exactly —
no duplicated probe logic.

---

## 6. Write `scripts/uninstall.sh`

Simple and safe:

1. Confirm the user wants to uninstall (prompt `y/N`).
2. Remove `$HOME/.local/bin/lmml` (or `$PREFIX/bin/lmml`).
3. Print the config and data paths and ask if the user wants to delete them:
   - `~/.config/lmml/`
   - `~/.local/share/lmml/`
4. Delete only what the user confirms.
5. Print:
   ```
   ✓ lmml uninstalled.
   ```

---

## 7. Set up a LAN-serveable directory structure

Create `dist/` (gitignored) with the structure a LAN HTTP server serves:

```
dist/
  install.sh                              # symlink or copy of scripts/install.sh
  lmml-<version>-x86_64-linux.tar.gz
  lmml-<version>-aarch64-linux.tar.gz
  lmml-<version>-x86_64-macos.tar.gz
  lmml-<version>-aarch64-macos.tar.gz
  SHA256SUMS
  latest                                  # plain text file: just the version string
```

`install.sh` reads `latest` to know which version to download when no version
is pinned.

Document in `README.md` how to serve this over a LAN:

```sh
# Serve the dist/ directory over LAN (port 8000)
cd dist && python3 -m http.server 8000

# Install from LAN on another machine
BASE_URL=http://192.168.1.100:8000 curl -fsSL http://192.168.1.100:8000/install.sh | sh
```

---

## 8. Add a `Makefile` (or `justfile`) for release workflow

```makefile
release:          ## Build release binary + package tarball
    scripts/package-release.sh

dist-serve:       ## Serve dist/ over LAN for testing
    cd dist && python3 -m http.server 8000

doctor:           ## Run lmml doctor against the local build
    cargo run --release -p lmml-tui -- doctor

clean-release:    ## Remove target/release and dist/
    rm -rf target/release dist/
```

---

## 9. Add a clean-machine smoke test

Create `tests/integration/clean_install.sh`:

This script simulates a fresh install and is run manually (not in CI) against
a real or VM machine. It must:

1. Check it is running in a clean environment (no existing `~/.config/lmml/`).
2. Run `scripts/install.sh` with `BASE_URL` pointing to a local `dist/` server.
3. Run `lmml doctor` and assert exit 0 (or document which warnings are expected).
4. Run `lmml` in headless/smoke mode for 5 seconds and assert it exits cleanly.
5. Run `scripts/uninstall.sh` with auto-confirm and assert the binary is removed.

Document in `docs/release-checklist.md` that this script must pass on a clean
Ubuntu 24.04 x86_64 VM with CUDA drivers installed before any release is tagged.

---

## 10. Update `README.md` with the install section

Replace any existing "how to build" instructions with the real install path:

```markdown
## Install

### One-line install (Linux / macOS)

```sh
curl -fsSL https://your-lan-or-github/install.sh | sh
```

### LAN install

If you are serving lmml on a local network:

```sh
BASE_URL=http://192.168.1.100:8000 curl -fsSL http://192.168.1.100:8000/install.sh | sh
```

### After install

```sh
lmml doctor    # check your system
lmml           # launch the TUI
```

### Uninstall

```sh
curl -fsSL https://your-lan-or-github/uninstall.sh | sh
# or if already installed:
lmml-uninstall
```
```

---

## 11. Final verification gate

Before this work is considered done, the following must all pass:

```sh
# Code quality
cargo fmt --all -- --check
cargo clippy --workspace -- -D warnings
cargo test --workspace

# Release build
cargo build --release -p lmml-tui
ldd target/release/lmml   # no unexpected shared libs

# Packaging
scripts/package-release.sh
ls dist/                   # tarballs + SHA256SUMS present

# Doctor works
./target/release/lmml doctor

# LAN serve + install simulation (on same machine for CI purposes)
cd dist && python3 -m http.server 8000 &
BASE_URL=http://127.0.0.1:8000 sh scripts/install.sh
~/.local/bin/lmml doctor
```

All of the above must succeed with no errors before committing. The final
commit message should be:

```
release: v<version> — LAN install ready, lmml doctor, clean-machine tested
```

---

## What NOT to do

- Do not use `pip` or `pyproject.toml`. lmml is a Rust binary. pip is the
  wrong distribution mechanism. The curl installer is the correct path.
- Do not add ROCm support in this pass. It is documented as a v2 gap.
  Do not claim it is implemented.
- Do not add new TUI features. This pass is packaging only.
- Do not skip the SHA256 verification step. It is not optional.
- Do not hardcode version strings. Always read from `Cargo.toml`.
