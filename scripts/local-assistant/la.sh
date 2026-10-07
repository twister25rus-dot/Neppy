#!/usr/bin/env bash
#
# la.sh — thin JSON-RPC wrapper for the local assistant and the MLX worker.
#
# Talks to a running core (`neppy-core serve`, or the desktop app's embedded
# core) over http://127.0.0.1:<port>/rpc with the core's bearer token.
#
#   NEPPY_CORE_PORT   core port (default 7788)
#   NEPPY_CORE_TOKEN  bearer; when unset, scripts/print-core-token.sh is
#                         used (reads $NEPPY_WORKSPACE/core.token)
#
# Usage:
#   la.sh start-task <project_root> "<goal>" [--edits] [--test "<cmd>"] [--max-steps N]
#   la.sh status <task_id>
#   la.sh list [limit]
#   la.sh pause                  # set_enabled false: checkpoint, stop the worker
#   la.sh resume [task_id]       # no id: set_enabled true (re-queues paused tasks)
#   la.sh cancel <task_id>
#   la.sh worker-status          # pressure state, worker pid + footprint, gate
#   la.sh metrics [since_ms] [limit] [--events]
#   la.sh stop-worker [server_id]   # force-stop (default server id: primary)
#   la.sh raw <method> '<json params>'
#
# Output is the JSON-RPC `result` (jq-formatted when jq is installed).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PORT="${NEPPY_CORE_PORT:-7788}"
URL="http://127.0.0.1:${PORT}/rpc"

token() {
  if [[ -n "${NEPPY_CORE_TOKEN:-}" ]]; then
    printf '%s' "$NEPPY_CORE_TOKEN"
  else
    bash "$SCRIPT_DIR/../print-core-token.sh"
  fi
}

# rpc <method-without-prefix> <params-json>
rpc() {
  local method="$1" params="$2" body reply
  body=$(node -e '
    const [m, p] = process.argv.slice(1);
    process.stdout.write(JSON.stringify({ jsonrpc: "2.0", id: 1, method: m, params: JSON.parse(p) }));
  ' "$method" "$params")
  reply=$(curl -sS --fail-with-body -X POST "$URL" \
    -H "content-type: application/json" \
    -H "authorization: Bearer $(token)" \
    --data-binary "$body") || { echo "la.sh: request to $URL failed: $reply" >&2; exit 1; }
  # Surface a JSON-RPC error as a non-zero exit; unwrap the {result, logs} envelope.
  node -e '
    const r = JSON.parse(process.argv[1]);
    if (r.error) { console.error("rpc error " + r.error.code + ": " + r.error.message); process.exit(1); }
    let v = r.result;
    if (v && typeof v === "object" && "result" in v && "logs" in v) v = v.result;
    console.log(JSON.stringify(v));
  ' "$reply" | { if command -v jq >/dev/null 2>&1; then jq .; else cat; fi; }
}

usage() { sed -n '2,/^# Output is/p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; }

cmd="${1:-}"
[[ -n "$cmd" ]] && shift || true

case "$cmd" in
  start-task)
    root="${1:?project_root required}"; goal="${2:?goal required}"; shift 2
    edits=false; test_cmd=""; max_steps=""
    while [[ $# -gt 0 ]]; do
      case "$1" in
        --edits) edits=true; shift ;;
        --test) test_cmd="${2:?--test needs a command}"; shift 2 ;;
        --max-steps) max_steps="${2:?--max-steps needs a number}"; shift 2 ;;
        *) echo "la.sh: unknown option $1" >&2; exit 2 ;;
      esac
    done
    params=$(node -e '
      const [root, goal, edits, test, steps] = process.argv.slice(1);
      const p = { project_root: root, goal, allow_edits: edits === "true" };
      if (test) p.test_command = test;
      if (steps) p.max_steps = Number(steps);
      process.stdout.write(JSON.stringify(p));
    ' "$root" "$goal" "$edits" "$test_cmd" "$max_steps")
    rpc neppy.local_assistant_start_task "$params" ;;
  status)      rpc neppy.local_assistant_status "{\"task_id\":\"${1:?task_id required}\"}" ;;
  list)        rpc neppy.local_assistant_list "{\"limit\":${1:-20}}" ;;
  pause)       rpc neppy.local_assistant_set_enabled '{"enabled":false}' ;;
  resume)
    if [[ -n "${1:-}" ]]; then rpc neppy.local_assistant_resume "{\"task_id\":\"$1\"}"
    else rpc neppy.local_assistant_set_enabled '{"enabled":true}'; fi ;;
  cancel)      rpc neppy.local_assistant_cancel "{\"task_id\":\"${1:?task_id required}\"}" ;;
  worker-status) rpc neppy.mlx_worker_status '{}' ;;
  metrics)
    since="${1:-0}"; limit="${2:-200}"; events=false
    [[ "${3:-}" == "--events" ]] && events=true
    rpc neppy.mlx_worker_metrics "{\"since_ms\":$since,\"limit\":$limit,\"events_only\":$events}" ;;
  stop-worker) rpc neppy.mlx_stop "{\"id\":\"${1:-primary}\"}" ;;
  raw)         method="${1:?method required}"; params="${2:-}"; [[ -z "$params" ]] && params='{}'; rpc "$method" "$params" ;;
  ""|-h|--help|help) usage ;;
  *) echo "la.sh: unknown command '$cmd'" >&2; usage >&2; exit 2 ;;
esac
