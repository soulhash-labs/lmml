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

The installer must verify SHA256 checksums, install `lmml`, run `lmml doctor`,
and print the success summary.

## ROCm Scope

ROCm/HIP remains a documented v2 production gap. Do not claim AMD GPU
acceleration is production-ready until the ROCm probe, build flags, telemetry,
settings wiring, and tests are implemented.
