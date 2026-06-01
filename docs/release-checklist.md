# lmml Release Checklist

Before tagging a release, this checklist must pass on a clean Ubuntu 24.04
x86_64 VM with CUDA drivers installed.

## Required Gates

```sh
cargo fmt --all -- --check
cargo clippy --workspace -- -D warnings
cargo test --workspace
cargo build --release -p lmml-tui
ldd target/release/lmml
scripts/package-release.sh
SOURCE_DATE_EPOCH=$(git log -1 --format=%ct) scripts/package-release.sh
./target/release/lmml doctor
tests/integration/clean_install.sh
```

## Dynamic Link Policy

The Linux release binary is built with rustls TLS to avoid OpenSSL runtime
dependencies. On the current Ubuntu build host, `ldd target/release/lmml` shows:

- `libgcc_s.so.1`
- `libm.so.6`
- `libc.so.6`
- `/lib64/ld-linux-x86-64.so.2`

`libgcc_s.so.1` is the GCC runtime unwind library provided by standard Linux
base systems. OpenSSL, zlib, and zstd must not appear in release `ldd` output.

## LAN Release Test

Build and serve the release:

```sh
scripts/package-release.sh
cd dist && python3 -m http.server 8000
```

Install from another machine on the LAN:

```sh
curl -fsSL http://192.168.1.100:8000/install.sh | BASE_URL=http://192.168.1.100:8000 sh
```

The installer must verify SHA256 checksums, install `lmml` and `lmml-uninstall`,
run `lmml doctor`, and print the success summary only when doctor passes. If
doctor reports missing hard prerequisites, the installer must exit non-zero with
the prerequisite error visible.

The LAN HTTP checksum is an integrity check, not authenticity. It detects
corrupt or incomplete downloads from a trusted release host, but it does not
protect against a host or network attacker who can replace both the tarball and
`SHA256SUMS`. Do not describe LAN HTTP installs as tamper-proof until signed
checksums or HTTPS-hosted releases are implemented.

## Reproducibility Check

`scripts/package-release.sh` requires GNU tar and writes archives with sorted
entries, numeric owner/group `0:0`, normalized file modes, fixed mtimes from
`SOURCE_DATE_EPOCH`, `gzip -n`, and `RELEASE-METADATA`.

Where feasible, run packaging twice with the same `SOURCE_DATE_EPOCH` and
confirm the tarball checksum is unchanged:

```sh
rm -rf dist target/package
SOURCE_DATE_EPOCH=$(git log -1 --format=%ct) scripts/package-release.sh
cp dist/SHA256SUMS /tmp/lmml-SHA256SUMS.first
rm -rf dist target/package
SOURCE_DATE_EPOCH=$(git log -1 --format=%ct) scripts/package-release.sh
diff -u /tmp/lmml-SHA256SUMS.first dist/SHA256SUMS
```

## ROCm Scope

ROCm/HIP remains a documented v2 production gap. Do not claim AMD GPU
acceleration is production-ready until the ROCm probe, build flags, telemetry,
settings wiring, and tests are implemented.
