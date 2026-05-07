#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MAIN_PORT="${MAIN_PORT:-5610}"
TARGET_PORT="${TARGET_PORT:-5620}"
DB_URL="${ORCHESTRATOR_DATABASE_URL:-sqlite:///private/tmp/previa-local-load-target.db}"
SCREEN_NAME="${SCREEN_NAME:-previa-local-load-target}"

cd "$ROOT"

npm --prefix app run build
cargo build --release

screen -S "$SCREEN_NAME" -X quit >/dev/null 2>&1 || true
screen -S previa-wave -X quit >/dev/null 2>&1 || true
for port in "$MAIN_PORT" 5611 5612 5613 "$TARGET_PORT"; do
  pids="$(lsof -tiTCP:"$port" -sTCP:LISTEN 2>/dev/null || true)"
  if [ -n "$pids" ]; then
    kill $pids 2>/dev/null || true
  fi
done

screen -dmS "$SCREEN_NAME" zsh -lc "
  cd '$ROOT'
  RUST_LOG=info PORT=$TARGET_PORT target/release/previa-load-target > /tmp/previa-load-target-$TARGET_PORT.log 2>&1 &
  RUST_LOG=info PORT=5611 target/release/previa-runner > /tmp/previa-runner-5611.log 2>&1 &
  RUST_LOG=info PORT=5612 target/release/previa-runner > /tmp/previa-runner-5612.log 2>&1 &
  RUST_LOG=info PORT=5613 target/release/previa-runner > /tmp/previa-runner-5613.log 2>&1 &
  RUST_LOG=info PREVIA_APP_ENABLED=1 ORCHESTRATOR_DATABASE_URL='$DB_URL' PORT=$MAIN_PORT RUNNER_ENDPOINTS=http://127.0.0.1:5611,http://127.0.0.1:5612,http://127.0.0.1:5613 target/release/previa-main > /tmp/previa-main-$MAIN_PORT.log 2>&1
"

for url in "http://127.0.0.1:$TARGET_PORT/health" "http://127.0.0.1:$MAIN_PORT/info"; do
  for attempt in $(seq 1 40); do
    if curl -fsS "$url" >/dev/null 2>&1; then
      break
    fi
    if [ "$attempt" -eq 40 ]; then
      echo "Service did not become ready: $url" >&2
      exit 1
    fi
    sleep 0.25
  done
done

PROJECT_ID="$(
  curl -fsS -X POST "http://127.0.0.1:$MAIN_PORT/api/v1/projects" \
    -H 'content-type: application/json' \
    -d '{"name":"Local Load Target Reference","description":"Projeto local para validar wave open-loop contra API deterministica.","pipelines":[]}' \
    | jq -r '.id'
)"

curl -fsS -X POST "http://127.0.0.1:$MAIN_PORT/api/v1/projects/$PROJECT_ID/env-groups" \
  -H 'content-type: application/json' \
  -d "{\"slug\":\"local\",\"name\":\"Local\",\"entries\":[{\"name\":\"api\",\"url\":\"http://127.0.0.1:$TARGET_PORT\",\"description\":\"Local deterministic load target\"}]}" >/dev/null

PIPELINE_PAYLOAD="$(
  ruby -rjson -ryaml -e 'data = YAML.load_file("fixtures/local-load-target.previa.yaml"); data.delete("id"); puts JSON.generate(data)'
)"

PIPELINE_ID="$(
  curl -fsS -X POST "http://127.0.0.1:$MAIN_PORT/api/v1/projects/$PROJECT_ID/pipelines" \
    -H 'content-type: application/json' \
    -d "$PIPELINE_PAYLOAD" \
    | jq -r '.id'
)"

curl -fsS "http://127.0.0.1:$TARGET_PORT/metrics/reset" >/dev/null

echo "Main:       http://127.0.0.1:$MAIN_PORT"
echo "Target:     http://127.0.0.1:$TARGET_PORT"
echo "Metrics:    http://127.0.0.1:$TARGET_PORT/metrics"
echo "Load test:  http://127.0.0.1:$MAIN_PORT/projects/$PROJECT_ID/pipeline/$PIPELINE_ID/load-test"
