#![cfg(unix)]

use std::fs;
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

#[test]
fn runtime_read_only_commands_do_not_create_state_file() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let state_path = tempdir.path().join("lmml").join("state.toml");

    let status = Command::new(env!("CARGO_BIN_EXE_lmml"))
        .env("XDG_CONFIG_HOME", tempdir.path())
        .args(["runtime", "status"])
        .status()
        .expect("run runtime status");

    assert!(status.success());
    assert!(!state_path.exists());

    let output = Command::new(env!("CARGO_BIN_EXE_lmml"))
        .env("XDG_CONFIG_HOME", tempdir.path())
        .args(["runtime", "print-config", "opencode"])
        .output()
        .expect("run runtime print-config");

    assert!(output.status.success());
    assert!(!state_path.exists());

    let output = Command::new(env!("CARGO_BIN_EXE_lmml"))
        .env("XDG_CONFIG_HOME", tempdir.path())
        .args(["runtime", "print-config", "codex"])
        .output()
        .expect("run runtime print-config codex");

    assert!(output.status.success());
    assert!(!state_path.exists());
}

#[test]
fn runtime_start_logs_and_stop_manage_detached_profile() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let config_home = tempdir.path().join("config");
    let state_home = tempdir.path().join("state");
    let data_home = tempdir.path().join("data");
    let lmml_config_dir = config_home.join("lmml");
    fs::create_dir_all(&lmml_config_dir).expect("config dir");
    fs::create_dir_all(&data_home).expect("data dir");

    let server = write_stub_server(tempdir.path());
    let model = tempdir.path().join("model.gguf");
    fs::write(&model, b"stub model").expect("model");
    let port = free_port();
    let state_path = lmml_config_dir.join("state.toml");
    let mut state = lmml_state::AppState::default();
    state.build.binary = server;
    state.runtime.opencode.model = model;
    state.runtime.opencode.port = port;
    state.save_to_path(&state_path).expect("save state");

    let start = lmml_cmd()
        .env("XDG_CONFIG_HOME", &config_home)
        .env("XDG_STATE_HOME", &state_home)
        .env("XDG_DATA_HOME", &data_home)
        .env("LMML_RUNTIME_STARTUP_TIMEOUT_MS", "800")
        .args(["runtime", "start", "opencode", "--detach"])
        .output()
        .expect("runtime start");
    assert!(
        start.status.success(),
        "start failed: {}",
        String::from_utf8_lossy(&start.stderr)
    );

    let loaded = lmml_state::AppState::load_from_path(&state_path).expect("load state");
    assert_eq!(
        loaded.runtime.state.opencode.status,
        lmml_state::RuntimeStatus::Ready
    );
    assert!(loaded.runtime.state.opencode.pid.is_some());
    assert!(loaded.runtime.state.opencode.log_path.exists());

    let logs = lmml_cmd()
        .env("XDG_CONFIG_HOME", &config_home)
        .env("XDG_STATE_HOME", &state_home)
        .env("XDG_DATA_HOME", &data_home)
        .args(["runtime", "logs", "opencode"])
        .output()
        .expect("runtime logs");
    assert!(logs.status.success());
    assert!(String::from_utf8_lossy(&logs.stdout).contains("stub runtime ready"));

    let stop = lmml_cmd()
        .env("XDG_CONFIG_HOME", &config_home)
        .env("XDG_STATE_HOME", &state_home)
        .env("XDG_DATA_HOME", &data_home)
        .args(["runtime", "stop", "opencode"])
        .output()
        .expect("runtime stop");
    assert!(
        stop.status.success(),
        "stop failed: {}",
        String::from_utf8_lossy(&stop.stderr)
    );

    let loaded = lmml_state::AppState::load_from_path(&state_path).expect("reload state");
    assert_eq!(
        loaded.runtime.state.opencode.status,
        lmml_state::RuntimeStatus::Stopped
    );
    assert!(loaded.runtime.state.opencode.pid.is_none());
}

#[test]
fn runtime_start_failure_kills_detached_descendant() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let config_home = tempdir.path().join("config");
    let state_home = tempdir.path().join("state");
    let data_home = tempdir.path().join("data");
    let lmml_config_dir = config_home.join("lmml");
    fs::create_dir_all(&lmml_config_dir).expect("config dir");
    fs::create_dir_all(&data_home).expect("data dir");

    let server = write_stub_server(tempdir.path());
    let model = tempdir.path().join("model.gguf");
    let child_pid_file = tempdir.path().join("child.pid");
    fs::write(&model, b"stub model").expect("model");
    let port = free_port();
    let state_path = lmml_config_dir.join("state.toml");
    let mut state = lmml_state::AppState::default();
    state.build.binary = server;
    state.runtime.opencode.model = model;
    state.runtime.opencode.port = port;
    state.runtime.opencode.extra_args = vec![
        "--stub-mode".to_string(),
        "never-child".to_string(),
        "--pid-file".to_string(),
        child_pid_file.display().to_string(),
    ];
    state.save_to_path(&state_path).expect("save state");

    let start = lmml_cmd()
        .env("XDG_CONFIG_HOME", &config_home)
        .env("XDG_STATE_HOME", &state_home)
        .env("XDG_DATA_HOME", &data_home)
        .env("LMML_RUNTIME_STARTUP_TIMEOUT_MS", "800")
        .args(["runtime", "start", "opencode", "--detach"])
        .output()
        .expect("runtime start");
    assert!(!start.status.success());

    let child_pid = wait_for_pid_file(&child_pid_file);
    assert!(
        wait_until_dead(child_pid, std::time::Duration::from_secs(5)),
        "descendant pid {child_pid} survived failed start cleanup"
    );
}

#[test]
fn runtime_start_refuses_existing_live_profile_before_overwriting_pid() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let config_home = tempdir.path().join("config");
    let state_home = tempdir.path().join("state");
    let data_home = tempdir.path().join("data");
    let lmml_config_dir = config_home.join("lmml");
    fs::create_dir_all(&lmml_config_dir).expect("config dir");
    fs::create_dir_all(&data_home).expect("data dir");

    let server = write_stub_server(tempdir.path());
    let model = tempdir.path().join("model.gguf");
    fs::write(&model, b"stub model").expect("model");
    let first_port = free_port();
    let state_path = lmml_config_dir.join("state.toml");
    let mut state = lmml_state::AppState::default();
    state.build.binary = server;
    state.runtime.opencode.model = model;
    state.runtime.opencode.port = first_port;
    state.save_to_path(&state_path).expect("save state");

    let start = lmml_cmd()
        .env("XDG_CONFIG_HOME", &config_home)
        .env("XDG_STATE_HOME", &state_home)
        .env("XDG_DATA_HOME", &data_home)
        .args(["runtime", "start", "opencode", "--detach"])
        .output()
        .expect("runtime start");
    assert!(
        start.status.success(),
        "start failed: {}",
        String::from_utf8_lossy(&start.stderr)
    );

    let mut state = lmml_state::AppState::load_from_path(&state_path).expect("load state");
    let original_pid = state.runtime.state.opencode.pid.expect("pid");
    state.runtime.opencode.port = free_port();
    state.save_to_path(&state_path).expect("save changed state");

    let second = lmml_cmd()
        .env("XDG_CONFIG_HOME", &config_home)
        .env("XDG_STATE_HOME", &state_home)
        .env("XDG_DATA_HOME", &data_home)
        .args(["runtime", "start", "opencode", "--detach"])
        .output()
        .expect("runtime second start");
    assert!(!second.status.success());
    assert!(String::from_utf8_lossy(&second.stderr).contains("already running"));

    let state = lmml_state::AppState::load_from_path(&state_path).expect("reload state");
    assert_eq!(state.runtime.state.opencode.pid, Some(original_pid));
    assert!(pid_is_alive(original_pid));

    let stop = lmml_cmd()
        .env("XDG_CONFIG_HOME", &config_home)
        .env("XDG_STATE_HOME", &state_home)
        .env("XDG_DATA_HOME", &data_home)
        .args(["runtime", "stop", "opencode"])
        .output()
        .expect("runtime stop");
    assert!(
        stop.status.success(),
        "stop failed: {}",
        String::from_utf8_lossy(&stop.stderr)
    );
}

fn lmml_cmd() -> Command {
    Command::new(env!("CARGO_BIN_EXE_lmml"))
}

fn write_stub_server(dir: &Path) -> PathBuf {
    let path = dir.join("llama-server-stub");
    fs::write(
        &path,
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo llama-server stub
  exit 0
fi
if [ "$1" = "--help" ]; then
  echo "--model --host --port --ctx-size -ngl --batch-size --ubatch-size --threads"
  exit 0
fi
HOST="127.0.0.1"
PORT="8080"
MODE="ready"
PID_FILE=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --host) shift; HOST="$1" ;;
    --port) shift; PORT="$1" ;;
    --stub-mode) shift; MODE="$1" ;;
    --pid-file) shift; PID_FILE="$1" ;;
    --model|-m|--ctx-size|--context-size|-c|-ngl|--n-gpu-layers|--batch-size|-b|--threads|-t|--ubatch-size|--micro-batch-size|-ub) shift ;;
  esac
  shift
done
if [ "$MODE" = "never-child" ]; then
  sleep 60 &
  CHILD="$!"
  if [ -n "$PID_FILE" ]; then echo "$CHILD" > "$PID_FILE"; fi
  wait "$CHILD"
  exit 0
fi
echo "stub runtime ready on $HOST:$PORT"
python3 -u - "$HOST" "$PORT" <<'PY' &
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
wait "$!"
"#,
    )
    .expect("write stub server");
    let mut perms = fs::metadata(&path).expect("metadata").permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&path, perms).expect("chmod");
    path
}

fn free_port() -> u16 {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind free port");
    listener.local_addr().expect("local addr").port()
}

fn wait_for_pid_file(path: &Path) -> u32 {
    for _ in 0..50 {
        if let Ok(content) = fs::read_to_string(path) {
            if let Ok(pid) = content.trim().parse() {
                return pid;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    panic!("pid file was not written: {}", path.display());
}

fn pid_is_alive(pid: u32) -> bool {
    if pid_is_zombie(pid) {
        return false;
    }
    // SAFETY: kill with signal 0 only checks process liveness.
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

fn wait_until_dead(pid: u32, timeout: std::time::Duration) -> bool {
    let start = std::time::Instant::now();
    while start.elapsed() < timeout {
        if !pid_is_alive(pid) {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    !pid_is_alive(pid)
}

#[cfg(target_os = "linux")]
fn pid_is_zombie(pid: u32) -> bool {
    fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()
        .is_some_and(|stat| {
            stat.rsplit_once(") ")
                .and_then(|(_, rest)| rest.split_whitespace().next())
                .is_some_and(|state| state == "Z")
        })
}

#[cfg(all(unix, not(target_os = "linux")))]
fn pid_is_zombie(pid: u32) -> bool {
    let output = Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "stat="])
        .output()
        .ok();
    output
        .filter(|output| output.status.success())
        .map(|output| {
            String::from_utf8_lossy(&output.stdout)
                .trim_start()
                .starts_with('Z')
        })
        .unwrap_or(false)
}
