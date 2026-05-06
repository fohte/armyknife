# Logging

Structured JSONL logs for debugging armyknife internals — primarily the
`cc auto-compact` and `cc sweep` workflows that misbehave silently when they
fail.

## Where logs go

```
~/.cache/armyknife/logs/armyknife.log.YYYY-MM-DD
```

- Daily rotation; the latest 7 files are retained, older files are removed
  by `tracing-appender` itself
- Format: one JSON object per line (JSONL)
- Init failures (no home dir, unwritable cache, etc.) are silently swallowed
  so that a CLI binary never dies because its log can't be opened

## Controlling the level

`ARMYKNIFE_LOG` accepts:

| value   | effect                     |
| ------- | -------------------------- |
| `off`   | No logs are written        |
| `error` | Only warnings/errors       |
| `info`  | Lifecycle events (default) |
| `debug` | Adds verbose internals     |

The variable also accepts `tracing-subscriber` directives like
`armyknife=debug,info` for module-level overrides.

## Anatomy of a log line

```json
{
  "timestamp": "2026-05-06T09:12:26.303328Z",
  "level": "INFO",
  "event": "cc.sweep.start",
  "timeout": "30m",
  "dry_run": true,
  "target": "a::commands::cc::sweep",
  "span": { "run_id": "6ac23df6", "name": "cc.sweep" }
}
```

Top-level fields:

- `timestamp` — UTC, RFC 3339
- `level` — `INFO` / `WARN` / `ERROR`
- `event` — fully-qualified `<area>.<verb>` (e.g. `cc.auto_compact.schedule.armed`),
  the primary key for filtering
- `target` — Rust module path the event was emitted from
- `span.run_id` — short hex id that groups every event from a single
  invocation (one sweep pass, one schedule worker, one hook call)
- `span.name` — area identifier matching the `event` prefix
  (`cc.sweep` / `cc.auto_compact.schedule` / `cc.hook`)

Other fields are event-specific. `session` is present whenever an event
relates to a single Claude Code session.

## Querying with jq

```bash
LOG=~/.cache/armyknife/logs/armyknife.log.$(date +%F)

# All events from one sweep run
jq -c 'select(.span.run_id == "6ac23df6")' "$LOG"

# Lifecycle of a specific Claude Code session
jq -c 'select(.session == "ec2143e0-…")' "$LOG"

# Decision distribution across schedule workers
jq -r 'select(.event == "cc.auto_compact.schedule.decision") | .decision' "$LOG" \
  | sort | uniq -c

# Everything from the auto-compact subsystem
jq -c 'select(.event | startswith("cc.auto_compact"))' "$LOG"

# Tail and pretty-print live
tail -F "$LOG" | jq -c .
```

## Event reference

### `cc auto-compact`

Stop hook (parent process):

| event                          | meaning                                                      |
| ------------------------------ | ------------------------------------------------------------ |
| `cc.auto_compact.spawn`        | Stop hook is forking a `schedule` worker                     |
| `cc.auto_compact.spawn_failed` | `current_exe` resolution or `spawn_detached` failed          |
| `cc.auto_compact.skipped`      | Spawn skipped (`reason=disabled` / `reason=bg_task_pending`) |

`schedule` worker (detached child):

| event                                           | meaning                                                                                                                   |
| ----------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------- |
| `cc.auto_compact.schedule.start`                | Worker entered its run loop                                                                                               |
| `cc.auto_compact.schedule.cancelled_prev`       | Sent SIGTERM to an earlier worker for the same pane; `already_gone=true` when the prior worker had already exited (ESRCH) |
| `cc.auto_compact.schedule.cancel_prev_failed`   | SIGTERM to a prior worker failed (non-ESRCH)                                                                              |
| `cc.auto_compact.schedule.armed`                | About to sleep `idle_timeout_secs`                                                                                        |
| `cc.auto_compact.schedule.exit`                 | Returned early; `reason` is `disabled` / `session_missing_at_arm` / `preempted` / `session_missing_at_wake`               |
| `cc.auto_compact.schedule.inputs`               | Snapshot of every input fed to `decide_compact` (status, branch_merged, context_tokens, …)                                |
| `cc.auto_compact.schedule.decision`             | Final `CompactDecision` value                                                                                             |
| `cc.auto_compact.schedule.compact_executed`     | SIGTERMed `claude` and spawned `claude -r -p /compact`                                                                    |
| `cc.auto_compact.schedule.sigterm_failed`       | SIGTERM to the live `claude` process failed (non-ESRCH)                                                                   |
| `cc.auto_compact.schedule.compact_spawn_failed` | `claude -r -p /compact` spawn failed                                                                                      |

### `cc sweep`

| event                     | meaning                                           |
| ------------------------- | ------------------------------------------------- |
| `cc.sweep.start`          | One sweep pass is starting (`timeout`, `dry_run`) |
| `cc.sweep.paused`         | Session was just SIGTERMed and flipped to Paused  |
| `cc.sweep.dry_run_pause`  | Would have paused if not in `--dry-run`           |
| `cc.sweep.no_pid`         | Pause was decided but no live `claude` pid found  |
| `cc.sweep.sigterm_failed` | SIGTERM to the resolved pid failed (non-ESRCH)    |
| `cc.sweep.summary`        | End-of-pass counters (`scanned` / `paused` / …)   |

## Debugging recipes

### "Auto-compact is not firing"

1. Trigger a Stop hook (let `claude` finish a turn) and tail the log.
2. Walk the event chain:
   - No `cc.auto_compact.spawn` → the Stop hook never reached armyknife.
     Check `~/.claude/settings.json` and `ARMYKNIFE_SKIP_HOOKS`.
   - `cc.auto_compact.skipped reason=disabled` → enable it in `~/.config/armyknife`.
   - `cc.auto_compact.skipped reason=bg_task_pending` → expected; the next
     genuine Stop will consume the flag.
   - `cc.auto_compact.spawn` but no `cc.auto_compact.schedule.start` → the
     detached child died. Look for `cc.auto_compact.spawn_failed`.
   - `cc.auto_compact.schedule.armed` but no `cc.auto_compact.schedule.decision`
     after `idle_timeout_secs` → the worker was preempted;
     `cc.auto_compact.schedule.exit reason=preempted` should be present.
   - `cc.auto_compact.schedule.decision` other than `Compact` → look at the
     preceding `cc.auto_compact.schedule.inputs` event to see which input
     vetoed it (most often `context_tokens < min_context_tokens` for
     `BelowThreshold`).

### "Sweep should have paused this session"

```bash
a cc sweep --dry-run --timeout 1s   # forces every Stopped session to be a candidate
jq -c 'select(.session == "<id>")' ~/.cache/armyknife/logs/armyknife.log.$(date +%F)
```

`cc.sweep.no_pid` means the session has no live `claude` (probably already
exited); otherwise the session should appear in a `cc.sweep.dry_run_pause`.

## Adding new events

When wiring a new lifecycle path:

- Create a span at the entry point with
  `tracing::info_span!("<area>", run_id = %short_run_id(), session = %session_id)`
  and either `enter()` (sync) or `.instrument(span)` (async). Every event
  below it inherits `run_id` automatically — the grouping is what makes the
  JSONL useful.
- Use a fully-qualified `event = "<area>.<verb>"` string. The area should
  match the span name (`cc.sweep`, `cc.auto_compact.schedule`, …) so
  `jq 'select(.event | startswith("<area>"))'` and
  `jq 'select(.span.name == "<area>")'` produce the same set.
- Pass values as fields, not formatted strings: `pid = pid` rather than
  `format!("pid={pid}")`. Prefer `Display` (`%expr`) over `Debug` (`?expr`)
  for fields you want to filter on cleanly.
- One concern per event. If two pieces of information would be queried
  independently, they belong on separate events.
