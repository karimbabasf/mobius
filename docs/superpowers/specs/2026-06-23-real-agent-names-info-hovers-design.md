# Real Agent Names And Info Hovers Design

## Goal

Mobius must display the provider's real chat/session title as the agent name. When that title is unavailable, the interface must avoid inventing a folder-based name and instead show a neutral provider plus session id identity.

## Decisions

- A real agent name is a provider-sourced title: Claude `aiTitle`/summary, Codex `thread_name`, or Hermes `sessions.title`.
- Fallback titles must not masquerade as names. Backend fallback sessions should carry `title: null` where possible, and the frontend should render `Provider · session-id`.
- Rename controls appear only when Mobius can write back to the provider's real title source. Current writable sources are Claude `aiTitle` and Codex `thread_name`.
- The expanded card should make title provenance explicit: real provider title when present, provider title unavailable when absent.
- Every expanded card section needs a small circular info affordance that opens explanatory help on hover and keyboard focus.

## UI Shape

The collapsed card keeps provider, status, current action, context, project, uptime, and model. The displayed name becomes either the real provider title or a neutral `Provider · session-id` string.

The expanded card sections are:

- Session: real provider title, rename control when writable, provenance, project, branch, model, process, and id.
- Run: autonomous run telemetry when available.
- Process: scanned process tree and stop control when available.
- Capacity: token counts and context-window detail.
- Activity: live file and command activity.

Each section header includes an `i` tooltip with short operational guidance.

## Data Flow

Adapters emit `AgentSession.title` only when a provider title is known. The collector may still enrich Codex sessions by joining `session_index.jsonl`. The frontend derives display identity from `titleSource` and `title`, not from `projectPath`.

## Error Handling

Rename continues to reject empty titles and unknown sessions. Read-only or missing-title sessions show explanatory text instead of a form. Tooltips are CSS-only and do not affect polling or card reconciliation.

## Testing

- Rust adapter tests verify fallback sessions no longer emit project/folder names as titles.
- Frontend card tests verify real names, provider/session-id fallback display, read-only rename states, and info tooltip content.
- Existing rename and process/activity tests continue to pass.
