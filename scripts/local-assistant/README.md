# `scripts/local-assistant/`: local assistant measurement and soak harness

Measures the memory behaviour of the managed MLX worker and the `local_assistant`
task loop on macOS, and gives a leak-versus-retention verdict per process and
per worker state. Node built-ins only; no new npm dependencies.

| File | Role |
| --- | --- |
| `la.sh` | JSON-RPC wrapper: `start-task`, `status`, `list`, `pause`, `resume`, `cancel`, `worker-status`, `metrics`, `stop-worker`, `raw` |
| `sampler-macos.mjs` | 1 Hz sampler: `footprint -p` (phys_footprint), `ps` RSS, threads, fds, `sysctl` pressure/available %, `vm_stat`, `ioreg` IOAccelerator, worker `/health` + `/metrics`, Ollama `/api/ps`, `mlx.worker_status` |
| `soak.mjs` | Driver: starts its own isolated core, runs phases A to F, writes the report |
| `analyze-cycles.mjs` | Cycle analyzer and verdicts (also usable on a recorded run) |
| `analyze-cycles.test.mjs` | `node --test scripts/local-assistant/analyze-cycles.test.mjs` |

## Starting and stopping the assistant (not the soak)

The running core is the controller (the desktop app, or `neppy-core serve`).
The worker starts lazily on the first task and stops itself when idle.

```bash
export OPENHUMAN_CORE_PORT=7788          # default
export OPENHUMAN_CORE_TOKEN=...          # or scripts/print-core-token.sh finds the token file
scripts/local-assistant/la.sh start-task /path/to/project "goal" --edits --test "cargo test -q" --max-steps 8
scripts/local-assistant/la.sh status <task_id>
scripts/local-assistant/la.sh pause          # checkpoint, then stop the worker
scripts/local-assistant/la.sh resume         # re-queue paused tasks (or: resume <task_id>)
scripts/local-assistant/la.sh stop-worker    # force-stop; a running task becomes interrupted and resumes later
scripts/local-assistant/la.sh worker-status  # pressure state, worker pid + footprint, gate
scripts/local-assistant/la.sh metrics 0 200 --events
```

## Running the soak

```bash
GGML_NATIVE=OFF cargo build --release --bin neppy-core --no-default-features \
  --features "$(bash scripts/ci/product-features.sh)"

# dry run: 2 cycles, short timers, kill test in cycle 2
node scripts/local-assistant/soak.mjs --scratch <dir> --phases A,C --a-secs 20 --cycles 2 \
  --idle-secs 45 --unload-secs 15 --stop-secs 30 --kill-cycle 2 --max-steps 3

# full protocol (about 2 hours with the defaults)
node scripts/local-assistant/soak.mjs --scratch <dir>
```

`--scratch` must be a disk-backed directory outside the repo. It holds `home/`
(HOME for the core), `ws/` (its `OPENHUMAN_WORKSPACE`), `neppy-soak/` (a
`git clone --local` of `--source-repo`, reset between cycles) and
`runs/run-<ts>/`.

### Isolation

- A separate `neppy-core serve` on port 17790 with its own workspace, HOME,
  file keyring (`OPENHUMAN_KEYRING_BACKEND=file`) and a locally generated test
  session token. `~/.neppy` is never read or written.
- MLX worker on port 18764 (not the app's 64744). `HF_HUB_OFFLINE=1`, so nothing
  downloads. Module downloads and the update check are disabled in the soak config.
- Only processes whose command line carries the soak's worker port are ever
  killed by the harness. A guard aborts the run if available memory stays below
  12% for 5 samples.
- Ctrl-C or a fatal error stops the worker and core and verifies no worker
  survives.

### Phases

| Phase | What | Idle policy |
| --- | --- | --- |
| A | worker stopped, 60 s baseline | always |
| B | one prompt each at about 2k, 8k and 14k tokens straight to the worker; footprint step compared with the 32 KiB/token KV prediction | pressure_only |
| C | N cycles of: reset clone, run the representative task, idle 150 s (loaded, then unloaded, then stopped); `kill -9` of the worker mid-generation in `--kill-cycle` | always, unload 45 s, stop 120 s |
| D | N cycles with the model resident, then a release probe: `/unload`, stop, respawn | pressure_only |
| F | the fallback model, same cycle shape as C | always |

The representative task edits the disposable clone only: find every MLX server
spawn/stop/reclaim path and add a `// NOTE(soak):` comment above each spawn call
site. It exercises the index over about 13,000 files, a multi-step plan, edits
and (when the model asks for it) the test command.

## Reading the report

`runs/run-<ts>/report.md` has the setup, per-phase peaks (worker and core
footprint and RSS, system used, minimum available %, maximum pressure level, GPU
counters, swap), a verdict per series, per-cycle deltas against cycle 2, the kill
test, and the cycle outcomes. `samples.jsonl` and `marks.jsonl` are the raw
inputs; re-analyze with
`node analyze-cycles.mjs --samples s.jsonl --marks m.jsonl --md report.md`.

Cycle 1 of each phase is warm-up and excluded. Verdicts:

- `no-growth`: growth over the run is within noise (3x MAD of cycle 2, with a floor).
- `plateau`: grew, then levelled off (what a cache or allocator arena looks like).
- `growth-below-threshold`: still rising but under the per-cycle leak threshold.
  This is reported, not waved away, and blocks a "no leak" statement.
- `retained-by-runtime`: still rising, but an explicit `/unload` brought the
  worker back within 5% of its unloaded baseline, so the memory was a cache.
- `retained-by-os`: system used or file cache grew while process footprints did not.
- `confounded`: core growth that tracks workspace size on disk, or system growth
  alongside a process-level leak call.
- `leak-suspected`: still rising over threshold with no such explanation. The
  note says whether stopping the process freed it.
- `insufficient-data`: fewer than 4 cycles after warm-up.

Thresholds: core 2 MiB/cycle, worker 64 MiB/cycle (32 unloaded), system 128
MiB/cycle; threads and fds use a monotonic-rise rule. The no-leak statement is
only offered for a phase with at least 10 cycles whose every core and worker
series is `no-growth` or `plateau`, and it is scoped to the run.

## Findings that shaped the code (2026-10-01 run, mlx_vlm 0.7.0, 9B 8-bit)

- `/unload` returns the server's model registry and MLX's buffer cache, not the
  weights: worker `phys_footprint` went 9.9 GiB (loaded) to 9.4 GiB (after
  unload), `footprint` still showed 9.1 GiB of IOAccelerator memory, and system
  free memory did not move (4.0 GiB unloaded vs 3.6 GiB loaded, 13.3 GiB once the
  process stopped). The watchdog now stops the process when an unload frees less
  than half of the footprint (`unload_ineffective` event).
- mlx_vlm ignores `chat_template_kwargs`; it reads a top-level `enable_thinking`.
  The assistant now sends both. Against a server started with `--enable-thinking`
  the old request produced an empty reply (all 1536 tokens spent on reasoning).
- Phase B sizes its prompt with a 3.85 chars/token factor. The recorded run used
  3.0 and so reached 1.6k, 6.2k and 10.9k tokens rather than 2k, 8k and 14k.

## Limits

- `phys_footprint` is the best sudo-free per-process number. It includes Metal
  allocations (the `IOAccelerator` category), which RSS does not. The ioreg
  "In use system memory" counter did not move with the model on the test machine;
  "Alloc system memory" is reported as well.
- Available % is the kernel's own estimate. The machine is shared with whatever
  else the user is running, so system-level series are noisy.
- Phase B calls the worker directly and so bypasses the single-flight gate.
- The 9B model chooses whether to request the test command. When no step does,
  the "test ran once per step" kill criterion is reported as not exercised.
- A verdict covers the tested cycles and workload only.
