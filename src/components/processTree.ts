import type { ProcessNode } from "../types";
import { escapeHtml } from "./format";

/** Every node in the subtree, root-first depth-first. */
export function flattenNodes(node: ProcessNode): ProcessNode[] {
  const nodes = [node];
  for (const child of node.children) {
    nodes.push(...flattenNodes(child));
  }
  return nodes;
}

/** Every pid in the subtree, root-first depth-first — the kill list. */
export function flattenPids(node: ProcessNode): number[] {
  return flattenNodes(node).map((n) => n.pid);
}

export interface KillPlan {
  pids: number[];
  /** Confirmation text spelling out exactly what will be signalled. */
  text: string;
}

/** The exact set of processes a Stop action would signal, for confirmation. */
export function killPlan(node: ProcessNode): KillPlan {
  const nodes = flattenNodes(node);
  const noun = nodes.length === 1 ? "process" : "processes";
  const lines = nodes.map((n) => `  ${n.pid}  ${n.command}`).join("\n");
  return {
    pids: nodes.map((n) => n.pid),
    text: `This will send SIGTERM to ${nodes.length} ${noun}:\n\n${lines}`,
  };
}

/** Short label for a process: the executable basename, not the whole argv. */
function processLabel(command: string): string {
  const exe = command.split(/\s+/)[0] ?? command;
  return exe.split("/").pop() || exe;
}

function renderNode(node: ProcessNode): string {
  const childList = node.children.length
    ? `<ul class="process-tree__children">${node.children.map(renderNode).join("")}</ul>`
    : "";
  return `
    <li class="process-tree__node">
      <span class="process-tree__row">
        <span class="process-tree__pid">${node.pid}</span>
        <span class="process-tree__name">${escapeHtml(processLabel(node.command))}</span>
        <span class="process-tree__cmd">${escapeHtml(node.command)}</span>
      </span>
      ${childList}
    </li>
  `;
}

/** A collapsible nested list of a process subtree. */
export function renderProcessTree(node: ProcessNode): string {
  return `<ul class="process-tree">${renderNode(node)}</ul>`;
}
