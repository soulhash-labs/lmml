#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use lmml_build::{BuildRunner, RealBuildRunner};
use lmml_compat::{LlamaBinaryCapabilities, ServerConfig};
use lmml_detect::{CpuFeatures, CudaCompatibility, DiskInfo, MemInfo, MetalSupport};
use lmml_models::{ModelEntry, ModelRegistry};
use lmml_server::ServerManager;
use lmml_tui::action::Action;
use lmml_tui::app::{App, AppEvent, ServerStatus};
use tokio::sync::mpsc;

static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

#[tokio::test]
async fn full_stubbed_runtime_chain_persists_state() {
    let _guard = ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .expect("env lock");
    let tempdir = tempfile::tempdir().expect("tempdir");
    let bin_dir = tempdir.path().join("bin");
    fs::create_dir_all(&bin_dir).expect("bin dir");
    write_git_stub(&bin_dir);
    write_cmake_stub(&bin_dir);
    let old_path = std::env::var_os("PATH").unwrap_or_default();
    let new_path = format!("{}:{}", bin_dir.display(), old_path.to_string_lossy());
    std::env::set_var("PATH", &new_path);

    let state_path = tempdir.path().join("state.toml");
    let models_dir = tempdir.path().join("models");
    fs::create_dir_all(&models_dir).expect("models dir");
    let model_path = models_dir.join("fixture-Q4_K_M.gguf");
    fs::write(&model_path, fixture_gguf()).expect("fixture gguf");

    let mut state = lmml_state::AppState::default();
    state.build.source_dir = tempdir.path().join("llama.cpp");
    state.model.models_dir = models_dir.clone();
    state.server.port = free_port();
    let mut app = App::new_with_state(state);
    app.state_save_path = Some(state_path.clone());

    app.handle_event(AppEvent::DetectComplete(Box::new(profile())));

    let config = app.build_config(false);
    let runner = RealBuildRunner;
    let mut build_rx = runner.run(config).await;
    while let Some(event) = build_rx.recv().await {
        let done = matches!(
            event,
            lmml_build::BuildEvent::Completed { .. }
                | lmml_build::BuildEvent::Failed { .. }
                | lmml_build::BuildEvent::Cancelled
                | lmml_build::BuildEvent::Skipped { .. }
        );
        app.handle_event(AppEvent::BuildEvent(event));
        if done {
            break;
        }
    }
    assert!(app.build_error.is_none());
    assert_eq!(app.state.build.commit, "stubcommit");
    assert!(!app.state.build.cmake_hash.is_empty());

    let registry = ModelRegistry {
        models_dir,
        aliases: Vec::new(),
    };
    app.handle_event(AppEvent::ModelScanComplete(registry.scan().await));
    assert_eq!(app.models.len(), 1);
    app.dispatch(Action::SelectModel(model_path.clone()));

    let server_model = app.selected_server_model().expect("selected model");
    let manager = ServerManager {
        binary: app.state.build.binary.clone(),
        caps: caps(),
    };
    let (log_tx, _log_rx) = mpsc::channel(32);
    let handle = manager
        .start_with_timeout(
            &server_model,
            &server_config(&server_model, app.state.server.port),
            log_tx,
            Duration::from_secs(2),
        )
        .await
        .expect("stub server starts");
    app.handle_event(AppEvent::ServerStarted(Ok(handle.clone())));
    assert!(matches!(app.server_status, ServerStatus::Ready { .. }));
    handle.stop().await;

    app.dispatch(Action::Quit);
    let reloaded = lmml_state::AppState::load_from_path(&state_path).expect("reload state");
    assert_eq!(reloaded.build.commit, "stubcommit");
    assert_eq!(reloaded.model.last_used, model_path);
    assert_eq!(reloaded.server.port, app.state.server.port);

    std::env::set_var("PATH", old_path);
}

fn write_git_stub(bin_dir: &Path) {
    write_executable(
        &bin_dir.join("git"),
        r#"#!/bin/sh
if [ "$1" = "clone" ]; then
  DEST=""
  for ARG in "$@"; do DEST="$ARG"; done
  mkdir -p "$DEST"
  echo "cmake_minimum_required(VERSION 3.21)" > "$DEST/CMakeLists.txt"
  exit 0
fi
if [ "$1" = "-C" ]; then
  if [ "$3" = "rev-parse" ]; then
    echo stubcommit
    exit 0
  fi
  if [ "$3" = "pull" ] || [ "$3" = "fetch" ] || [ "$3" = "checkout" ]; then
    exit 0
  fi
fi
exit 0
"#,
    );
}

fn write_cmake_stub(bin_dir: &Path) {
    write_executable(
        &bin_dir.join("cmake"),
        r#"#!/bin/sh
BUILD=""
PREV=""
for ARG in "$@"; do
  if [ "$PREV" = "-B" ]; then BUILD="$ARG"; fi
  if [ "$PREV" = "--build" ]; then BUILD="$ARG"; fi
  PREV="$ARG"
done
mkdir -p "$BUILD/bin"
cat > "$BUILD/bin/llama-cli" <<'CLI'
#!/bin/sh
echo llama-cli stub
exit 0
CLI
chmod +x "$BUILD/bin/llama-cli"
cat > "$BUILD/bin/llama-finetune" <<'FINETUNE'
#!/bin/sh
echo llama-finetune stub
exit 0
FINETUNE
chmod +x "$BUILD/bin/llama-finetune"
cat > "$BUILD/bin/llama-export-lora" <<'EXPORT'
#!/bin/sh
echo llama-export-lora stub
exit 0
EXPORT
chmod +x "$BUILD/bin/llama-export-lora"
cat > "$BUILD/bin/llama-server" <<'SERVER'
#!/bin/sh
if [ "$1" = "--version" ]; then
  echo llama-server stub
  exit 0
fi
HOST="127.0.0.1"
PORT="8080"
while [ "$#" -gt 0 ]; do
  case "$1" in
    --host) shift; HOST="$1" ;;
    --port) shift; PORT="$1" ;;
    --model|-m|--ctx-size|-ngl|--batch-size|--ubatch-size|--threads) shift ;;
  esac
  shift
done
exec python3 -u - "$HOST" "$PORT" <<'PY'
import http.server
import sys
host = sys.argv[1]
port = int(sys.argv[2])
class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path in ("/health", "/v1/health"):
            self.send_response(200)
            self.end_headers()
            self.wfile.write(b"ok")
        else:
            self.send_response(404)
            self.end_headers()
    def log_message(self, format, *args):
        return
http.server.ThreadingHTTPServer((host, port), Handler).serve_forever()
PY
SERVER
chmod +x "$BUILD/bin/llama-server"
echo stub cmake "$@"
exit 0
"#,
    );
}

fn write_executable(path: &Path, content: &str) {
    fs::write(path, content).expect("write executable");
    let mut perms = fs::metadata(path).expect("metadata").permissions();
    perms.set_mode(0o755);
    fs::set_permissions(path, perms).expect("chmod");
}

fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("bind free port");
    listener.local_addr().expect("local addr").port()
}

fn server_config(model: &ModelEntry, port: u16) -> ServerConfig {
    ServerConfig {
        model: model.path.clone(),
        port,
        host: "127.0.0.1".to_string(),
        ctx_size: 128,
        n_gpu_layers: 0,
        batch_size: 16,
        ubatch_size: 16,
        threads: 1,
        flash_attn: false,
        mlock: false,
        api_key: None,
        chat_template: None,
        jinja: false,
        extra_args: Vec::new(),
    }
}

fn caps() -> LlamaBinaryCapabilities {
    LlamaBinaryCapabilities {
        version: Some("stub".to_string()),
        flash_attn: false,
        flash_attn_requires_value: false,
        mlock: false,
        api_key: false,
        ubatch_size: true,
        chat_template: false,
        jinja: false,
        reranking: false,
        flags: vec![
            "--model".to_string(),
            "--host".to_string(),
            "--port".to_string(),
            "--ctx-size".to_string(),
            "-ngl".to_string(),
            "--batch-size".to_string(),
            "--ubatch-size".to_string(),
            "--threads".to_string(),
        ],
    }
}

fn profile() -> lmml_detect::SystemProfile {
    lmml_detect::SystemProfile {
        compiler: None,
        cmake: None,
        git: None,
        cuda: CudaCompatibility::NoGpu,
        rocm: lmml_detect::RocmSupport::default(),
        gpus: Vec::new(),
        gpu_probe_error: None,
        nvidia_devices: lmml_detect::NvidiaDeviceNodes::default(),
        sccache: None,
        metal: MetalSupport {
            available: false,
            displays: Vec::new(),
        },
        vulkan: lmml_detect::VulkanSupport {
            available: false,
            devices: Vec::new(),
        },
        cpu: CpuFeatures {
            model: "test".to_string(),
            cores: 2,
            threads: 4,
            avx: true,
            avx2: false,
            avx512: false,
            neon: false,
            features: Vec::new(),
        },
        memory: MemInfo {
            total_mb: 4096,
            available_mb: 2048,
        },
        disk: DiskInfo {
            available_bytes: 8 * 1024 * 1024 * 1024,
            path: PathBuf::from("."),
        },
    }
}

fn fixture_gguf() -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"GGUF");
    bytes.extend_from_slice(&3_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u64.to_le_bytes());
    bytes.extend_from_slice(&2_u64.to_le_bytes());
    write_kv_string(&mut bytes, "general.name", "E2E Test");
    write_kv_string(&mut bytes, "general.architecture", "llama");
    write_string(&mut bytes, "blk.0.attn_q.weight");
    bytes.extend_from_slice(&2_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u64.to_le_bytes());
    bytes.extend_from_slice(&1_u64.to_le_bytes());
    bytes.extend_from_slice(&12_u32.to_le_bytes());
    bytes.extend_from_slice(&0_u64.to_le_bytes());
    bytes
}

fn write_kv_string(bytes: &mut Vec<u8>, key: &str, value: &str) {
    write_string(bytes, key);
    bytes.extend_from_slice(&8_u32.to_le_bytes());
    write_string(bytes, value);
}

fn write_string(bytes: &mut Vec<u8>, value: &str) {
    bytes.extend_from_slice(&(value.len() as u64).to_le_bytes());
    bytes.extend_from_slice(value.as_bytes());
}
