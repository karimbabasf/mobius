import { invoke } from "@tauri-apps/api/core";
import type { AgentSession } from "./types";

declare global {
  interface Window {
    __TAURI_INTERNALS__?: unknown;
  }
}

function isTauriRuntime(): boolean {
  return typeof window !== "undefined" && window.__TAURI_INTERNALS__ != null;
}

export async function getSessions(): Promise<AgentSession[]> {
  if (!isTauriRuntime()) {
    return [];
  }
  return invoke<AgentSession[]>("get_sessions");
}

export async function renameSession(sessionId: string, newTitle: string): Promise<void> {
  if (!isTauriRuntime()) {
    throw new Error("Rename is only available in the desktop app.");
  }
  await invoke("rename_session", { sessionId, newTitle });
}

export async function killProcesses(pids: number[]): Promise<void> {
  if (!isTauriRuntime()) {
    throw new Error("Stopping processes is only available in the desktop app.");
  }
  await invoke("kill_processes", { pids });
}
