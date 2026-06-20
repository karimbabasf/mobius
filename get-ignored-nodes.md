# get-ignored-nodes

Deferred feature "nodes" — things we deliberately chose **not** to build into the
current version, parked here so they aren't lost. Each entry is a candidate for a
future branch.

> Convention: keep these scoped and actionable. When a node is picked up, move it
> out of this file and into a real ticket/branch.

---

## History per profile, per section
Persist session lifecycle to the already-provisioned `state.sqlite`
(`~/Library/Application Support/AgentMissionControl/state.sqlite`) so terminated
agents and their activity survive an app restart. Browsable **per profile**
(e.g. `~/.codex` vs `~/.codex-karim`) and **per section** (per tool, per project),
giving a real "past sessions" timeline instead of the current in-memory-only view
where a terminated agent disappears for good.

- Today: in-memory only. A session vanishes the moment its process dies; nothing
  is recorded.
- Needs: a lightweight schema (sessions + events), a write path in the collector
  on first-seen / on-terminate, and a history view in the UI.

## Push-based termination (kqueue `EVFILT_PROC` / `NOTE_EXIT`)
We currently confirm liveness by polling `kill(pid, 0)` every ~1.5s. macOS can
notify us the instant a watched PID exits via kqueue `EVFILT_PROC` with
`NOTE_EXIT` (Rust `kqueue` crate). Would make terminations feel instantaneous and
cut polling overhead. Combine with the existing PID discovery in
`src-tauri/src/collector/liveness.rs`.

## Stale-registry / PID-reuse hardening
`liveness.rs` matches Claude sessions on `sessionId`, which already neutralises
the common PID-reuse case. The remaining theoretical edge: a crashed Claude
leaves an un-cleaned `~/.claude/sessions/<PID>.json`, that PID gets reused by an
unrelated process, and `kill(pid, 0)` passes. Optional hardening: cross-check the
process name (via `sysinfo`/`libproc`) or the registry `startedAt` against the
process start time before trusting the entry.

## Cursor adapter
`Tool::Cursor` exists in the session model and the UI themes for it (green), but
there is no adapter parsing Cursor's local session data, and no liveness path for
it. Add `src-tauri/src/collector/adapters/cursor.rs` plus a Cursor liveness
signal once we know where/how Cursor records sessions on disk.

## Codex liveness for non-desktop variants
Codex liveness relies on the process holding its rollout `.jsonl` open
(`lsof -c codex`). Confirmed for Codex Desktop; other Codex front-ends (e.g. the
VS Code extension binary) may not hold the file open, which would hide an
otherwise-live session. Investigate a fallback (process-args/cwd correlation) if
this surfaces in practice.
