import { api } from "@/lib/api";

/**
 * Web commands (`src/web/commands_api.rs`): actions the web accepts and the
 * worker carries out. The screens that send them (`다시 받기`, a rule's
 * `보관`·`복원`) share these.
 */

export type CommandState = "pending" | "running" | "done" | "failed";

export interface CommandOutcome {
  /** A code of the command's kind: for a retry (`receive_once`), a history result. */
  result: string;
  reason: string | null;
}

export interface Command {
  id: string;
  kind: string;
  state: CommandState;
  created_at: number;
  updated_at: number;
  finished_at: number | null;
  outcome: CommandOutcome | null;
}

/** Whether the worker has not ended the command yet. */
export function isOpen(command: Pick<Command, "state">): boolean {
  return command.state === "pending" || command.state === "running";
}

/**
 * A command ID for one user action. The browser makes it, sends it together
 * with the request's content, and asks for the command by it after a lost
 * answer. `randomUUID` needs a secure context, so the ID is built from
 * `getRandomValues`, which the app also has over plain HTTP.
 */
export function newCommandId(): string {
  const bytes = new Uint8Array(16);
  crypto.getRandomValues(bytes);
  return Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
}

/** Sends a command; `202` when stored now, `200` when the same command was stored before: both give its current state. */
export function sendCommand(id: string, kind: string, payload: unknown): Promise<Command> {
  return api<Command>("/commands", { method: "POST", body: { id, kind, payload } });
}

export function getCommand(id: string): Promise<Command> {
  return api<Command>(`/commands/${id}`);
}
