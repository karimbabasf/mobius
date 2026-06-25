# Real Agent Names And Info Hovers Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Show only provider-sourced agent names, use provider plus session id when unavailable, and add compact explanatory info hovers to every expanded card section.

**Architecture:** Keep the backend contract honest by emitting `title: null` for fallback names. Keep frontend display logic local to `agentCard.ts`, with a reusable tooltip renderer and CSS-only hover/focus behavior.

**Tech Stack:** Rust/Tauri collector, TypeScript/Vite frontend, Vitest/jsdom tests, Cargo tests.

## Global Constraints

- No `git push`, `gh pr create`, or MR/PR creation without Karim explicitly saying to push or open the PR in the current message.
- Rename is allowed only when Mobius can write back to a provider real-title source.
- Fallback project/folder names must not be shown as agent names.
- Tooltips must work on hover and keyboard focus.

---

### Task 1: Backend Title Contract

**Files:**
- Modify: `src-tauri/src/collector/adapters/claude.rs`
- Modify: `src-tauri/src/collector/adapters/codex.rs`
- Modify: `src-tauri/src/collector/mod.rs`

**Interfaces:**
- Consumes: `AgentSession.title: Option<String>`, `TitleSource`
- Produces: fallback sessions with `title: None`, provider sessions with `title: Some(real_name)`

- [ ] **Step 1: Write the failing tests**

Update Claude/Codex/collector assertions so provider-less sessions expect `title.is_none()` and provider title sessions still expect a real title.

- [ ] **Step 2: Run tests to verify failure**

Run: `cd src-tauri && cargo test collector::adapters::claude collector::adapters::codex collector::tests::`

Expected: failures where fallback code still returns project or binary names.

- [ ] **Step 3: Implement minimal backend change**

Change Claude title derivation to return `(Option<String>, TitleSource)`, Codex default title to `None`, and synthesized untracked process title to `None`.

- [ ] **Step 4: Verify backend tests pass**

Run: `cd src-tauri && cargo test`

Expected: all Rust tests pass.

### Task 2: Frontend Real-Name Rendering

**Files:**
- Modify: `src/components/agentCard.ts`
- Modify: `src/components/agentCard.test.ts`
- Modify: `src/main.ts`

**Interfaces:**
- Consumes: `AgentSession.title`, `AgentSession.titleSource`, `AgentSession.canRename`
- Produces: `displayName`, `providerTitle`, and neutral fallback text in rendered card HTML

- [ ] **Step 1: Write the failing tests**

Add tests that a fallback card renders `Claude · session-id`, does not render the project folder as the name, and labels the provider title as unavailable.

- [ ] **Step 2: Run tests to verify failure**

Run: `npm test -- src/components/agentCard.test.ts src/main.test.ts`

Expected: new fallback-display test fails.

- [ ] **Step 3: Implement minimal frontend rendering**

Add helpers in `agentCard.ts` for `providerTitle`, `displayName`, and title status. Use them in the collapsed name and expanded rename/session panel. After successful rename in `main.ts`, keep `titleSource` as `provider`.

- [ ] **Step 4: Verify frontend tests pass**

Run: `npm test -- src/components/agentCard.test.ts src/main.test.ts`

Expected: all selected frontend tests pass.

### Task 3: Info Hovers

**Files:**
- Modify: `src/components/agentCard.ts`
- Modify: `src/components/agentCard.test.ts`
- Modify: `src/styles.css`

**Interfaces:**
- Produces: reusable `renderInfoTip(label, body)` markup
- Produces: `.panel-heading` and `.info-tip` styles

- [ ] **Step 1: Write the failing tests**

Add tests that expanded cards include tooltip text for session, run, process, capacity, and activity sections when those sections render.

- [ ] **Step 2: Run tests to verify failure**

Run: `npm test -- src/components/agentCard.test.ts`

Expected: tooltip-content test fails.

- [ ] **Step 3: Implement tooltip renderer and CSS**

Use a focusable circular `i` element in every panel heading. Add CSS for stable placement, hover/focus reveal, and mobile-safe wrapping.

- [ ] **Step 4: Verify frontend tests pass**

Run: `npm test -- src/components/agentCard.test.ts`

Expected: card tests pass.

### Task 4: Docs And Full Verification

**Files:**
- Modify: `docs/DECISIONS.md`
- Modify: `docs/ARCHITECTURE.md`

**Interfaces:**
- Produces: documented source-of-truth naming rule and info-hover UI note

- [ ] **Step 1: Update docs**

Record that provider titles are the only real names, fallback UI uses provider plus session id, and info hovers explain expanded-card sections.

- [ ] **Step 2: Run full verification**

Run: `npm test`, `npm run build`, and `cd src-tauri && cargo test`.

Expected: all commands pass.
