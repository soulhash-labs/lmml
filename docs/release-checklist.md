# lmml Release Checklist

Before tagging the local release, this checklist must pass on this Ubuntu 24.04
x86_64 CUDA machine.

Passing this checklist supports a narrow local v0.1.0 claim: LAN install works
on the tested host/target. Do not broaden that into “all platforms
production-ready” until each target tarball is built and validated on a matching
machine or CI runner.

For local-only v0.1.0, a real minisign release keypair is not required. The
signed-checksum installer and packaging hooks must keep passing fixture tests so
the future public-release path remains wired.

## Required Gates

```sh
cargo fmt --all -- --check
cargo clippy --workspace -- -D warnings
cargo test --workspace
tests/integration/script_syntax.sh
tests/integration/preflight.sh
tests/integration/signed_checksums.sh
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

Optional source-build bootstrap from LAN must use the checksummed source tarball
from `dist/`:

```sh
curl -fsSL http://192.168.1.100:8000/preflight.sh | LMML_INSTALL_MODE=source bash
curl -fsSL http://192.168.1.100:8000/install.sh | BASE_URL=http://192.168.1.100:8000 INSTALL_MODE=source bash
```

Intentional CPU-only nodes must say so explicitly:

```sh
curl -fsSL http://192.168.1.100:8000/preflight.sh | LMML_INSTALL_MODE=source LMML_GPU_MODE=cpu-only bash
curl -fsSL http://192.168.1.100:8000/install.sh | BASE_URL=http://192.168.1.100:8000 INSTALL_MODE=source LMML_GPU_MODE=cpu-only bash
```

The installer must verify SHA256 checksums, install `lmml` and `lmml-uninstall`,
run `lmml doctor`, and print the success summary only when doctor passes. If
doctor reports missing hard prerequisites, the installer must exit non-zero with
the prerequisite error visible.

The LAN HTTP checksum is an integrity check, not authenticity. It detects
corrupt or incomplete downloads from a trusted release host, but it does not
protect against a host or network attacker who can replace both the tarball and
`SHA256SUMS`.

Signed checksum verification is supported with minisign. Local/LAN v0.1.0 does
not require a real release signing keypair. Future public releases should
publish `SHA256SUMS.minisig` and require signature verification:

```sh
LMML_SIGN_CHECKSUMS=1 LMML_MINISIGN_SECRET_KEY_FILE=/secure/lmml-minisign.key scripts/package-release.sh
curl -fsSL https://release.example/install.sh | LMML_CHECKSUM_VERIFY=required LMML_MINISIGN_PUBLIC_KEY='RW...' sh
```

For LAN testing, the installer defaults to `LMML_CHECKSUM_VERIFY=optional`.
That mode verifies `SHA256SUMS.minisig` when a signature and public key are
configured, otherwise it warns and falls back to SHA256 integrity only.

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

## Cross-Target Validation

Do not publish or advertise a target tarball until it has been built and
validated on a matching builder or CI runner.

Required targets before broader platform claims:

- `aarch64-unknown-linux-gnu`
- `x86_64-apple-darwin`
- `aarch64-apple-darwin`

For each target:

```sh
TARGET_TRIPLE=<target> scripts/package-release.sh
tar -tzf dist/lmml-0.1.0-<target>.tar.gz
```

Then install on matching hardware/OS and run:

```sh
lmml doctor
lmml smoke
```

Record the target triple, builder/runner, OS version, command results, and any
runtime dependency notes before marking the target release-ready.

## This-Machine CUDA Validation

Before making broader GPU readiness claims for the local release target,
validate on this Ubuntu 24.04 x86_64 machine with NVIDIA driver and CUDA toolkit
installed. A separate VM is not required for local v0.1.0 validation.

Host prechecks:

```sh
nvidia-smi
nvcc --version
rustc --version
cargo --version
rustup show active-toolchain
```

Release install checks:

```sh
curl -fsSL http://<release-host>:8000/install.sh | BASE_URL=http://<release-host>:8000 sh
lmml doctor
lmml smoke
lmml-uninstall

curl -fsSL http://<release-host>:8000/preflight.sh | LMML_INSTALL_MODE=source bash
curl -fsSL http://<release-host>:8000/install.sh | BASE_URL=http://<release-host>:8000 INSTALL_MODE=source bash
lmml doctor
lmml smoke
lmml-uninstall
```

Both install modes must pass without `LMML_GPU_MODE=cpu-only`, and
`lmml doctor` must report CUDA available with GPU name and compute capability.
