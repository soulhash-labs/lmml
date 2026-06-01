# lmml

lmml is a Rust terminal UI for managing llama.cpp locally: detect hardware,
build llama.cpp, manage GGUF models, and run the inference server.

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
trusted LAN release host until signed checksums or HTTPS public releases are in
place.

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
