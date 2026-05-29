use std::ffi::OsString;
use std::fs;
use std::io::Write;
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use assert_cmd::prelude::*;
use tempfile::TempDir;
use uuid::Uuid;

#[cfg(target_os = "linux")]
const TEST_BINARY_VERSION: &str = env!("CARGO_PKG_VERSION");

fn python3_available() -> bool {
    Command::new("python3").arg("--version").output().is_ok()
}

fn write_browser_capture_script(path: &Path) {
    let script = r#"#!/bin/sh
printf '%s' "$1" > "$PREVIA_OPEN_CAPTURE"
"#;

    fs::write(path, script).expect("write browser capture script");
    let mut permissions = fs::metadata(path).expect("metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).expect("chmod");
}

#[cfg(target_os = "linux")]
fn write_fake_binary(path: &Path, label: &str) {
    let script = r#"#!/bin/sh
if [ "$1" = "--version" ] || [ "$1" = "-v" ]; then
  printf '%s __VERSION__\n' "__LABEL__"
  exit 0
fi
exec python3 -u - <<'PY'
import json
import os
import pathlib
import signal
import sys
from http.server import BaseHTTPRequestHandler, HTTPServer
from urllib.parse import parse_qs, urlparse
from urllib.parse import parse_qs, urlparse

address = os.environ.get("ADDRESS", "127.0.0.1")
port = int(os.environ.get("PORT", "0"))
health_status = int(os.environ.get("HEALTH_STATUS", "200"))
health_status_file = os.environ.get("HEALTH_STATUS_FILE")
fail_port = os.environ.get("FAIL_PORT")
import_state_path = pathlib.Path(
    os.environ.get(
        "PREVIA_IMPORT_STATE",
        str(pathlib.Path(os.environ.get("PREVIA_HOME", ".")) / "fake-imports.json"),
    )
)

if os.environ.get("FAIL_STARTUP") == "1":
    sys.exit(1)
if fail_port and fail_port == str(port):
    sys.exit(1)

def load_import_state():
    if not import_state_path.exists():
        return {"projects": {}}
    return json.loads(import_state_path.read_text(encoding="utf-8"))

def save_import_state(state):
    import_state_path.parent.mkdir(parents=True, exist_ok=True)
    import_state_path.write_text(json.dumps(state, indent=2), encoding="utf-8")

def catalog_from_state(state):
    catalog = state.get("catalog")
    if isinstance(catalog, list):
        items = []
        for item in catalog:
            if not isinstance(item, dict):
                continue
            project_id = str(item.get("id", "")).strip()
            name = str(item.get("name", "")).strip()
            pipelines = item.get("pipelines")
            if not project_id or not name or not isinstance(pipelines, list):
                continue
            items.append({
                "id": project_id,
                "name": name,
                "pipelines": pipelines,
            })
        return items

    projects = state.get("projects", {})
    if not isinstance(projects, dict):
        return []

    items = []
    for stack_name, payload in projects.items():
        if not isinstance(payload, dict):
            continue
        pipelines = payload.get("pipelines")
        if not isinstance(pipelines, list):
            continue
        items.append({
            "id": str(payload.get("projectId", f"project-{stack_name}")),
            "name": str(payload.get("projectName", payload.get("stackName", stack_name))),
            "pipelines": pipelines,
        })
    return items

class Handler(BaseHTTPRequestHandler):
    def do_OPTIONS(self):
        if self.path == "/mcp":
            self.send_response(204)
            self.end_headers()
            return
        self.send_response(404)
        self.end_headers()

    def do_GET(self):
        parsed = urlparse(self.path)
        path = parsed.path
        if path == "/health":
            status = health_status
            if health_status_file and os.path.exists(health_status_file):
                with open(health_status_file, "r", encoding="utf-8") as fh:
                    status = int(fh.read().strip() or "200")
            self.send_response(status)
            self.end_headers()
            self.wfile.write(b"ok")
        elif path == "/info":
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.end_headers()
            self.wfile.write(b'{"pid":1,"memoryBytes":0,"virtualMemoryBytes":0,"cpuUsagePercent":0.0}')
        elif path == "/api/v1/projects":
            query = parse_qs(parsed.query or "")
            limit = int((query.get("limit") or ["100"])[0] or "100")
            offset = int((query.get("offset") or ["0"])[0] or "0")
            projects = catalog_from_state(load_import_state())
            page = projects[offset:offset + limit]
            self.respond_json(200, [
                {
                    "id": item["id"],
                    "name": item["name"],
                    "description": None,
                    "createdAt": "2026-01-01T00:00:00Z",
                    "updatedAt": "2026-01-01T00:00:00Z",
                }
                for item in page
            ])
        elif path.startswith("/api/v1/projects/"):
            projects = catalog_from_state(load_import_state())
            suffix = path[len("/api/v1/projects/"):]
            parts = [part for part in suffix.split("/") if part]
            if len(parts) == 1:
                project_id = parts[0]
                project = next((item for item in projects if item["id"] == project_id), None)
                if project is None:
                    self.respond_json(404, {
                        "error": "not_found",
                        "message": "project not found",
                    })
                    return
                self.respond_json(200, {
                    "id": project["id"],
                    "name": project["name"],
                    "description": None,
                    "createdAt": "2026-01-01T00:00:00Z",
                    "updatedAt": "2026-01-01T00:00:00Z",
                })
            elif len(parts) == 2 and parts[1] == "pipelines":
                project_id = parts[0]
                project = next((item for item in projects if item["id"] == project_id), None)
                if project is None:
                    self.respond_json(404, {
                        "error": "not_found",
                        "message": "project not found",
                    })
                    return
                self.respond_json(200, project["pipelines"])
            else:
                self.send_response(404)
                self.end_headers()
        else:
            self.send_response(404)
            self.end_headers()

    def do_POST(self):
        if self.path != "/api/v1/projects/import/pipelines":
            self.send_response(404)
            self.end_headers()
            return

        content_length = int(self.headers.get("Content-Length", "0") or "0")
        raw_body = self.rfile.read(content_length)
        try:
            payload = json.loads(raw_body.decode("utf-8") or "{}")
        except json.JSONDecodeError:
            self.respond_json(400, {
                "error": "bad_request",
                "message": "invalid json payload",
            })
            return

        stack_name = str(payload.get("stackName", "")).strip()
        pipelines = payload.get("pipelines")
        if not stack_name:
            self.respond_json(400, {
                "error": "bad_request",
                "message": "stackName is required",
            })
            return
        if not isinstance(pipelines, list) or not pipelines:
            self.respond_json(400, {
                "error": "bad_request",
                "message": "at least one pipeline is required",
            })
            return
        for index, pipeline in enumerate(pipelines, start=1):
            if not isinstance(pipeline, dict) or not str(pipeline.get("name", "")).strip():
                self.respond_json(400, {
                    "error": "bad_request",
                    "message": f"pipeline #{index} name is required",
                })
                return

        state = load_import_state()
        if stack_name in state.setdefault("projects", {}):
            self.respond_json(409, {
                "error": "conflict",
                "message": f"project '{stack_name}' already exists",
            })
            return

        project_id = f"project-{stack_name}"
        state["projects"][stack_name] = {
            **payload,
            "projectId": project_id,
            "projectName": stack_name,
        }
        save_import_state(state)
        self.respond_json(201, {
            "projectId": project_id,
            "stackName": stack_name,
            "pipelinesImported": len(pipelines),
        })

    def respond_json(self, status, payload):
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.end_headers()
        self.wfile.write(json.dumps(payload).encode("utf-8"))

    def log_message(self, fmt, *args):
        return

httpd = HTTPServer((address, port), Handler)
print(f"fake binary service listening on {address}:{port} pid={os.getpid()}", flush=True)

def stop(_signum, _frame):
    httpd.shutdown()

signal.signal(signal.SIGTERM, stop)
signal.signal(signal.SIGINT, stop)
httpd.serve_forever()
PY
"#
    .replace("__LABEL__", label)
    .replace("__VERSION__", TEST_BINARY_VERSION);

    fs::write(path, script).expect("write fake binary");
    let mut permissions = fs::metadata(path).expect("metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).expect("chmod");
}

#[cfg(target_os = "linux")]
fn write_fake_auth_runner_binary(path: &Path) {
    let script = r#"#!/bin/sh
if [ "$1" = "--version" ] || [ "$1" = "-v" ]; then
  printf '%s __VERSION__\n' "previa-runner"
  exit 0
fi
exec python3 -u - <<'PY'
import json
import os
import signal
import sys
from http.server import BaseHTTPRequestHandler, HTTPServer
from urllib.parse import parse_qs, urlparse

address = os.environ.get("ADDRESS", "127.0.0.1")
port = int(os.environ.get("PORT", "0"))
auth_key = os.environ.get("RUNNER_AUTH_KEY", "").strip()

class Handler(BaseHTTPRequestHandler):
    def authorized(self):
        if not auth_key:
            return True
        return self.headers.get("Authorization", "").strip() == auth_key

    def do_GET(self):
        if self.path in ("/health", "/info"):
            if not self.authorized():
                self.send_response(401)
                self.end_headers()
                return
            if self.path == "/health":
                self.send_response(200)
                self.end_headers()
                self.wfile.write(b"ok")
                return
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.end_headers()
            self.wfile.write(json.dumps({
                "pid": 1,
                "memoryBytes": 0,
                "virtualMemoryBytes": 0,
                "cpuUsagePercent": 0.0,
            }).encode("utf-8"))
            return

        self.send_response(404)
        self.end_headers()

    def log_message(self, fmt, *args):
        return

httpd = HTTPServer((address, port), Handler)
print(f"fake protected runner listening on {address}:{port} pid={os.getpid()}", flush=True)

def stop(_signum, _frame):
    httpd.shutdown()

signal.signal(signal.SIGTERM, stop)
signal.signal(signal.SIGINT, stop)
httpd.serve_forever()
PY
"#
    .replace("__VERSION__", TEST_BINARY_VERSION);

    fs::write(path, script).expect("write fake auth runner");
    let mut permissions = fs::metadata(path).expect("metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).expect("chmod");
}

fn write_fake_docker(path: &Path) {
    let script = r#"#!/bin/sh
exec python3 -u - "$@" <<'PY'
import json
import os
import pathlib
import signal
import subprocess
import sys
import time

SERVER_CODE = r"""
import json
import os
import pathlib
import signal
import sys
from http.server import BaseHTTPRequestHandler, HTTPServer
from urllib.parse import parse_qs, urlparse

address = os.environ.get("ADDRESS", "127.0.0.1")
port = int(os.environ.get("PORT", "0"))
health_status = int(os.environ.get("HEALTH_STATUS", "200"))
health_status_file = os.environ.get("HEALTH_STATUS_FILE")
fail_port = os.environ.get("FAIL_PORT")
import_state_path = pathlib.Path(
    os.environ.get(
        "PREVIA_IMPORT_STATE",
        str(pathlib.Path(os.environ.get("PREVIA_HOME", ".")) / "fake-imports.json"),
    )
)

if os.environ.get("FAIL_STARTUP") == "1":
    sys.exit(1)
if fail_port and fail_port == str(port):
    sys.exit(1)

def load_import_state():
    if not import_state_path.exists():
        return {"projects": {}}
    return json.loads(import_state_path.read_text(encoding="utf-8"))

def save_import_state(state):
    import_state_path.parent.mkdir(parents=True, exist_ok=True)
    import_state_path.write_text(json.dumps(state, indent=2), encoding="utf-8")

def catalog_from_state(state):
    catalog = state.get("catalog")
    if isinstance(catalog, list):
        items = []
        for item in catalog:
            if not isinstance(item, dict):
                continue
            project_id = str(item.get("id", "")).strip()
            name = str(item.get("name", "")).strip()
            pipelines = item.get("pipelines")
            if not project_id or not name or not isinstance(pipelines, list):
                continue
            items.append({
                "id": project_id,
                "name": name,
                "pipelines": pipelines,
            })
        return items

    projects = state.get("projects", {})
    if not isinstance(projects, dict):
        return []

    items = []
    for stack_name, payload in projects.items():
        if not isinstance(payload, dict):
            continue
        pipelines = payload.get("pipelines")
        if not isinstance(pipelines, list):
            continue
        items.append({
            "id": str(payload.get("projectId", f"project-{stack_name}")),
            "name": str(payload.get("projectName", payload.get("stackName", stack_name))),
            "pipelines": pipelines,
        })
    return items

class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        parsed = urlparse(self.path)
        path = parsed.path
        if path == "/health":
            status = health_status
            if health_status_file and os.path.exists(health_status_file):
                with open(health_status_file, "r", encoding="utf-8") as fh:
                    status = int(fh.read().strip() or "200")
            self.send_response(status)
            self.end_headers()
            self.wfile.write(b"ok")
        elif path == "/info":
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.end_headers()
            self.wfile.write(b'{"pid":1,"memoryBytes":0,"virtualMemoryBytes":0,"cpuUsagePercent":0.0}')
        elif path == "/api/v1/projects":
            query = parse_qs(parsed.query or "")
            limit = int((query.get("limit") or ["100"])[0] or "100")
            offset = int((query.get("offset") or ["0"])[0] or "0")
            projects = catalog_from_state(load_import_state())
            page = projects[offset:offset + limit]
            self.respond_json(200, [
                {
                    "id": item["id"],
                    "name": item["name"],
                    "description": None,
                    "createdAt": "2026-01-01T00:00:00Z",
                    "updatedAt": "2026-01-01T00:00:00Z",
                }
                for item in page
            ])
        elif path.startswith("/api/v1/projects/"):
            projects = catalog_from_state(load_import_state())
            suffix = path[len("/api/v1/projects/"):]
            parts = [part for part in suffix.split("/") if part]
            if len(parts) == 1:
                project_id = parts[0]
                project = next((item for item in projects if item["id"] == project_id), None)
                if project is None:
                    self.respond_json(404, {
                        "error": "not_found",
                        "message": "project not found",
                    })
                    return
                self.respond_json(200, {
                    "id": project["id"],
                    "name": project["name"],
                    "description": None,
                    "createdAt": "2026-01-01T00:00:00Z",
                    "updatedAt": "2026-01-01T00:00:00Z",
                })
            elif len(parts) == 2 and parts[1] == "pipelines":
                project_id = parts[0]
                project = next((item for item in projects if item["id"] == project_id), None)
                if project is None:
                    self.respond_json(404, {
                        "error": "not_found",
                        "message": "project not found",
                    })
                    return
                self.respond_json(200, project["pipelines"])
            else:
                self.send_response(404)
                self.end_headers()
        else:
            self.send_response(404)
            self.end_headers()

    def do_POST(self):
        if self.path != "/api/v1/projects/import/pipelines":
            self.send_response(404)
            self.end_headers()
            return

        content_length = int(self.headers.get("Content-Length", "0") or "0")
        raw_body = self.rfile.read(content_length)
        try:
            payload = json.loads(raw_body.decode("utf-8") or "{}")
        except json.JSONDecodeError:
            self.respond_json(400, {
                "error": "bad_request",
                "message": "invalid json payload",
            })
            return

        stack_name = str(payload.get("stackName", "")).strip()
        pipelines = payload.get("pipelines")
        if not stack_name:
            self.respond_json(400, {
                "error": "bad_request",
                "message": "stackName is required",
            })
            return
        if not isinstance(pipelines, list) or not pipelines:
            self.respond_json(400, {
                "error": "bad_request",
                "message": "at least one pipeline is required",
            })
            return
        for index, pipeline in enumerate(pipelines, start=1):
            if not isinstance(pipeline, dict) or not str(pipeline.get("name", "")).strip():
                self.respond_json(400, {
                    "error": "bad_request",
                    "message": f"pipeline #{index} name is required",
                })
                return

        state = load_import_state()
        if stack_name in state.setdefault("projects", {}):
            self.respond_json(409, {
                "error": "conflict",
                "message": f"project '{stack_name}' already exists",
            })
            return

        project_id = f"project-{stack_name}"
        state["projects"][stack_name] = {
            **payload,
            "projectId": project_id,
            "projectName": stack_name,
        }
        save_import_state(state)
        self.respond_json(201, {
            "projectId": project_id,
            "stackName": stack_name,
            "pipelinesImported": len(pipelines),
        })

    def respond_json(self, status, payload):
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.end_headers()
        self.wfile.write(json.dumps(payload).encode("utf-8"))

    def log_message(self, fmt, *args):
        return

httpd = HTTPServer((address, port), Handler)
print(f"fake compose service listening on {address}:{port} pid={os.getpid()}", flush=True)

def stop(_signum, _frame):
    httpd.shutdown()

signal.signal(signal.SIGTERM, stop)
signal.signal(signal.SIGINT, stop)
httpd.serve_forever()
"""

STATE_PATH = pathlib.Path(
    os.environ.get(
        "PREVIA_FAKE_DOCKER_STATE",
        str(pathlib.Path(os.environ["PREVIA_HOME"]) / "fake-docker-state.json"),
    )
)
LOG_ROOT = pathlib.Path(os.environ["PREVIA_HOME"]) / "fake-docker-logs"


def load_state():
    if not STATE_PATH.exists():
        return {"projects": {}}
    return json.loads(STATE_PATH.read_text(encoding="utf-8"))


def save_state(state):
    STATE_PATH.parent.mkdir(parents=True, exist_ok=True)
    STATE_PATH.write_text(json.dumps(state, indent=2), encoding="utf-8")


def append_log():
    log_path = os.environ.get("PREVIA_DOCKER_LOG")
    if not log_path:
        return
    with open(log_path, "a", encoding="utf-8") as fh:
        fh.write(" ".join(sys.argv[1:]) + "\n")


def process_exists(pid):
    if pid <= 0:
        return False
    try:
        os.kill(pid, 0)
        return True
    except OSError:
        return False


def stop_pid(pid):
    if not process_exists(pid):
        return
    try:
        os.kill(pid, signal.SIGTERM)
    except OSError:
        return
    for _ in range(20):
        if not process_exists(pid):
            return
        time.sleep(0.05)
    try:
        os.kill(pid, signal.SIGKILL)
    except OSError:
        return


def project_entry(state, project):
    return state.setdefault("projects", {}).setdefault(project, {"services": {}})


def spawn_service(project, service_name, service):
    service_log_dir = LOG_ROOT / project
    service_log_dir.mkdir(parents=True, exist_ok=True)
    log_path = service_log_dir / f"{service_name}.log"
    log_handle = open(log_path, "w", encoding="utf-8")

    env = os.environ.copy()
    for key, value in service.get("environment", {}).items():
        env[key] = str(value)

    ports = service.get("ports", [])
    bind_address = "127.0.0.1"
    bind_port = 0
    if ports:
        first_port = ports[0]
        if isinstance(first_port, dict):
            bind_address = str(first_port.get("host_ip", "127.0.0.1"))
            bind_port = int(first_port.get("published", 0))
        elif isinstance(first_port, str):
            parts = first_port.split(":")
            if len(parts) == 3:
                bind_address = parts[0] or "127.0.0.1"
                bind_port = int(parts[1] or "0")

    env["ADDRESS"] = bind_address
    env["PORT"] = str(bind_port)
    process = subprocess.Popen(
        ["python3", "-u", "-c", SERVER_CODE],
        env=env,
        stdout=log_handle,
        stderr=log_handle,
        close_fds=True,
    )
    time.sleep(0.2)
    if process.poll() is not None:
        log_handle.close()
        return None

    log_handle.close()
    return {
        "container_id": f"{project}_{service_name}",
        "service_name": service_name,
        "pid": process.pid,
        "running": True,
        "log_path": str(log_path),
    }


def stop_service(metadata):
    if metadata.get("running") and metadata.get("pid"):
        stop_pid(int(metadata["pid"]))
    metadata["running"] = False
    metadata["pid"] = 0


def render_logs(service_names, project_state, tail):
    chunks = []
    for service_name in service_names:
        metadata = project_state["services"].get(service_name)
        if not metadata:
            continue
        path = pathlib.Path(metadata["log_path"])
        if not path.exists():
            continue
        contents = path.read_text(encoding="utf-8")
        if tail is not None:
            lines = contents.splitlines()
            if len(lines) > tail:
                lines = lines[-tail:]
            contents = "\n".join(lines)
            if lines:
                contents += "\n"
        chunks.append(contents)
    return "".join(chunks)


append_log()
argv = sys.argv[1:]
if not argv:
    sys.exit(1)

if argv[0] == "pull":
    sys.exit(0)

if argv[0] == "inspect":
    state = load_state()
    records = []
    for container_id in argv[1:]:
        for project in state.get("projects", {}).values():
            for metadata in project.get("services", {}).values():
                if metadata["container_id"] == container_id:
                    records.append(
                        {
                            "LogPath": metadata["log_path"],
                            "State": {
                                "Running": metadata["running"],
                                "Pid": metadata["pid"],
                            },
                        }
                    )
    print(json.dumps(records))
    sys.exit(0)

if argv[0] == "compose" and len(argv) > 1 and argv[1] == "version":
    sys.exit(0)

if argv[0] != "compose":
    sys.exit(1)

idx = 1
project = None
compose_file = None
while idx < len(argv):
    if argv[idx] == "-p":
        project = argv[idx + 1]
        idx += 2
    elif argv[idx] == "-f":
        compose_file = argv[idx + 1]
        idx += 2
    else:
        break

command = argv[idx]
rest = argv[idx + 1 :]
state = load_state()
project_state = project_entry(state, project)

if command == "up":
    detached = "-d" in rest
    force_recreate = "--force-recreate" in rest
    requested_services = [value for value in rest if not value.startswith("-")]
    doc = json.loads(pathlib.Path(compose_file).read_text(encoding="utf-8"))
    services = doc.get("services", {})
    if requested_services:
        services = {name: services[name] for name in requested_services}

    if force_recreate:
        for metadata in project_state["services"].values():
            stop_service(metadata)
        project_state["services"] = {}

    started = []
    for service_name, service in services.items():
        metadata = spawn_service(project, service_name, service)
        if metadata is None:
            for started_service in started:
                stop_service(project_state["services"][started_service])
                del project_state["services"][started_service]
            save_state(state)
            sys.exit(1)
        project_state["services"][service_name] = metadata
        started.append(service_name)

    save_state(state)
    if detached:
        sys.exit(0)

    try:
        while True:
            time.sleep(0.25)
    except KeyboardInterrupt:
        for metadata in project_state["services"].values():
            stop_service(metadata)
        save_state(state)
        sys.exit(0)

elif command == "down":
    for metadata in project_state["services"].values():
        stop_service(metadata)
    state.get("projects", {}).pop(project, None)
    save_state(state)
    sys.exit(0)

elif command == "stop":
    for service_name in rest:
        metadata = project_state["services"].get(service_name)
        if metadata:
            stop_service(metadata)
    save_state(state)
    sys.exit(0)

elif command == "rm":
    service_names = [value for value in rest if not value.startswith("-")]
    for service_name in service_names:
        metadata = project_state["services"].get(service_name)
        if metadata:
            stop_service(metadata)
            del project_state["services"][service_name]
    save_state(state)
    sys.exit(0)

elif command == "ps":
    service_names = [value for value in rest if not value.startswith("-")]
    if not service_names:
        service_names = sorted(project_state["services"].keys())
    for service_name in service_names:
        metadata = project_state["services"].get(service_name)
        if metadata:
            print(metadata["container_id"])
    sys.exit(0)

elif command == "logs":
    tail = None
    follow = False
    service_names = []
    idx = 0
    while idx < len(rest):
        value = rest[idx]
        if value == "--tail":
            tail = int(rest[idx + 1])
            idx += 2
        elif value == "--follow":
            follow = True
            idx += 1
        elif value == "--no-color":
            idx += 1
        else:
            service_names.append(value)
            idx += 1
    if not service_names:
        service_names = [name for name in sorted(project_state["services"].keys())]
    sys.stdout.write(render_logs(service_names, project_state, tail))
    sys.stdout.flush()
    if follow:
        sys.exit(0)
    sys.exit(0)

sys.exit(1)
PY
"#;

    fs::write(path, script).expect("write fake docker script");
    let mut permissions = fs::metadata(path).expect("metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).expect("chmod");
}

#[cfg(target_os = "linux")]
fn write_fake_claude(path: &Path) {
    let script = r#"#!/bin/sh
exec python3 -u - "$@" <<'PY'
import json
import os
import pathlib
import sys

state_path = pathlib.Path(
    os.environ.get(
        "PREVIA_FAKE_CLAUDE_STATE",
        str(pathlib.Path(os.environ.get("PREVIA_HOME", ".")) / "fake-claude-mcp.json"),
    )
)

def load_state():
    if not state_path.exists():
        return {"user": {}, "project": {}}
    return json.loads(state_path.read_text(encoding="utf-8"))

def save_state(state):
    state_path.parent.mkdir(parents=True, exist_ok=True)
    state_path.write_text(json.dumps(state, indent=2), encoding="utf-8")

argv = sys.argv[1:]
if len(argv) < 2 or argv[0] != "mcp":
    sys.stderr.write("unsupported command\n")
    sys.exit(1)

command = argv[1]
scope = "user"
idx = 2
while idx < len(argv):
    if argv[idx] == "--scope":
        scope = argv[idx + 1]
        idx += 2
    elif argv[idx] == "--transport":
        idx += 2
    else:
        break

state = load_state()
bucket = state.setdefault(scope, {})

if command == "add":
    name = argv[idx]
    url = argv[idx + 1]
    bucket[name] = {"url": url}
    save_state(state)
    print(f"added {name} {url} {scope}")
    sys.exit(0)

if command == "remove":
    name = argv[idx]
    if name in bucket:
        del bucket[name]
        save_state(state)
        print(f"removed {name} {scope}")
        sys.exit(0)
    sys.stderr.write("not found\n")
    sys.exit(1)

if command == "get":
    name = argv[idx]
    entry = bucket.get(name)
    if not entry:
        sys.stderr.write("not found\n")
        sys.exit(1)
    print(f"name={name} url={entry['url']} scope={scope}")
    sys.exit(0)

sys.stderr.write("unsupported command\n")
sys.exit(1)
PY
"#;

    fs::write(path, script).expect("write fake claude");
    let mut permissions = fs::metadata(path).expect("metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).expect("chmod");
}

fn cargo_bin() -> Command {
    Command::cargo_bin("previa").expect("previa binary")
}

fn prepend_path(dir: &Path) -> OsString {
    let mut value = OsString::from(dir.as_os_str());
    if let Some(current) = std::env::var_os("PATH") {
        value.push(":");
        value.push(current);
    }
    value
}

fn find_free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("bind free port")
        .local_addr()
        .expect("local addr")
        .port()
}

#[cfg(target_os = "linux")]
fn find_free_port_range(count: u16) -> (u16, u16) {
    for _ in 0..1000 {
        let start = find_free_port();
        let Some(end) = start.checked_add(count.saturating_sub(1)) else {
            continue;
        };
        let listeners = (start..=end)
            .map(|port| TcpListener::bind(("127.0.0.1", port)))
            .collect::<Result<Vec<_>, _>>();

        if listeners.is_ok() {
            return (start, end);
        }
    }

    panic!("could not find {count} adjacent free ports");
}

fn setup_fake_docker() -> TempDir {
    let temp = TempDir::new().expect("tempdir");
    let docker_dir = temp.path().join("docker-bin");
    fs::create_dir_all(&docker_dir).expect("docker dir");
    write_fake_docker(&docker_dir.join("docker"));
    temp
}

#[cfg(target_os = "linux")]
fn setup_fake_claude(temp: &TempDir) {
    let bin_dir = temp.path().join("bin");
    fs::create_dir_all(&bin_dir).expect("bin dir");
    write_fake_claude(&bin_dir.join("claude"));
}

#[cfg(target_os = "linux")]
fn setup_fake_binaries(temp: &TempDir) {
    let bin_dir = temp.path().join("bin");
    fs::create_dir_all(&bin_dir).expect("bin dir");
    write_fake_binary(&bin_dir.join("previa-main"), "previa-main");
    write_fake_binary(&bin_dir.join("previa-runner"), "previa-runner");
}

#[cfg(target_os = "linux")]
fn setup_fake_binaries_with_protected_runner(temp: &TempDir) {
    let bin_dir = temp.path().join("bin");
    fs::create_dir_all(&bin_dir).expect("bin dir");
    write_fake_binary(&bin_dir.join("previa-main"), "previa-main");
    write_fake_auth_runner_binary(&bin_dir.join("previa-runner"));
}

fn docker_env(temp: &TempDir, command: &mut Command) {
    command
        .current_dir(temp.path())
        .env("HOME", temp.path())
        .env("PREVIA_HOME", temp.path())
        .env("PATH", prepend_path(&temp.path().join("docker-bin")));
}

fn docker_env_with_previa_home(preview_home: &Path, docker_root: &TempDir, command: &mut Command) {
    command
        .current_dir(docker_root.path())
        .env("HOME", docker_root.path())
        .env("PREVIA_HOME", preview_home)
        .env("PATH", prepend_path(&docker_root.path().join("docker-bin")));
}

#[cfg(target_os = "linux")]
fn mcp_env(temp: &TempDir, command: &mut Command) -> std::path::PathBuf {
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("workspace dir");
    command
        .current_dir(&workspace)
        .env("HOME", temp.path())
        .env("PREVIA_HOME", temp.path());
    workspace
}

fn write_pipeline_json(path: &Path, name: &str, pipeline_id: Option<&str>) {
    let pipeline_id = pipeline_id
        .map(|value| format!(r#""id":"{value}","#))
        .unwrap_or_default();
    fs::write(
        path,
        format!(
            r#"{{
  {pipeline_id}"name":"{name}",
  "description":"Generated by CLI test",
  "steps":[
    {{
      "id":"step-1",
      "name":"Request",
      "method":"GET",
      "url":"https://example.com",
      "headers":{{}},
      "asserts":[]
    }}
  ]
}}"#
        ),
    )
    .expect("write pipeline json");
}

fn write_pipeline_yaml(path: &Path, name: &str, pipeline_id: Option<&str>) {
    let pipeline_id = pipeline_id
        .map(|value| format!("id: {value}\n"))
        .unwrap_or_default();
    fs::write(
        path,
        format!(
            r#"{pipeline_id}name: {name}
description: Generated by CLI test
steps:
  - id: step-1
    name: Request
    method: GET
    url: https://example.com
    headers: {{}}
    asserts: []
"#
        ),
    )
    .expect("write pipeline yaml");
}

fn read_fake_import_state(previa_home: &Path) -> serde_json::Value {
    serde_json::from_slice(
        &fs::read(previa_home.join("fake-imports.json")).expect("read fake import state"),
    )
    .expect("fake import state json")
}

#[cfg(target_os = "linux")]
fn write_fake_api_state(previa_home: &Path, state: serde_json::Value) {
    fs::write(
        previa_home.join("fake-imports.json"),
        serde_json::to_vec_pretty(&state).expect("serialize fake api state"),
    )
    .expect("write fake api state");
}

#[cfg(target_os = "linux")]
fn fake_pipeline_value(name: &str, pipeline_id: Option<&str>) -> serde_json::Value {
    serde_json::json!({
        "id": pipeline_id,
        "name": name,
        "description": "Generated by CLI test",
        "steps": [{
            "id": "step-1",
            "name": "Request",
            "method": "GET",
            "url": "https://example.com",
            "headers": {},
            "asserts": []
        }]
    })
}

#[cfg(target_os = "linux")]
fn read_exported_yaml(path: &Path) -> serde_json::Value {
    serde_yaml::from_slice(&fs::read(path).expect("read exported yaml"))
        .expect("parse exported yaml")
}

#[cfg(target_os = "linux")]
fn read_exported_json(path: &Path) -> serde_json::Value {
    serde_json::from_slice(&fs::read(path).expect("read exported json"))
        .expect("parse exported json")
}

#[cfg(target_os = "linux")]
fn start_detached_bin_context(temp: &TempDir, stack: &str) {
    setup_fake_binaries(temp);
    let main_port = find_free_port();
    let runner_port = find_free_port();

    let mut up = cargo_bin();
    docker_env(temp, &mut up);
    up.args([
        "up",
        "--bin",
        "--context",
        stack,
        "--detach",
        "--main-address",
        "127.0.0.1",
        "-p",
        &main_port.to_string(),
        "--runner-address",
        "127.0.0.1",
        "-P",
        &format!("{runner_port}:{runner_port}"),
        "--runners",
        "1",
    ])
    .assert()
    .success();
}

fn read_generated_compose(previa_home: &Path, context: &str) -> serde_json::Value {
    serde_json::from_slice(
        &fs::read(
            previa_home
                .join("stacks")
                .join(context)
                .join("run")
                .join("docker-compose.generated.yaml"),
        )
        .expect("read generated compose"),
    )
    .expect("generated compose json")
}

fn read_env_var(path: &Path, key: &str) -> Option<String> {
    fs::read_to_string(path)
        .expect("read env file")
        .lines()
        .find_map(|line| {
            line.strip_prefix(&format!("{key}="))
                .map(|value| value.to_owned())
        })
}

#[test]
fn dry_run_rejects_detach() {
    let temp = setup_fake_docker();
    let mut command = cargo_bin();
    docker_env(&temp, &mut command);
    command
        .args(["up", "--dry-run", "--detach"])
        .assert()
        .failure();
}

#[test]
fn up_reports_doctor_hint_when_compose_is_missing() {
    let home = TempDir::new().expect("home tempdir");
    let empty_path = TempDir::new().expect("empty path tempdir");
    let mut command = cargo_bin();
    let output = command
        .args([
            "--home",
            home.path().to_str().expect("home str"),
            "up",
            "-d",
        ])
        .env("PATH", empty_path.path())
        .output()
        .expect("up output");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("previa doctor"),
        "expected stderr to mention previa doctor, got: {stderr}"
    );
}

#[test]
fn version_accepts_global_home_override() {
    let temp = TempDir::new().expect("tempdir");
    let mut command = cargo_bin();
    command.args(["--home", temp.path().to_str().expect("home str"), "version"]);
    command.assert().success();
}

#[test]
fn init_creates_default_previa_compose_yaml() {
    let temp = TempDir::new().expect("tempdir");
    let mut command = cargo_bin();
    command.current_dir(temp.path());
    command.args(["init"]);
    command.assert().success();

    let compose = temp.path().join("previa-compose.yaml");
    assert!(compose.exists());
    assert_eq!(
        fs::read_to_string(&compose).expect("compose contents"),
        r#"version: 1
main:
  address: 0.0.0.0
  port: 5588
runners:
  local:
    address: 127.0.0.1
    count: 1
    port_range:
      start: 55880
      end: 55889
"#
    );
}

#[test]
fn init_refuses_to_overwrite_existing_previa_compose_yaml() {
    let temp = TempDir::new().expect("tempdir");
    let compose = temp.path().join("previa-compose.yaml");
    fs::write(&compose, "version: 1\n").expect("seed compose");

    let mut command = cargo_bin();
    let output = command
        .current_dir(temp.path())
        .args(["init"])
        .output()
        .expect("init output");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("utf8 stderr");
    assert!(stderr.contains("already exists"));
    assert_eq!(
        fs::read_to_string(&compose).expect("compose contents"),
        "version: 1\n"
    );
}

#[test]
fn init_force_overwrites_existing_previa_compose_yaml() {
    let temp = TempDir::new().expect("tempdir");
    let compose = temp.path().join("previa-compose.yaml");
    fs::write(&compose, "old: true\n").expect("seed compose");

    let mut command = cargo_bin();
    command.current_dir(temp.path());
    command.args(["init", "--force"]);
    command.assert().success();

    let contents = fs::read_to_string(&compose).expect("compose contents");
    assert!(contents.contains("version: 1"));
    assert!(contents.contains("count: 1"));
    assert!(!contents.contains("old: true"));
}

#[test]
fn init_generated_compose_is_usable_by_up_dry_run() {
    let temp = setup_fake_docker();
    let main_port = find_free_port();
    let runner_port = find_free_port();
    let mut init = cargo_bin();
    init.current_dir(temp.path());
    init.args(["init"]);
    init.assert().success();

    let mut up = cargo_bin();
    docker_env(&temp, &mut up);
    up.current_dir(temp.path())
        .args([
            "up",
            "--dry-run",
            "--main-port",
            &main_port.to_string(),
            "--runner-port-range",
            &format!("{runner_port}:{runner_port}"),
            ".",
        ])
        .assert()
        .success();
}

#[cfg(target_os = "linux")]
#[test]
fn up_bin_rejects_version_override() {
    let temp = setup_fake_docker();
    setup_fake_binaries(&temp);

    let mut command = cargo_bin();
    docker_env(&temp, &mut command);
    let output = command
        .args(["up", "--bin", "--version", "0.0.7"])
        .output()
        .expect("up output");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("utf8 stderr");
    assert!(stderr.contains("--version cannot be used with --bin"));
}

#[cfg(target_os = "linux")]
#[test]
fn up_bin_reports_download_failures_when_exact_runtime_binary_is_unavailable() {
    let temp = setup_fake_docker();
    let unavailable_port = find_free_port();

    let mut command = cargo_bin();
    docker_env(&temp, &mut command);
    let output = command
        .current_dir(temp.path())
        .env(
            "PREVIA_DOWNLOAD_MANIFEST_URL",
            format!("http://127.0.0.1:{unavailable_port}/latest.json"),
        )
        .args(["up", "--bin"])
        .output()
        .expect("up output");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("utf8 stderr");
    assert!(stderr.contains("failed to download binary 'previa-main'"));
}

#[test]
fn pull_defaults_to_all_current_cli_version_without_local_binaries() {
    let temp = setup_fake_docker();
    let docker_log = temp.path().join("docker.log");

    let mut command = cargo_bin();
    docker_env(&temp, &mut command);
    command
        .env("PREVIA_DOCKER_LOG", &docker_log)
        .args(["pull"])
        .assert()
        .success();

    let output = fs::read_to_string(&docker_log).expect("docker log");
    assert!(output.contains(&format!(
        "pull ghcr.io/runvibe/main:{}",
        env!("CARGO_PKG_VERSION")
    )));
    assert!(output.contains(&format!(
        "pull ghcr.io/runvibe/runner:{}",
        env!("CARGO_PKG_VERSION")
    )));
}

#[test]
fn pull_accepts_explicit_version_for_single_target() {
    let temp = setup_fake_docker();
    let docker_log = temp.path().join("docker.log");

    let mut command = cargo_bin();
    docker_env(&temp, &mut command);
    command
        .env("PREVIA_DOCKER_LOG", &docker_log)
        .args(["pull", "runner", "--version", "0.0.7"])
        .assert()
        .success();

    let output = fs::read_to_string(&docker_log).expect("docker log");
    assert_eq!(
        output.lines().collect::<Vec<_>>(),
        vec!["pull ghcr.io/runvibe/runner:0.0.7"]
    );
}

#[test]
fn dry_run_resolves_compose_without_writing_runtime() {
    let temp = setup_fake_docker();
    let compose = temp.path().join("previa-compose.yaml");
    let main_port = find_free_port();
    let runner_port = find_free_port();
    fs::write(
        &compose,
        format!(
            r#"version: 1
main:
  address: 127.0.0.1
  port: {main_port}
runners:
  local:
    address: 127.0.0.1
    count: 1
    port_range:
      start: {runner_port}
      end: {runner_port}
"#
        ),
    )
    .expect("write compose");

    let mut command = cargo_bin();
    docker_env(&temp, &mut command);
    command
        .args(["up", "--dry-run", compose.to_str().expect("compose str")])
        .assert()
        .success();

    assert!(!temp.path().join("stacks/default/run/state.json").exists());
}

#[test]
fn detached_lifecycle_supports_status_ps_logs_list_and_down() {
    if !python3_available() {
        return;
    }

    let temp = setup_fake_docker();
    let stack = "itest";
    let main_port = find_free_port();
    let runner_port = find_free_port();

    let mut up = cargo_bin();
    docker_env(&temp, &mut up);
    up.args([
        "up",
        "--context",
        stack,
        "--detach",
        "--main-address",
        "127.0.0.1",
        "-p",
        &main_port.to_string(),
        "--runner-address",
        "127.0.0.1",
        "-P",
        &format!("{runner_port}:{runner_port}"),
        "--runners",
        "1",
    ])
    .assert()
    .success();

    thread::sleep(Duration::from_millis(500));

    let mut status = cargo_bin();
    docker_env(&temp, &mut status);
    let status_output = status
        .args(["status", "--context", stack, "--json"])
        .output()
        .expect("status output");
    assert!(status_output.status.success());
    let status_json: serde_json::Value =
        serde_json::from_slice(&status_output.stdout).expect("status json");
    assert_eq!(status_json["state"], "running");
    assert_eq!(status_json["main"]["address"], "127.0.0.1");
    assert_eq!(status_json["runners"][0]["port"], runner_port);

    let mut ps = cargo_bin();
    docker_env(&temp, &mut ps);
    let ps_output = ps
        .args(["ps", "--context", stack, "--json"])
        .output()
        .expect("ps output");
    assert!(ps_output.status.success());
    let ps_json: serde_json::Value = serde_json::from_slice(&ps_output.stdout).expect("ps json");
    assert_eq!(ps_json.as_array().expect("ps array").len(), 2);
    assert_eq!(ps_json[0]["role"], "main");
    assert_eq!(ps_json[1]["role"], "runner");

    let mut list = cargo_bin();
    docker_env(&temp, &mut list);
    let list_output = list.args(["list", "--json"]).output().expect("list output");
    assert!(list_output.status.success());
    let list_json: serde_json::Value =
        serde_json::from_slice(&list_output.stdout).expect("list json");
    assert_eq!(list_json.as_array().expect("list array")[0]["name"], stack);

    let mut logs = cargo_bin();
    docker_env(&temp, &mut logs);
    let logs_output = logs
        .args(["logs", "--context", stack, "--main"])
        .output()
        .expect("logs output");
    assert!(logs_output.status.success());
    let logs = String::from_utf8(logs_output.stdout).expect("utf8 logs");
    assert!(logs.contains("fake compose service listening"));

    let mut down = cargo_bin();
    docker_env(&temp, &mut down);
    down.args(["down", "--context", stack]).assert().success();

    assert!(
        !temp
            .path()
            .join("stacks")
            .join(stack)
            .join("run/state.json")
            .exists()
    );
}

#[test]
fn home_override_detached_lifecycle_uses_override_instead_of_previa_home_env() {
    if !python3_available() {
        return;
    }

    let docker_root = setup_fake_docker();
    let env_home = TempDir::new().expect("env home");
    let cli_home = TempDir::new().expect("cli home");
    let stack = "home-override";
    let main_port = find_free_port();
    let runner_port = find_free_port();

    let mut up = cargo_bin();
    docker_env_with_previa_home(env_home.path(), &docker_root, &mut up);
    up.args([
        "--home",
        cli_home.path().to_str().expect("home str"),
        "up",
        "--context",
        stack,
        "--detach",
        "--main-address",
        "127.0.0.1",
        "-p",
        &main_port.to_string(),
        "--runner-address",
        "127.0.0.1",
        "-P",
        &format!("{runner_port}:{runner_port}"),
        "--runners",
        "1",
    ])
    .assert()
    .success();

    let cli_runtime = cli_home
        .path()
        .join("stacks")
        .join(stack)
        .join("run/state.json");
    let env_runtime = env_home
        .path()
        .join("stacks")
        .join(stack)
        .join("run/state.json");
    assert!(cli_runtime.exists());
    assert!(!env_runtime.exists());

    let mut status = cargo_bin();
    docker_env_with_previa_home(env_home.path(), &docker_root, &mut status);
    let status_output = status
        .args([
            "--home",
            cli_home.path().to_str().expect("home str"),
            "status",
            "--context",
            stack,
            "--json",
        ])
        .output()
        .expect("status output");
    assert!(status_output.status.success());
    let status_json: serde_json::Value =
        serde_json::from_slice(&status_output.stdout).expect("status json");
    assert_eq!(
        status_json["runtime_file"],
        cli_runtime.display().to_string()
    );

    let mut down = cargo_bin();
    docker_env_with_previa_home(env_home.path(), &docker_root, &mut down);
    down.args([
        "--home",
        cli_home.path().to_str().expect("home str"),
        "down",
        "--context",
        stack,
    ])
    .assert()
    .success();

    assert!(!cli_runtime.exists());
}

#[test]
fn relative_home_override_is_resolved_from_current_directory() {
    if !python3_available() {
        return;
    }

    let docker_root = setup_fake_docker();
    let cwd = TempDir::new().expect("cwd");
    let main_port = find_free_port();
    let runner_port = find_free_port();

    let mut up = cargo_bin();
    docker_env_with_previa_home(docker_root.path(), &docker_root, &mut up);
    up.current_dir(cwd.path())
        .args([
            "--home",
            "custom-home",
            "up",
            "--detach",
            "--main-address",
            "127.0.0.1",
            "-p",
            &main_port.to_string(),
            "--runner-address",
            "127.0.0.1",
            "-P",
            &format!("{runner_port}:{runner_port}"),
            "--runners",
            "1",
        ])
        .assert()
        .success();

    assert!(
        cwd.path()
            .join("custom-home")
            .join("stacks")
            .join("default")
            .join("run/state.json")
            .exists()
    );
}

#[test]
fn up_process_runner_auth_key_overrides_compose_and_env_files() {
    if !python3_available() {
        return;
    }

    let temp = setup_fake_docker();
    let main_port = find_free_port();
    let runner_port = find_free_port();
    let compose = temp.path().join("previa-compose.yaml");
    fs::write(
        &compose,
        r#"version: 1
main:
  env:
    RUNNER_AUTH_KEY: compose-key
runners:
  local:
    count: 1
    env:
      RUNNER_AUTH_KEY: compose-key
"#,
    )
    .expect("write compose");

    let stack_config_dir = temp.path().join("stacks/default/config");
    fs::create_dir_all(&stack_config_dir).expect("stack config dir");
    fs::write(
        stack_config_dir.join("main.env"),
        "RUNNER_AUTH_KEY=env-file-key\nRUST_LOG=info\n",
    )
    .expect("main env");
    fs::write(
        stack_config_dir.join("runner.env"),
        "RUNNER_AUTH_KEY=env-file-key\nRUST_LOG=info\n",
    )
    .expect("runner env");

    let mut command = cargo_bin();
    docker_env(&temp, &mut command);
    command
        .env("RUNNER_AUTH_KEY", "process-key")
        .args([
            "up",
            "--detach",
            "-p",
            &main_port.to_string(),
            "-P",
            &format!("{runner_port}:{runner_port}"),
            compose.to_str().expect("compose path"),
        ])
        .assert()
        .success();

    let generated = read_generated_compose(temp.path(), "default");
    assert_eq!(
        generated["services"]["main"]["environment"]["RUNNER_AUTH_KEY"],
        "process-key"
    );
    assert_eq!(
        generated["services"][format!("runner-{runner_port}")]["environment"]["RUNNER_AUTH_KEY"],
        "process-key"
    );
}

#[test]
fn up_auto_generates_runner_auth_key_for_local_runners() {
    let temp = setup_fake_docker();
    let main_port = find_free_port();
    let runner_port = find_free_port();

    let mut command = cargo_bin();
    docker_env(&temp, &mut command);
    command
        .args([
            "up",
            "--detach",
            "-p",
            &main_port.to_string(),
            "-P",
            &format!("{runner_port}:{runner_port}"),
            "--runners",
            "1",
        ])
        .assert()
        .success();

    let generated = read_generated_compose(temp.path(), "default");
    let main_key = generated["services"]["main"]["environment"]["RUNNER_AUTH_KEY"]
        .as_str()
        .expect("main runner auth key");
    let runner_key =
        generated["services"][format!("runner-{runner_port}")]["environment"]["RUNNER_AUTH_KEY"]
            .as_str()
            .expect("runner auth key");

    assert_eq!(main_key, runner_key);
    assert!(Uuid::parse_str(main_key).is_ok());

    let main_env_key = read_env_var(
        &temp.path().join("stacks/default/config/main.env"),
        "RUNNER_AUTH_KEY",
    )
    .expect("main env key");
    let runner_env_key = read_env_var(
        &temp.path().join("stacks/default/config/runner.env"),
        "RUNNER_AUTH_KEY",
    )
    .expect("runner env key");

    assert_eq!(main_env_key, main_key);
    assert_eq!(runner_env_key, main_key);
}

#[cfg(target_os = "linux")]
#[test]
fn up_bin_uses_generated_runner_auth_key_for_protected_runner_health_checks() {
    if !python3_available() {
        return;
    }

    let temp = TempDir::new().expect("tempdir");
    setup_fake_binaries_with_protected_runner(&temp);
    let main_port = find_free_port();
    let runner_port = find_free_port();

    let mut up = cargo_bin();
    up.env("PREVIA_HOME", temp.path())
        .args([
            "up",
            "--bin",
            "--detach",
            "--main-address",
            "127.0.0.1",
            "-p",
            &main_port.to_string(),
            "--runner-address",
            "127.0.0.1",
            "-P",
            &format!("{runner_port}:{runner_port}"),
            "--runners",
            "1",
        ])
        .assert()
        .success();

    let state: serde_json::Value = serde_json::from_slice(
        &fs::read(temp.path().join("stacks/default/run/state.json")).expect("runtime state"),
    )
    .expect("runtime json");
    let runner_auth_key = state["runner_auth_key"]
        .as_str()
        .expect("runner auth key in state");
    assert!(Uuid::parse_str(runner_auth_key).is_ok());

    let mut status = cargo_bin();
    let output = status
        .env("PREVIA_HOME", temp.path())
        .args(["status", "--json"])
        .output()
        .expect("status output");
    assert!(output.status.success());
    let status_json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("status json");
    assert_eq!(status_json["state"], "running");
    assert_eq!(status_json["runners"][0]["state"], "running");

    let mut down = cargo_bin();
    down.env("PREVIA_HOME", temp.path())
        .args(["down"])
        .assert()
        .success();
}

#[test]
fn up_attach_runner_requires_runner_auth_key() {
    let temp = setup_fake_docker();

    let mut command = cargo_bin();
    docker_env(&temp, &mut command);
    let output = command
        .args(["up", "--detach", "--attach-runner", "55880"])
        .output()
        .expect("up output");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("utf8 stderr");
    assert!(stderr.contains("RUNNER_AUTH_KEY is required when using --attach-runner"));
}

#[test]
fn up_import_requires_stack_name() {
    let temp = setup_fake_docker();
    let pipeline = temp.path().join("single.previa.json");
    write_pipeline_json(&pipeline, "single-pipeline", None);

    let mut command = cargo_bin();
    docker_env(&temp, &mut command);
    let output = command
        .args([
            "up",
            "--detach",
            "--import",
            pipeline.to_str().expect("pipeline path"),
        ])
        .output()
        .expect("up output");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("utf8 stderr");
    assert!(stderr.contains("--stack is required when using --import"));
}

#[test]
fn up_recursive_requires_import_flag() {
    let temp = setup_fake_docker();

    let mut command = cargo_bin();
    docker_env(&temp, &mut command);
    let output = command
        .args(["up", "--detach", "--recursive"])
        .output()
        .expect("up output");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("utf8 stderr");
    assert!(stderr.contains("--recursive requires --import"));
}

#[test]
fn up_import_directory_requires_recursive_mode() {
    let temp = setup_fake_docker();
    let import_dir = temp.path().join("pipelines");
    fs::create_dir_all(&import_dir).expect("import dir");

    let mut command = cargo_bin();
    docker_env(&temp, &mut command);
    let output = command
        .args([
            "up",
            "--detach",
            "--import",
            import_dir.to_str().expect("dir path"),
            "--stack",
            "imported-stack",
        ])
        .output()
        .expect("up output");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("utf8 stderr");
    assert!(stderr.contains("--import requires a file path unless --recursive is used"));
}

#[test]
fn up_recursive_requires_directory_import_path() {
    let temp = setup_fake_docker();
    let pipeline = temp.path().join("single.previa.json");
    write_pipeline_json(&pipeline, "single-pipeline", None);

    let mut command = cargo_bin();
    docker_env(&temp, &mut command);
    let output = command
        .args([
            "up",
            "--detach",
            "--import",
            pipeline.to_str().expect("pipeline path"),
            "--recursive",
            "--stack",
            "imported-stack",
        ])
        .output()
        .expect("up output");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("utf8 stderr");
    assert!(stderr.contains("--recursive requires --import to point to a directory"));
}

#[test]
fn detached_up_imports_single_pipeline_file() {
    if !python3_available() {
        return;
    }

    let temp = setup_fake_docker();
    let stack = "import-single";
    let pipeline = temp.path().join("single.previa.json");
    write_pipeline_json(&pipeline, "single-pipeline", Some("pipe-single"));
    let main_port = find_free_port();
    let runner_port = find_free_port();

    let mut up = cargo_bin();
    docker_env(&temp, &mut up);
    let output = up
        .args([
            "up",
            "--context",
            stack,
            "--detach",
            "--main-address",
            "127.0.0.1",
            "-p",
            &main_port.to_string(),
            "--runner-address",
            "127.0.0.1",
            "-P",
            &format!("{runner_port}:{runner_port}"),
            "--runners",
            "1",
            "--import",
            pipeline.to_str().expect("pipeline path"),
            "--stack",
            "my_new_stack",
        ])
        .output()
        .expect("up output");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
    assert!(stdout.contains("imported 1 pipeline(s) into stack 'my_new_stack'"));

    let state = read_fake_import_state(temp.path());
    assert_eq!(
        state["projects"]["my_new_stack"]["stackName"],
        "my_new_stack"
    );
    assert_eq!(
        state["projects"]["my_new_stack"]["pipelines"][0]["id"],
        "pipe-single"
    );
    assert_eq!(
        state["projects"]["my_new_stack"]["pipelines"][0]["name"],
        "single-pipeline"
    );
    assert!(
        temp.path()
            .join("stacks")
            .join(stack)
            .join("run/state.json")
            .exists()
    );
}

#[test]
fn detached_up_recursive_imports_matching_pipeline_files_only() {
    if !python3_available() {
        return;
    }

    let temp = setup_fake_docker();
    let stack = "import-recursive";
    let import_dir = temp.path().join("pipelines");
    fs::create_dir_all(import_dir.join("nested")).expect("nested dir");
    write_pipeline_json(
        &import_dir.join("alpha.previa.json"),
        "alpha-pipeline",
        Some("pipe-alpha"),
    );
    write_pipeline_yaml(
        &import_dir.join("nested/beta.previa.yaml"),
        "beta-pipeline",
        Some("pipe-beta"),
    );
    write_pipeline_json(
        &import_dir.join("nested/gamma.previa"),
        "gamma-pipeline",
        Some("pipe-gamma"),
    );
    fs::write(
        import_dir.join("ignored.json"),
        r#"{"name":"ignored","steps":[]}"#,
    )
    .expect("ignored file");
    let main_port = find_free_port();
    let runner_port = find_free_port();

    let mut up = cargo_bin();
    docker_env(&temp, &mut up);
    let output = up
        .args([
            "up",
            "--context",
            stack,
            "--detach",
            "--main-address",
            "127.0.0.1",
            "-p",
            &main_port.to_string(),
            "--runner-address",
            "127.0.0.1",
            "-P",
            &format!("{runner_port}:{runner_port}"),
            "--runners",
            "1",
            "-i",
            import_dir.to_str().expect("import dir"),
            "-r",
            "-s",
            "recursive_stack",
        ])
        .output()
        .expect("up output");

    assert!(output.status.success());
    let state = read_fake_import_state(temp.path());
    let pipelines = state["projects"]["recursive_stack"]["pipelines"]
        .as_array()
        .expect("pipeline array");
    assert_eq!(pipelines.len(), 3);
    assert_eq!(pipelines[0]["name"], "alpha-pipeline");
    assert_eq!(pipelines[1]["name"], "beta-pipeline");
    assert_eq!(pipelines[2]["name"], "gamma-pipeline");
}

#[test]
fn detached_up_recursive_import_fails_for_invalid_matching_file_and_keeps_runtime() {
    if !python3_available() {
        return;
    }

    let temp = setup_fake_docker();
    let stack = "import-invalid";
    let import_dir = temp.path().join("pipelines");
    fs::create_dir_all(&import_dir).expect("import dir");
    write_pipeline_json(
        &import_dir.join("valid.previa.json"),
        "valid-pipeline",
        Some("pipe-valid"),
    );
    fs::write(import_dir.join("broken.previa.yaml"), "name: [").expect("broken file");
    let main_port = find_free_port();
    let runner_port = find_free_port();

    let mut up = cargo_bin();
    docker_env(&temp, &mut up);
    let output = up
        .args([
            "up",
            "--context",
            stack,
            "--detach",
            "--main-address",
            "127.0.0.1",
            "-p",
            &main_port.to_string(),
            "--runner-address",
            "127.0.0.1",
            "-P",
            &format!("{runner_port}:{runner_port}"),
            "--runners",
            "1",
            "--import",
            import_dir.to_str().expect("import dir"),
            "--recursive",
            "--stack",
            "broken_stack",
        ])
        .output()
        .expect("up output");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("utf8 stderr");
    assert!(stderr.contains("broken.previa.yaml"));
    assert!(
        temp.path()
            .join("stacks")
            .join(stack)
            .join("run/state.json")
            .exists()
    );
}

#[test]
fn detached_up_import_stack_conflict_keeps_runtime_running() {
    if !python3_available() {
        return;
    }

    let temp = setup_fake_docker();
    let pipeline = temp.path().join("single.previa.json");
    write_pipeline_json(&pipeline, "conflict-pipeline", Some("pipe-conflict"));

    let first_main_port = find_free_port();
    let first_runner_port = find_free_port();
    let mut first = cargo_bin();
    docker_env(&temp, &mut first);
    first
        .args([
            "up",
            "--context",
            "import-one",
            "--detach",
            "--main-address",
            "127.0.0.1",
            "-p",
            &first_main_port.to_string(),
            "--runner-address",
            "127.0.0.1",
            "-P",
            &format!("{first_runner_port}:{first_runner_port}"),
            "--runners",
            "1",
            "--import",
            pipeline.to_str().expect("pipeline path"),
            "--stack",
            "shared_stack",
        ])
        .assert()
        .success();

    let second_main_port = find_free_port();
    let second_runner_port = find_free_port();
    let mut second = cargo_bin();
    docker_env(&temp, &mut second);
    let output = second
        .args([
            "up",
            "--context",
            "import-two",
            "--detach",
            "--main-address",
            "127.0.0.1",
            "-p",
            &second_main_port.to_string(),
            "--runner-address",
            "127.0.0.1",
            "-P",
            &format!("{second_runner_port}:{second_runner_port}"),
            "--runners",
            "1",
            "--import",
            pipeline.to_str().expect("pipeline path"),
            "--stack",
            "shared_stack",
        ])
        .output()
        .expect("up output");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("utf8 stderr");
    assert!(stderr.contains("project 'shared_stack' already exists"));
    assert!(
        temp.path()
            .join("stacks")
            .join("import-two")
            .join("run/state.json")
            .exists()
    );
}

#[cfg(target_os = "linux")]
#[test]
fn export_pipelines_writes_default_yaml_files_using_project_name() {
    if !python3_available() {
        return;
    }

    let temp = setup_fake_docker();
    let stack = "export-yaml";
    start_detached_bin_context(&temp, stack);
    write_fake_api_state(
        temp.path(),
        serde_json::json!({
            "catalog": [{
                "id": "project-users",
                "name": "Users API",
                "pipelines": [
                    fake_pipeline_value("Alpha Smoke", Some("pipe-alpha")),
                    fake_pipeline_value("Beta Smoke", None)
                ]
            }]
        }),
    );

    let output_dir = temp.path().join("exports");
    let mut command = cargo_bin();
    docker_env(&temp, &mut command);
    let output = command
        .args([
            "export",
            "pipelines",
            "--context",
            stack,
            "--project",
            "Users API",
            "--output-dir",
            output_dir.to_str().expect("output dir"),
        ])
        .output()
        .expect("export output");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
    assert!(stdout.contains("exported 2 pipeline(s) from project 'Users API' (project-users)"));
    assert!(stdout.contains("as yaml"));

    let alpha_path = output_dir.join("pipe-alpha.previa.yaml");
    let beta_path = output_dir.join("beta-smoke.previa.yaml");
    assert!(alpha_path.exists());
    assert!(beta_path.exists());
    assert_eq!(read_exported_yaml(&alpha_path)["name"], "Alpha Smoke");
    assert_eq!(read_exported_yaml(&beta_path)["name"], "Beta Smoke");
}

#[cfg(target_os = "linux")]
#[test]
fn export_pipelines_supports_json_format_and_project_id_lookup() {
    if !python3_available() {
        return;
    }

    let temp = setup_fake_docker();
    let stack = "export-json";
    start_detached_bin_context(&temp, stack);
    write_fake_api_state(
        temp.path(),
        serde_json::json!({
            "catalog": [{
                "id": "project-users",
                "name": "Users API",
                "pipelines": [fake_pipeline_value("Alpha Smoke", Some("pipe-alpha"))]
            }]
        }),
    );

    let output_dir = temp.path().join("json-exports");
    let mut command = cargo_bin();
    docker_env(&temp, &mut command);
    command
        .args([
            "export",
            "pipelines",
            "--context",
            stack,
            "--project",
            "project-users",
            "--output-dir",
            output_dir.to_str().expect("output dir"),
            "--format",
            "json",
        ])
        .assert()
        .success();

    let path = output_dir.join("pipe-alpha.previa.json");
    assert!(path.exists());
    assert_eq!(read_exported_json(&path)["id"], "pipe-alpha");
}

#[cfg(target_os = "linux")]
#[test]
fn export_pipelines_filters_selection_and_preserves_stored_order() {
    if !python3_available() {
        return;
    }

    let temp = setup_fake_docker();
    let stack = "export-filtered";
    start_detached_bin_context(&temp, stack);
    write_fake_api_state(
        temp.path(),
        serde_json::json!({
            "catalog": [{
                "id": "project-users",
                "name": "Users API",
                "pipelines": [
                    fake_pipeline_value("Alpha Smoke", Some("pipe-alpha")),
                    fake_pipeline_value("Beta Smoke", Some("pipe-beta")),
                    fake_pipeline_value("Gamma Smoke", Some("pipe-gamma"))
                ]
            }]
        }),
    );

    let output_dir = temp.path().join("filtered");
    let mut command = cargo_bin();
    docker_env(&temp, &mut command);
    let output = command
        .args([
            "export",
            "pipelines",
            "--context",
            stack,
            "--project",
            "project-users",
            "--output-dir",
            output_dir.to_str().expect("output dir"),
            "--pipeline",
            "pipe-gamma",
            "--pipeline",
            "pipe-alpha",
        ])
        .output()
        .expect("export output");

    assert!(output.status.success());
    assert!(output_dir.join("pipe-alpha.previa.yaml").exists());
    assert!(output_dir.join("pipe-gamma.previa.yaml").exists());
    assert!(!output_dir.join("pipe-beta.previa.yaml").exists());

    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
    let written = stdout.lines().skip(1).collect::<Vec<_>>();
    assert_eq!(written.len(), 2);
    assert!(written[0].ends_with("pipe-alpha.previa.yaml"));
    assert!(written[1].ends_with("pipe-gamma.previa.yaml"));
}

#[cfg(target_os = "linux")]
#[test]
fn export_pipelines_fails_for_ambiguous_project_name() {
    if !python3_available() {
        return;
    }

    let temp = setup_fake_docker();
    let stack = "export-project-ambiguous";
    start_detached_bin_context(&temp, stack);
    write_fake_api_state(
        temp.path(),
        serde_json::json!({
            "catalog": [
                {"id": "project-a", "name": "Users API", "pipelines": [fake_pipeline_value("Alpha", Some("pipe-a"))]},
                {"id": "project-b", "name": "Users API", "pipelines": [fake_pipeline_value("Beta", Some("pipe-b"))]}
            ]
        }),
    );

    let mut command = cargo_bin();
    docker_env(&temp, &mut command);
    let output = command
        .args([
            "export",
            "pipelines",
            "--context",
            stack,
            "--project",
            "Users API",
            "--output-dir",
            temp.path().join("exports").to_str().expect("output dir"),
        ])
        .output()
        .expect("export output");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("utf8 stderr");
    assert!(stderr.contains("project name 'Users API' is ambiguous"));
    assert!(stderr.contains("project-a"));
    assert!(stderr.contains("project-b"));
}

#[cfg(target_os = "linux")]
#[test]
fn export_pipelines_fails_for_missing_pipeline_selector() {
    if !python3_available() {
        return;
    }

    let temp = setup_fake_docker();
    let stack = "export-pipeline-missing";
    start_detached_bin_context(&temp, stack);
    write_fake_api_state(
        temp.path(),
        serde_json::json!({
            "catalog": [{
                "id": "project-users",
                "name": "Users API",
                "pipelines": [fake_pipeline_value("Alpha Smoke", Some("pipe-alpha"))]
            }]
        }),
    );

    let mut command = cargo_bin();
    docker_env(&temp, &mut command);
    let output = command
        .args([
            "export",
            "pipelines",
            "--context",
            stack,
            "--project",
            "project-users",
            "--output-dir",
            temp.path().join("exports").to_str().expect("output dir"),
            "--pipeline",
            "missing-pipeline",
        ])
        .output()
        .expect("export output");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("utf8 stderr");
    assert!(stderr.contains("pipeline 'missing-pipeline' not found"));
}

#[cfg(target_os = "linux")]
#[test]
fn export_pipelines_fails_for_ambiguous_pipeline_name() {
    if !python3_available() {
        return;
    }

    let temp = setup_fake_docker();
    let stack = "export-pipeline-ambiguous";
    start_detached_bin_context(&temp, stack);
    write_fake_api_state(
        temp.path(),
        serde_json::json!({
            "catalog": [{
                "id": "project-users",
                "name": "Users API",
                "pipelines": [
                    fake_pipeline_value("Smoke", Some("pipe-alpha")),
                    fake_pipeline_value("Smoke", Some("pipe-beta"))
                ]
            }]
        }),
    );

    let mut command = cargo_bin();
    docker_env(&temp, &mut command);
    let output = command
        .args([
            "export",
            "pipelines",
            "--context",
            stack,
            "--project",
            "project-users",
            "--output-dir",
            temp.path().join("exports").to_str().expect("output dir"),
            "--pipeline",
            "Smoke",
        ])
        .output()
        .expect("export output");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("utf8 stderr");
    assert!(stderr.contains("pipeline name 'Smoke' is ambiguous"));
    assert!(stderr.contains("pipe-alpha"));
    assert!(stderr.contains("pipe-beta"));
}

#[cfg(target_os = "linux")]
#[test]
fn export_pipelines_respects_overwrite_policy() {
    if !python3_available() {
        return;
    }

    let temp = setup_fake_docker();
    let stack = "export-overwrite";
    start_detached_bin_context(&temp, stack);
    write_fake_api_state(
        temp.path(),
        serde_json::json!({
            "catalog": [{
                "id": "project-users",
                "name": "Users API",
                "pipelines": [fake_pipeline_value("Alpha Smoke", Some("pipe-alpha"))]
            }]
        }),
    );

    let output_dir = temp.path().join("exports");
    fs::create_dir_all(&output_dir).expect("export dir");
    let existing = output_dir.join("pipe-alpha.previa.yaml");
    fs::write(&existing, "old: true\n").expect("existing export");

    let mut without_overwrite = cargo_bin();
    docker_env(&temp, &mut without_overwrite);
    let output = without_overwrite
        .args([
            "export",
            "pipelines",
            "--context",
            stack,
            "--project",
            "project-users",
            "--output-dir",
            output_dir.to_str().expect("output dir"),
        ])
        .output()
        .expect("export output");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("utf8 stderr");
    assert!(stderr.contains("already exists"));
    assert_eq!(
        fs::read_to_string(&existing).expect("existing contents"),
        "old: true\n"
    );

    let mut with_overwrite = cargo_bin();
    docker_env(&temp, &mut with_overwrite);
    with_overwrite
        .args([
            "export",
            "pipelines",
            "--context",
            stack,
            "--project",
            "project-users",
            "--output-dir",
            output_dir.to_str().expect("output dir"),
            "--overwrite",
        ])
        .assert()
        .success();

    assert_eq!(read_exported_yaml(&existing)["name"], "Alpha Smoke");
}

#[cfg(target_os = "linux")]
#[test]
fn export_pipelines_detects_duplicate_target_filenames_before_writing() {
    if !python3_available() {
        return;
    }

    let temp = setup_fake_docker();
    let stack = "export-duplicate-path";
    start_detached_bin_context(&temp, stack);
    write_fake_api_state(
        temp.path(),
        serde_json::json!({
            "catalog": [{
                "id": "project-users",
                "name": "Users API",
                "pipelines": [
                    fake_pipeline_value("Alpha", Some("dup")),
                    fake_pipeline_value("Beta", Some("dup"))
                ]
            }]
        }),
    );

    let output_dir = temp.path().join("exports");
    let mut command = cargo_bin();
    docker_env(&temp, &mut command);
    let output = command
        .args([
            "export",
            "pipelines",
            "--context",
            stack,
            "--project",
            "project-users",
            "--output-dir",
            output_dir.to_str().expect("output dir"),
        ])
        .output()
        .expect("export output");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("utf8 stderr");
    assert!(stderr.contains("same output file"));
    assert!(!output_dir.join("dup.previa.yaml").exists());
}

#[cfg(target_os = "linux")]
#[test]
fn export_pipelines_creates_output_directory_when_missing() {
    if !python3_available() {
        return;
    }

    let temp = setup_fake_docker();
    let stack = "export-create-dir";
    start_detached_bin_context(&temp, stack);
    write_fake_api_state(
        temp.path(),
        serde_json::json!({
            "catalog": [{
                "id": "project-users",
                "name": "Users API",
                "pipelines": [fake_pipeline_value("Alpha Smoke", Some("pipe-alpha"))]
            }]
        }),
    );

    let output_dir = temp.path().join("nested/exports");
    let mut command = cargo_bin();
    docker_env(&temp, &mut command);
    command
        .args([
            "export",
            "pipelines",
            "--context",
            stack,
            "--project",
            "project-users",
            "--output-dir",
            output_dir.to_str().expect("output dir"),
        ])
        .assert()
        .success();

    assert!(output_dir.exists());
    assert!(output_dir.join("pipe-alpha.previa.yaml").exists());
}

#[test]
fn export_pipelines_requires_detached_context() {
    let temp = setup_fake_docker();
    let mut command = cargo_bin();
    docker_env(&temp, &mut command);
    let output = command
        .args([
            "export",
            "pipelines",
            "--context",
            "missing-context",
            "--project",
            "project-users",
            "--output-dir",
            temp.path().join("exports").to_str().expect("output dir"),
        ])
        .output()
        .expect("export output");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("utf8 stderr");
    assert!(stderr.contains("no detached runtime exists for context 'missing-context'"));
}

#[cfg(target_os = "linux")]
#[test]
fn mcp_install_status_print_and_uninstall_codex_global() {
    if !python3_available() {
        return;
    }

    let temp = setup_fake_docker();
    let stack = "mcp-codex";
    start_detached_bin_context(&temp, stack);

    let mut install = cargo_bin();
    mcp_env(&temp, &mut install);
    install
        .args(["mcp", "install", "codex", "--context", stack])
        .assert()
        .success();

    let config_path = temp.path().join(".codex").join("config.toml");
    let config = fs::read_to_string(&config_path).expect("codex config");
    assert!(config.contains("mcp_servers"));
    assert!(config.contains("previa"));
    assert!(config.contains("enabled = true"));
    assert!(config.contains("url = \"http://127.0.0.1:"));
    assert!(config.contains("/mcp\""));

    let mut status = cargo_bin();
    mcp_env(&temp, &mut status);
    let output = status
        .args(["mcp", "status", "codex"])
        .output()
        .expect("status output");
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
    assert!(stdout.contains("target: codex"));
    assert!(stdout.contains("installed: yes"));
    assert!(stdout.contains("live: reachable"));

    let mut print = cargo_bin();
    mcp_env(&temp, &mut print);
    let output = print
        .args(["mcp", "print", "codex", "--context", stack])
        .output()
        .expect("print output");
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
    assert!(stdout.contains("[mcp_servers.previa]"));

    let mut uninstall = cargo_bin();
    mcp_env(&temp, &mut uninstall);
    uninstall
        .args(["mcp", "uninstall", "codex"])
        .assert()
        .success();

    let config = fs::read_to_string(&config_path).expect("codex config after uninstall");
    assert!(!config.contains("[mcp_servers.previa]"));
}

#[cfg(target_os = "linux")]
#[test]
fn mcp_install_codex_requires_force_for_conflicting_entry() {
    let temp = TempDir::new().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(workspace.join(".codex")).expect("workspace codex dir");
    fs::create_dir_all(temp.path().join(".codex")).expect("global codex dir");
    fs::write(
        temp.path().join(".codex").join("config.toml"),
        r#"[mcp_servers.previa]
enabled = true
url = "http://old.example/mcp"
"#,
    )
    .expect("seed codex config");

    let mut install = cargo_bin();
    install
        .current_dir(&workspace)
        .env("HOME", temp.path())
        .env("PREVIA_HOME", temp.path());
    let output = install
        .args([
            "mcp",
            "install",
            "codex",
            "--url",
            "http://new.example/mcp",
            "--no-verify",
        ])
        .output()
        .expect("install output");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("utf8 stderr");
    assert!(stderr.contains("rerun with --force"));

    let mut force_install = cargo_bin();
    force_install
        .current_dir(&workspace)
        .env("HOME", temp.path())
        .env("PREVIA_HOME", temp.path());
    force_install
        .args([
            "mcp",
            "install",
            "codex",
            "--url",
            "http://new.example/mcp",
            "--force",
            "--no-verify",
        ])
        .assert()
        .success();

    let config =
        fs::read_to_string(temp.path().join(".codex").join("config.toml")).expect("codex config");
    assert!(config.contains("http://new.example/mcp"));
}

#[cfg(target_os = "linux")]
#[test]
fn mcp_install_and_uninstall_cursor_project_preserves_other_servers() {
    let temp = TempDir::new().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(workspace.join(".cursor")).expect("cursor dir");
    fs::write(
        workspace.join(".cursor").join("mcp.json"),
        serde_json::to_string_pretty(&serde_json::json!({
            "mcpServers": {
                "existing": {
                    "url": "http://existing.example/mcp"
                }
            },
            "otherKey": true
        }))
        .expect("seed json"),
    )
    .expect("seed file");

    let mut install = cargo_bin();
    install
        .current_dir(&workspace)
        .env("HOME", temp.path())
        .env("PREVIA_HOME", temp.path());
    install
        .args([
            "mcp",
            "install",
            "cursor",
            "--scope",
            "project",
            "--url",
            "http://cursor.example/mcp",
            "--no-verify",
        ])
        .assert()
        .success();

    let config: serde_json::Value = serde_json::from_slice(
        &fs::read(workspace.join(".cursor").join("mcp.json")).expect("cursor config"),
    )
    .expect("cursor json");
    assert_eq!(config["otherKey"], true);
    assert_eq!(
        config["mcpServers"]["existing"]["url"],
        "http://existing.example/mcp"
    );
    assert_eq!(
        config["mcpServers"]["previa"]["url"],
        "http://cursor.example/mcp"
    );

    let mut uninstall = cargo_bin();
    uninstall
        .current_dir(&workspace)
        .env("HOME", temp.path())
        .env("PREVIA_HOME", temp.path());
    uninstall
        .args(["mcp", "uninstall", "cursor", "--scope", "project"])
        .assert()
        .success();

    let config: serde_json::Value = serde_json::from_slice(
        &fs::read(workspace.join(".cursor").join("mcp.json")).expect("cursor config"),
    )
    .expect("cursor json");
    assert!(config["mcpServers"].get("previa").is_none());
    assert_eq!(
        config["mcpServers"]["existing"]["url"],
        "http://existing.example/mcp"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn mcp_install_and_status_copilot_vscode_project() {
    let temp = TempDir::new().expect("tempdir");

    let mut install = cargo_bin();
    let workspace = mcp_env(&temp, &mut install);
    install
        .args([
            "mcp",
            "install",
            "copilot-vscode",
            "--scope",
            "project",
            "--url",
            "http://copilot.example/mcp",
            "--no-verify",
        ])
        .assert()
        .success();

    let config: serde_json::Value = serde_json::from_slice(
        &fs::read(workspace.join(".vscode").join("mcp.json")).expect("copilot config"),
    )
    .expect("copilot json");
    assert_eq!(
        config["mcpServers"]["previa"]["url"],
        "http://copilot.example/mcp"
    );

    let mut status = cargo_bin();
    mcp_env(&temp, &mut status);
    let output = status
        .args(["mcp", "status", "copilot-vscode", "--scope", "project"])
        .output()
        .expect("status output");
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
    assert!(stdout.contains("target: copilot-vscode"));
    assert!(stdout.contains("installed: yes"));
    assert!(stdout.contains("live: unreachable"));
}

#[cfg(target_os = "linux")]
#[test]
fn mcp_install_status_print_and_uninstall_warp_global() {
    let temp = TempDir::new().expect("tempdir");
    let mut install = cargo_bin();
    mcp_env(&temp, &mut install);
    install
        .args([
            "mcp",
            "install",
            "warp",
            "--url",
            "http://warp.example/mcp",
            "--no-verify",
        ])
        .assert()
        .success();

    let path = temp.path().join("clients").join("warp").join("previa.json");
    let config: serde_json::Value =
        serde_json::from_slice(&fs::read(&path).expect("warp config")).expect("warp json");
    assert_eq!(
        config["mcpServers"]["previa"]["url"],
        "http://warp.example/mcp"
    );

    let mut status = cargo_bin();
    mcp_env(&temp, &mut status);
    let output = status
        .args(["mcp", "status", "warp"])
        .output()
        .expect("status output");
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
    assert!(stdout.contains("config:"));
    assert!(stdout.contains("installed: yes"));

    let mut print = cargo_bin();
    mcp_env(&temp, &mut print);
    let output = print
        .args(["mcp", "print", "warp", "--url", "http://warp.example/mcp"])
        .output()
        .expect("print output");
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
    assert!(stdout.contains("oz agent run --mcp"));

    let mut uninstall = cargo_bin();
    mcp_env(&temp, &mut uninstall);
    uninstall
        .args(["mcp", "uninstall", "warp"])
        .assert()
        .success();
    assert!(!path.exists());
}

#[cfg(target_os = "linux")]
#[test]
fn mcp_warp_project_scope_is_unsupported() {
    let temp = TempDir::new().expect("tempdir");
    let mut command = cargo_bin();
    mcp_env(&temp, &mut command);
    let output = command
        .args([
            "mcp",
            "install",
            "warp",
            "--scope",
            "project",
            "--url",
            "http://warp.example/mcp",
        ])
        .output()
        .expect("install output");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("utf8 stderr");
    assert!(stderr.contains("supports only --scope global"));
}

#[cfg(target_os = "linux")]
#[test]
fn mcp_claude_code_install_status_print_and_uninstall() {
    let temp = TempDir::new().expect("tempdir");
    setup_fake_claude(&temp);

    let mut install = cargo_bin();
    mcp_env(&temp, &mut install);
    install.env("PATH", prepend_path(&temp.path().join("bin")));
    install
        .args([
            "mcp",
            "install",
            "claude-code",
            "--url",
            "http://claude.example/mcp",
            "--no-verify",
        ])
        .assert()
        .success();

    let mut status = cargo_bin();
    mcp_env(&temp, &mut status);
    status.env("PATH", prepend_path(&temp.path().join("bin")));
    let output = status
        .args(["mcp", "status", "claude-code"])
        .output()
        .expect("status output");
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
    assert!(stdout.contains("mode: claude-cli"));
    assert!(stdout.contains("installed: yes"));
    assert!(stdout.contains("url: http://claude.example/mcp"));

    let mut print = cargo_bin();
    mcp_env(&temp, &mut print);
    let output = print
        .env("PATH", prepend_path(&temp.path().join("bin")))
        .args([
            "mcp",
            "print",
            "claude-code",
            "--url",
            "http://claude.example/mcp",
        ])
        .output()
        .expect("print output");
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
    assert!(
        stdout.contains(
            "claude mcp add --scope user --transport http previa http://claude.example/mcp"
        )
    );

    let mut uninstall = cargo_bin();
    mcp_env(&temp, &mut uninstall);
    uninstall.env("PATH", prepend_path(&temp.path().join("bin")));
    uninstall
        .args(["mcp", "uninstall", "claude-code"])
        .assert()
        .success();
}

#[cfg(target_os = "linux")]
#[test]
fn mcp_claude_desktop_is_manual_only() {
    let temp = TempDir::new().expect("tempdir");

    let mut print = cargo_bin();
    mcp_env(&temp, &mut print);
    let output = print
        .args([
            "mcp",
            "print",
            "claude-desktop",
            "--url",
            "http://desktop.example/mcp",
        ])
        .output()
        .expect("print output");
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
    assert!(stdout.contains("manual-only"));
    assert!(stdout.contains("http://desktop.example/mcp"));

    let mut install = cargo_bin();
    mcp_env(&temp, &mut install);
    let output = install
        .args([
            "mcp",
            "install",
            "claude-desktop",
            "--url",
            "http://desktop.example/mcp",
        ])
        .output()
        .expect("install output");
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("utf8 stderr");
    assert!(stderr.contains("print claude-desktop"));
}

#[cfg(target_os = "linux")]
#[test]
fn mcp_install_fails_without_context_or_url() {
    let temp = TempDir::new().expect("tempdir");
    let mut command = cargo_bin();
    mcp_env(&temp, &mut command);
    let output = command
        .args(["mcp", "install", "codex"])
        .output()
        .expect("install output");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("utf8 stderr");
    assert!(stderr.contains("no detached runtime exists for context 'default'"));
}

#[cfg(target_os = "linux")]
#[test]
fn mcp_install_honors_no_verify_for_unreachable_url() {
    let temp = TempDir::new().expect("tempdir");

    let mut without = cargo_bin();
    mcp_env(&temp, &mut without);
    let output = without
        .args(["mcp", "install", "codex", "--url", "http://127.0.0.1:9/mcp"])
        .output()
        .expect("install output");
    assert!(!output.status.success());

    let mut with = cargo_bin();
    mcp_env(&temp, &mut with);
    with.args([
        "mcp",
        "install",
        "codex",
        "--url",
        "http://127.0.0.1:9/mcp",
        "--no-verify",
    ])
    .assert()
    .success();
}

#[cfg(target_os = "linux")]
#[test]
fn detached_binary_lifecycle_supports_status_ps_logs_restart_and_down() {
    if !python3_available() {
        return;
    }

    let temp = setup_fake_docker();
    setup_fake_binaries(&temp);
    let stack = "bin-itest";
    let main_port = find_free_port();
    let runner_port = find_free_port();

    let mut up = cargo_bin();
    docker_env(&temp, &mut up);
    up.args([
        "up",
        "--bin",
        "--context",
        stack,
        "--detach",
        "--main-address",
        "127.0.0.1",
        "-p",
        &main_port.to_string(),
        "--runner-address",
        "127.0.0.1",
        "-P",
        &format!("{runner_port}:{runner_port}"),
        "--runners",
        "1",
    ])
    .assert()
    .success();

    let mut status_json = serde_json::Value::Null;
    for _ in 0..10 {
        let mut status = cargo_bin();
        docker_env(&temp, &mut status);
        let status_output = status
            .args(["status", "--context", stack, "--json"])
            .output()
            .expect("status output");
        assert!(status_output.status.success());
        status_json = serde_json::from_slice(&status_output.stdout).expect("status json");
        if status_json["state"] == "running" {
            break;
        }
        thread::sleep(Duration::from_millis(200));
    }

    let state: serde_json::Value = serde_json::from_slice(
        &fs::read(
            temp.path()
                .join("stacks")
                .join(stack)
                .join("run/state.json"),
        )
        .expect("runtime state"),
    )
    .expect("runtime json");
    assert_eq!(state["backend"], "bin");
    assert!(state["main"]["pid"].as_u64().unwrap_or_default() > 0);
    assert_eq!(status_json["state"], "running");
    assert_eq!(status_json["main"]["address"], "127.0.0.1");

    let mut ps = cargo_bin();
    docker_env(&temp, &mut ps);
    let ps_output = ps
        .args(["ps", "--context", stack, "--json"])
        .output()
        .expect("ps output");
    assert!(ps_output.status.success());
    let ps_json: serde_json::Value = serde_json::from_slice(&ps_output.stdout).expect("ps json");
    assert_eq!(ps_json.as_array().expect("ps array").len(), 2);
    assert_eq!(ps_json[0]["role"], "main");
    assert!(ps_json[0]["pid"].as_u64().unwrap_or_default() > 0);

    let mut logs = cargo_bin();
    docker_env(&temp, &mut logs);
    let logs_output = logs
        .args(["logs", "--context", stack, "--main"])
        .output()
        .expect("logs output");
    assert!(logs_output.status.success());
    let logs = String::from_utf8(logs_output.stdout).expect("utf8 logs");
    assert!(logs.contains("fake binary service listening"));

    let mut restart = cargo_bin();
    docker_env(&temp, &mut restart);
    restart
        .args(["restart", "--context", stack])
        .assert()
        .success();

    let mut restart_with_version = cargo_bin();
    docker_env(&temp, &mut restart_with_version);
    let restart_output = restart_with_version
        .args(["restart", "--context", stack, "--version", "0.0.8"])
        .output()
        .expect("restart output");
    assert!(!restart_output.status.success());
    let restart_stderr = String::from_utf8(restart_output.stderr).expect("utf8 stderr");
    assert!(restart_stderr.contains("--version is only supported for compose-backed runtimes"));

    let mut down = cargo_bin();
    docker_env(&temp, &mut down);
    down.args(["down", "--context", stack]).assert().success();

    assert!(
        !temp
            .path()
            .join("stacks")
            .join(stack)
            .join("run/state.json")
            .exists()
    );
}

#[cfg(target_os = "linux")]
#[test]
fn down_runner_removes_selected_runner_and_rewrites_runtime() {
    if !python3_available() {
        return;
    }

    let temp = setup_fake_docker();
    let stack = "partial";
    let main_port = find_free_port();
    let (runner_start, runner_end) = find_free_port_range(2);

    let mut up = cargo_bin();
    docker_env(&temp, &mut up);
    up.args([
        "up",
        "--context",
        stack,
        "--detach",
        "--main-address",
        "127.0.0.1",
        "-p",
        &main_port.to_string(),
        "--runner-address",
        "127.0.0.1",
        "-P",
        &format!("{runner_start}:{runner_end}"),
        "--runners",
        "2",
    ])
    .assert()
    .success();

    let mut down = cargo_bin();
    docker_env(&temp, &mut down);
    down.args([
        "down",
        "--context",
        stack,
        "--runner",
        &runner_start.to_string(),
    ])
    .assert()
    .success();

    let state: serde_json::Value = serde_json::from_slice(
        &fs::read(
            temp.path()
                .join("stacks")
                .join(stack)
                .join("run/state.json"),
        )
        .expect("runtime state"),
    )
    .expect("runtime json");
    assert_eq!(state["runners"].as_array().expect("runner array").len(), 1);
    assert_eq!(state["runners"][0]["port"], runner_end);

    let compose_file = fs::read_to_string(
        temp.path()
            .join("stacks")
            .join(stack)
            .join("run/docker-compose.generated.yaml"),
    )
    .expect("compose file");
    assert!(!compose_file.contains(&format!("runner-{runner_start}")));
    assert!(compose_file.contains(&format!("runner-{runner_end}")));
}

#[test]
fn restart_allows_overriding_image_tag() {
    if !python3_available() {
        return;
    }

    let temp = setup_fake_docker();
    let stack = "retag";
    let main_port = find_free_port();
    let runner_port = find_free_port();

    let mut up = cargo_bin();
    docker_env(&temp, &mut up);
    up.args([
        "up",
        "--context",
        stack,
        "--detach",
        "--version",
        "0.0.7",
        "--main-address",
        "127.0.0.1",
        "-p",
        &main_port.to_string(),
        "--runner-address",
        "127.0.0.1",
        "-P",
        &format!("{runner_port}:{runner_port}"),
        "--runners",
        "1",
    ])
    .assert()
    .success();

    let mut restart = cargo_bin();
    docker_env(&temp, &mut restart);
    restart
        .args(["restart", "--context", stack, "--version", "0.0.8"])
        .assert()
        .success();

    let state: serde_json::Value = serde_json::from_slice(
        &fs::read(
            temp.path()
                .join("stacks")
                .join(stack)
                .join("run/state.json"),
        )
        .expect("runtime state"),
    )
    .expect("runtime json");
    assert_eq!(state["image_tag"], "0.0.8");

    let compose_file = fs::read_to_string(
        temp.path()
            .join("stacks")
            .join(stack)
            .join("run/docker-compose.generated.yaml"),
    )
    .expect("compose file");
    assert!(compose_file.contains("ghcr.io/runvibe/main:0.0.8"));
}

#[test]
fn up_fails_early_when_context_is_already_running() {
    if !python3_available() {
        return;
    }

    let temp = setup_fake_docker();
    let stack = "busy";
    let main_port = find_free_port();
    let runner_port = find_free_port();

    let mut first = cargo_bin();
    docker_env(&temp, &mut first);
    first
        .args([
            "up",
            "--context",
            stack,
            "--detach",
            "--main-address",
            "127.0.0.1",
            "-p",
            &main_port.to_string(),
            "--runner-address",
            "127.0.0.1",
            "-P",
            &format!("{runner_port}:{runner_port}"),
            "--runners",
            "1",
        ])
        .assert()
        .success();

    let next_main_port = find_free_port();
    let mut second = cargo_bin();
    docker_env(&temp, &mut second);
    let output = second
        .args([
            "up",
            "--context",
            stack,
            "--main-port",
            &next_main_port.to_string(),
        ])
        .output()
        .expect("up output");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("utf8 stderr");
    assert!(!stderr.trim().is_empty());
}

#[test]
fn down_all_context_stops_every_detached_context() {
    if !python3_available() {
        return;
    }

    let temp = setup_fake_docker();
    let alpha_main_port = find_free_port();
    let alpha_runner_port = find_free_port();
    let beta_main_port = find_free_port();
    let beta_runner_port = find_free_port();

    let mut alpha = cargo_bin();
    docker_env(&temp, &mut alpha);
    alpha
        .args([
            "up",
            "--context",
            "alpha",
            "--detach",
            "--main-address",
            "127.0.0.1",
            "-p",
            &alpha_main_port.to_string(),
            "--runner-address",
            "127.0.0.1",
            "-P",
            &format!("{alpha_runner_port}:{alpha_runner_port}"),
            "--runners",
            "1",
        ])
        .assert()
        .success();

    let mut beta = cargo_bin();
    docker_env(&temp, &mut beta);
    beta.args([
        "up",
        "--context",
        "beta",
        "--detach",
        "--main-address",
        "127.0.0.1",
        "-p",
        &beta_main_port.to_string(),
        "--runner-address",
        "127.0.0.1",
        "-P",
        &format!("{beta_runner_port}:{beta_runner_port}"),
        "--runners",
        "1",
    ])
    .assert()
    .success();

    let mut down = cargo_bin();
    docker_env(&temp, &mut down);
    down.args(["down", "--all-contexts"]).assert().success();

    assert!(!temp.path().join("stacks/alpha/run/state.json").exists());
    assert!(!temp.path().join("stacks/beta/run/state.json").exists());
}

#[test]
fn logs_supports_tail_count() {
    if !python3_available() {
        return;
    }

    let temp = setup_fake_docker();
    let stack = "tailtest";
    let main_port = find_free_port();
    let runner_port = find_free_port();

    let mut up = cargo_bin();
    docker_env(&temp, &mut up);
    up.args([
        "up",
        "--context",
        stack,
        "--detach",
        "--main-address",
        "127.0.0.1",
        "-p",
        &main_port.to_string(),
        "--runner-address",
        "127.0.0.1",
        "-P",
        &format!("{runner_port}:{runner_port}"),
        "--runners",
        "1",
    ])
    .assert()
    .success();

    let main_log = temp
        .path()
        .join("fake-docker-logs")
        .join("previa_tailtest")
        .join("main.log");
    fs::OpenOptions::new()
        .append(true)
        .open(&main_log)
        .expect("open main log")
        .write_all(b"line-one\nline-two\nline-three\n")
        .expect("append main log");

    let mut logs = cargo_bin();
    docker_env(&temp, &mut logs);
    let logs_output = logs
        .args(["logs", "--context", stack, "--main", "-t", "2"])
        .output()
        .expect("logs output");
    assert!(logs_output.status.success());
    let logs = String::from_utf8(logs_output.stdout).expect("utf8 logs");
    assert_eq!(logs, "line-two\nline-three\n");
}

#[test]
fn up_prompts_and_accepts_shifted_main_port_on_enter() {
    if !python3_available() {
        return;
    }

    let temp = setup_fake_docker();
    let occupied_main_port = find_free_port();
    let runner_port = find_free_port();
    let _occupied = TcpListener::bind(("127.0.0.1", occupied_main_port)).expect("occupy main");

    let output = run_command_with_stdin(
        temp.path(),
        [
            "up",
            "--detach",
            "--main-address",
            "127.0.0.1",
            "-p",
            &occupied_main_port.to_string(),
            "--runner-address",
            "127.0.0.1",
            "-P",
            &format!("{runner_port}:{runner_port}"),
        ],
        "\n",
    );
    assert!(output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("utf8 stderr");
    assert!(stderr.contains("press [Y] to continue with main port"));

    let state: serde_json::Value = serde_json::from_slice(
        &fs::read(temp.path().join("stacks/default/run/state.json")).expect("runtime state"),
    )
    .expect("runtime json");
    assert_eq!(state["main"]["port"], occupied_main_port + 100);
}

#[test]
fn up_prompts_and_accepts_shifted_runner_range_on_enter() {
    if !python3_available() {
        return;
    }

    let temp = setup_fake_docker();
    let main_port = find_free_port();
    let occupied = TcpListener::bind("127.0.0.1:0").expect("occupy runner port");
    let occupied_runner_port = occupied.local_addr().expect("occupied runner addr").port();

    let output = run_command_with_stdin(
        temp.path(),
        [
            "up",
            "--detach",
            "--main-address",
            "127.0.0.1",
            "-p",
            &main_port.to_string(),
            "--runner-address",
            "127.0.0.1",
            "-P",
            &format!("{occupied_runner_port}:{occupied_runner_port}"),
        ],
        "\n",
    );
    assert!(output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("utf8 stderr");
    assert!(stderr.contains("press [Y] to continue with runner ports starting at"));

    let state: serde_json::Value = serde_json::from_slice(
        &fs::read(temp.path().join("stacks/default/run/state.json")).expect("runtime state"),
    )
    .expect("runtime json");
    assert_eq!(
        state["runner_port_range"]["start"],
        occupied_runner_port + 100
    );
}

#[test]
fn status_reports_degraded_when_health_is_not_200() {
    if !python3_available() {
        return;
    }

    let temp = setup_fake_docker();
    let stack = "healthcheck";
    let stack_config_dir = temp.path().join("stacks").join(stack).join("config");
    let health_status_file = temp.path().join("main-health-status.txt");
    fs::create_dir_all(&stack_config_dir).expect("stack config dir");
    fs::write(
        stack_config_dir.join("main.env"),
        format!(
            "ADDRESS=127.0.0.1\nPORT=5588\nRUNNER_ENDPOINTS=http://127.0.0.1:55880\nHEALTH_STATUS_FILE={}\n",
            health_status_file.display()
        ),
    )
    .expect("main env");

    let main_port = find_free_port();
    let runner_port = find_free_port();

    let mut up = cargo_bin();
    docker_env(&temp, &mut up);
    up.args([
        "up",
        "--context",
        stack,
        "--detach",
        "--main-address",
        "127.0.0.1",
        "-p",
        &main_port.to_string(),
        "--runner-address",
        "127.0.0.1",
        "-P",
        &format!("{runner_port}:{runner_port}"),
        "--runners",
        "1",
    ])
    .assert()
    .success();

    fs::write(&health_status_file, "204\n").expect("health status file");

    let mut status = cargo_bin();
    docker_env(&temp, &mut status);
    let status_output = status
        .args(["status", "--context", stack, "--json"])
        .output()
        .expect("status output");
    assert!(status_output.status.success());
    let status_json: serde_json::Value =
        serde_json::from_slice(&status_output.stdout).expect("status json");
    assert_eq!(status_json["state"], "degraded");
    assert_eq!(status_json["main"]["state"], "degraded");
    assert_eq!(status_json["runners"][0]["state"], "running");
}

#[test]
fn up_rejects_zero_ports_from_cli_and_compose() {
    let temp = setup_fake_docker();

    let mut main_port = cargo_bin();
    docker_env(&temp, &mut main_port);
    main_port
        .args(["up", "--dry-run", "--main-port", "0"])
        .assert()
        .failure();

    let mut runner_port = cargo_bin();
    docker_env(&temp, &mut runner_port);
    runner_port
        .args(["up", "--dry-run", "--runner-port-range", "0:56000"])
        .assert()
        .failure();

    let main_port_zero = temp.path().join("compose-main-port-zero.yaml");
    fs::write(
        &main_port_zero,
        r#"version: 1
main:
  port: 0
runners:
  local:
    count: 1
"#,
    )
    .expect("compose main port zero");

    let mut main_compose = cargo_bin();
    docker_env(&temp, &mut main_compose);
    main_compose
        .args([
            "up",
            "--dry-run",
            main_port_zero.to_str().expect("compose path"),
        ])
        .assert()
        .failure();

    let runner_port_zero = temp.path().join("compose-runner-port-zero.yaml");
    fs::write(
        &runner_port_zero,
        r#"version: 1
runners:
  local:
    count: 1
    port_range:
      start: 0
      end: 56000
"#,
    )
    .expect("compose runner port zero");

    let mut runner_compose = cargo_bin();
    docker_env(&temp, &mut runner_compose);
    runner_compose
        .args([
            "up",
            "--dry-run",
            runner_port_zero.to_str().expect("compose path"),
        ])
        .assert()
        .failure();
}

#[test]
fn up_leaves_no_runtime_state_when_compose_startup_fails() {
    if !python3_available() {
        return;
    }

    let temp = setup_fake_docker();
    let stack = "rollback";
    let main_port = find_free_port();
    let runner_port = find_free_port();
    let failing_runner_port = runner_port + 1;
    let stack_config_dir = temp.path().join("stacks").join(stack).join("config");
    fs::create_dir_all(&stack_config_dir).expect("stack config dir");
    fs::write(
        stack_config_dir.join("runner.env"),
        format!("FAIL_PORT={failing_runner_port}\n"),
    )
    .expect("runner env");

    let mut up = cargo_bin();
    docker_env(&temp, &mut up);
    let output = up
        .args([
            "up",
            "--context",
            stack,
            "--detach",
            "--main-address",
            "127.0.0.1",
            "-p",
            &main_port.to_string(),
            "--runner-address",
            "127.0.0.1",
            "-P",
            &format!("{runner_port}:{failing_runner_port}"),
            "--runners",
            "2",
        ])
        .output()
        .expect("up output");

    assert!(!output.status.success());
    assert!(
        !temp
            .path()
            .join("stacks")
            .join(stack)
            .join("run/state.json")
            .exists()
    );
}

#[test]
fn open_launches_local_main_app_url() {
    let temp = setup_fake_docker();
    let stack = "other";
    let stack_dir = temp.path().join("stacks").join(stack);
    let run_dir = stack_dir.join("run");
    fs::create_dir_all(&run_dir).expect("run dir");
    let browser = temp.path().join("capture-browser.sh");
    let capture = temp.path().join("opened-url.txt");
    write_browser_capture_script(&browser);

    fs::write(
        run_dir.join("state.json"),
        format!(
            r#"{{
  "name": "{stack}",
  "mode": "detached",
  "started_at": "2026-03-11T00:00:00Z",
  "image_tag": "latest",
  "compose_file": "{}",
  "compose_project": "previa_{stack}",
  "main": {{
    "service_name": "main",
    "address": "0.0.0.0",
    "port": 5588
  }},
  "runner_port_range": {{
    "start": 55880,
    "end": 55979
  }},
  "attached_runners": [],
  "runners": []
}}"#,
            stack_dir
                .join("run")
                .join("docker-compose.generated.yaml")
                .display()
        ),
    )
    .expect("runtime state");

    let mut command = cargo_bin();
    docker_env(&temp, &mut command);
    let output = command
        .env("PREVIA_OPEN_BROWSER", &browser)
        .env("PREVIA_OPEN_CAPTURE", &capture)
        .args(["open", "--context", stack])
        .output()
        .expect("open output");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
    let opened = fs::read_to_string(&capture).expect("captured URL");
    let expected = "http://127.0.0.1:5588";
    assert_eq!(opened, expected);
    assert_eq!(stdout.trim(), expected);
}

#[test]
fn open_prints_url_when_browser_launch_fails() {
    let temp = setup_fake_docker();
    let stack = "other";
    let stack_dir = temp.path().join("stacks").join(stack);
    let run_dir = stack_dir.join("run");
    fs::create_dir_all(&run_dir).expect("run dir");

    fs::write(
        run_dir.join("state.json"),
        format!(
            r#"{{
  "name": "{stack}",
  "mode": "detached",
  "started_at": "2026-03-11T00:00:00Z",
  "image_tag": "latest",
  "compose_file": "{}",
  "compose_project": "previa_{stack}",
  "main": {{
    "service_name": "main",
    "address": "0.0.0.0",
    "port": 5588
  }},
  "runner_port_range": {{
    "start": 55880,
    "end": 55979
  }},
  "attached_runners": [],
  "runners": []
}}"#,
            stack_dir
                .join("run")
                .join("docker-compose.generated.yaml")
                .display()
        ),
    )
    .expect("runtime state");

    let mut command = cargo_bin();
    docker_env(&temp, &mut command);
    let output = command
        .env("PREVIA_OPEN_BROWSER", temp.path().join("missing-browser"))
        .args(["open", "--context", stack])
        .output()
        .expect("open output");

    assert!(!output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
    let stderr = String::from_utf8(output.stderr).expect("utf8 stderr");
    let expected = "http://127.0.0.1:5588";

    assert_eq!(stdout.trim(), expected);
    assert!(stderr.contains("\u{1b}[31m"));
    assert!(stderr.contains("failed to open the browser automatically"));
    assert!(stderr.contains("failed to launch browser command"));
    assert!(stderr.contains("open the URL above manually"));
}

fn run_command_with_stdin<const N: usize>(
    previa_home: &Path,
    args: [&str; N],
    stdin_input: &str,
) -> std::process::Output {
    let mut command = cargo_bin();
    command
        .env("PREVIA_HOME", previa_home)
        .env("PATH", prepend_path(&previa_home.join("docker-bin")))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .args(args);

    let mut child = command.spawn().expect("spawn command");
    child
        .stdin
        .as_mut()
        .expect("stdin pipe")
        .write_all(stdin_input.as_bytes())
        .expect("write stdin");
    child.wait_with_output().expect("wait with output")
}
