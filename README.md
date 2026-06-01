# lmml

lmml is a Rust terminal UI for managing llama.cpp locally: detect hardware,
build llama.cpp, manage GGUF models, and run the inference server.

Current v0.1.0 release scope is the tested Linux x86_64 LAN install flow. Other
target tarballs should be built and validated on matching builders before they
are advertised as release-ready.

This is a local/LAN release scope, not a broad production-ready claim. GPU
acceleration is the primary path; intentional CPU-only nodes are supported by
explicit opt-in during preflight/install.

## Install

### One-line install (Linux / macOS)

```sh
curl -fsSL https://your-lan-or-github/install.sh | sh
```

### LAN install

If you are serving lmml on a local network:

```sh
curl -fsSL http://192.168.1.100:8000/install.sh | BASE_URL=http://192.168.1.100:8000 sh
```

Serve the packaged `dist/` directory from the release host:

```sh
cd dist && python3 -m http.server 8000
```

The LAN HTTP flow verifies `SHA256SUMS` to catch corrupt or incomplete
downloads. It is not tamper-proof: anyone who can alter the HTTP response can
alter both the tarball and checksum file. Treat it as an integrity check for a
trusted LAN release host unless you require signed checksum verification.

For a future public or non-local signed release, publish `SHA256SUMS.minisig`
from `scripts/package-release.sh` and require minisign verification during
install:

```sh
curl -fsSL https://release.example/install.sh | LMML_CHECKSUM_VERIFY=required LMML_MINISIGN_PUBLIC_KEY='RW...' sh
```

### Preflight and source-build bootstrap

The default install path above uses the verified binary tarball. For a
source-build LAN/dev bootstrap, run preflight first and then opt into source
mode explicitly:

```sh
curl -fsSL http://192.168.1.100:8000/preflight.sh | LMML_INSTALL_MODE=source bash
curl -fsSL http://192.168.1.100:8000/install.sh | BASE_URL=http://192.168.1.100:8000 INSTALL_MODE=source bash
```

GPU acceleration is primary and first-class in preflight. Intentional CPU-only
nodes must opt in explicitly:

```sh
curl -fsSL http://192.168.1.100:8000/preflight.sh | LMML_INSTALL_MODE=source LMML_GPU_MODE=cpu-only bash
curl -fsSL http://192.168.1.100:8000/install.sh | BASE_URL=http://192.168.1.100:8000 INSTALL_MODE=source LMML_GPU_MODE=cpu-only bash
```

Narrow apt fixes for compiler/CMake/Git/curl are opt-in:

```sh
curl -fsSL http://192.168.1.100:8000/preflight.sh | LMML_INSTALL_MODE=source LMML_FIX_DEPS=1 bash
```

### After install

```sh
lmml doctor    # check your system
lmml           # launch the TUI
```

The installer runs `lmml doctor` before reporting success. Missing hard
prerequisites such as a compiler, CMake, Git, or required disk space cause the
install command to fail clearly even though the binary has already been copied.
Fix the reported prerequisites and rerun `lmml doctor`.

### Uninstall

```sh
curl -fsSL https://your-lan-or-github/uninstall.sh | sh
```

Or, after installing:

```sh
lmml-uninstall
```

## Build From Source

```sh
cargo build --release -p lmml-tui
./target/release/lmml doctor
```
