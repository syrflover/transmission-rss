import { api, timedRead } from "@/lib/api";
import type {
  Command,
  CommandKind,
  CommandOutcome,
  CommandRequest,
  CommandState,
} from "@/lib/api-types";

/**
 * Web commands (`src/web/commands_api.rs`): actions the web accepts and the
 * worker carries out. The screens that send or follow them (`다시 받기`, the
 * move of a work folder) share these.
 */

export type { Command, CommandKind, CommandOutcome, CommandState };

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
export function sendCommand<K extends CommandKind>(
  id: string,
  kind: K,
  payload: Extract<CommandRequest, { kind: K }>["payload"],
): Promise<Command> {
  const body: CommandRequest = { id, kind, payload } as CommandRequest;
  return api<Command>("/commands", { method: "POST", body });
}

export function getCommand(id: string): Promise<Command> {
  return api<Command>(`/commands/${id}`, timedRead());
}
