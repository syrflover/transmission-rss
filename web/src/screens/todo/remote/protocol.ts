/**
 * The messages of a job's remote screen socket (`web/screen_api.rs` in `trss` is the contract). The shapes are generated
 * from the Rust types (`lib/api-types`, `cargo types`): `ServerMessage` from the server's messages and `ClientMessage`
 * from the ones it takes, so a message the server cannot send or read is not one this file knows. What is here is
 * pure: the screen's component only moves these between the socket and the page. It imports the types by a relative
 * path, type-only, so it runs under `node --test`.
 */

import type { ClientMessage, PageDialog, ServerMessage, Tab } from "../../../lib/api-types";

export type {
  ClientMessage,
  DialogKind,
  EndedReason,
  PageDialog,
  PageNav,
  ScreenLimits,
  ServerMessage,
  Tab,
  TouchPoint,
  Viewport,
} from "../../../lib/api-types";

const isNumber = (value: unknown): value is number => typeof value === "number" && Number.isFinite(value);

function parseTab(value: unknown): Tab | null {
  if (typeof value !== "object" || value === null) return null;
  const t = value as Record<string, unknown>;
  return typeof t.id === "string" &&
    t.id !== "" &&
    typeof t.title === "string" &&
    (t.host === null || typeof t.host === "string") &&
    typeof t.shown === "boolean" &&
    typeof t.closable === "boolean"
    ? { id: t.id, title: t.title, host: t.host, shown: t.shown, closable: t.closable }
    : null;
}

function parseDialog(value: unknown): PageDialog | null {
  if (typeof value !== "object" || value === null) return null;
  const d = value as Record<string, unknown>;
  return isNumber(d.id) &&
    (d.kind === "alert" || d.kind === "confirm" || d.kind === "prompt" || d.kind === "beforeunload") &&
    typeof d.message === "string" &&
    (d.host === null || typeof d.host === "string") &&
    typeof d.prompt === "string"
    ? { id: d.id, kind: d.kind, message: d.message, host: d.host, prompt: d.prompt }
    : null;
}

/** The message a text frame carries, or `null` when it is not one of the protocol's. */
export function parseServerMessage(text: string): ServerMessage | null {
  let value: unknown;
  try {
    value = JSON.parse(text);
  } catch {
    return null;
  }
  if (typeof value !== "object" || value === null) return null;
  const m = value as Record<string, unknown>;
  switch (m.type) {
    case "viewport":
      return isNumber(m.gen) && isNumber(m.width) && isNumber(m.height) && isNumber(m.dpr) && typeof m.touch === "boolean"
        ? { type: "viewport", gen: m.gen, width: m.width, height: m.height, dpr: m.dpr, touch: m.touch }
        : null;
    case "frame":
      return isNumber(m.gen) && isNumber(m.width) && isNumber(m.height) && m.width > 0 && m.height > 0 && typeof m.data === "string"
        ? { type: "frame", gen: m.gen, width: m.width, height: m.height, data: m.data }
        : null;
    case "dropped":
      return isNumber(m.gen) ? { type: "dropped", gen: m.gen } : null;
    case "nav":
      return typeof m.back === "boolean" && typeof m.forward === "boolean" && (m.host === null || typeof m.host === "string")
        ? { type: "nav", back: m.back, forward: m.forward, host: m.host }
        : null;
    case "tabs": {
      if (!Array.isArray(m.tabs)) return null;
      const tabs: Tab[] = [];
      for (const item of m.tabs as unknown[]) {
        const t = parseTab(item);
        if (t === null) return null;
        tabs.push(t);
      }
      return { type: "tabs", tabs };
    }
    case "page":
      return typeof m.responding === "boolean" ? { type: "page", responding: m.responding } : null;
    case "dialog": {
      if (m.dialog === null) return { type: "dialog", dialog: null };
      const dialog = parseDialog(m.dialog);
      return dialog === null ? null : { type: "dialog", dialog };
    }
    case "ended":
      return m.reason === "browser" || m.reason === "run" || m.reason === "unreachable" || m.reason === "stuck" || m.reason === "replaced"
        ? { type: "ended", reason: m.reason }
        : null;
    default:
      return null;
  }
}

/*
 * The server's `ClientMessage` is one enum tagged by `type`, so the parts the input code names are that union narrowed.
 * An input is made before it is stamped with the generation of the frame it was made on.
 */
type MouseMessage = Extract<ClientMessage, { type: "mouse" }>;
export type MouseEventName = MouseMessage["event"];
export type MouseButton = NonNullable<MouseMessage["button"]>;
export type TouchEventName = Extract<ClientMessage, { type: "touch" }>["event"];
export type KeyEventName = Extract<ClientMessage, { type: "key" }>["event"];

/** An input as the device makes it, before it is stamped with the generation of the frame it was made on. */
export type InputBody = DistributiveOmit<Extract<ClientMessage, { gen: number }>, "gen">;

type DistributiveOmit<T, K extends PropertyKey> = T extends unknown ? Omit<T, K> : never;

/** The text frame to send, or `null` when it would be too large for the socket (`maxBytes`). */
export function encode(message: ClientMessage, maxBytes: number): string | null {
  const text = JSON.stringify(message);
  return new TextEncoder().encode(text).length <= maxBytes ? text : null;
}

/** Pasted or typed text in pieces the server takes, never cutting a character in two. */
export function textChunks(text: string, size = 1000): string[] {
  const chunks: string[] = [];
  let chunk = "";
  let count = 0;
  for (const char of text) {
    chunk += char;
    count += 1;
    if (count === size) {
      chunks.push(chunk);
      chunk = "";
      count = 0;
    }
  }
  if (chunk !== "") chunks.push(chunk);
  return chunks;
}

/** The address of a job's remote screen socket for the binding of `run` made at `bound`, on the page's own origin. */
export function socketUrl(location: { protocol: string; host: string }, job: string, run: string, bound: number | null): string {
  const scheme = location.protocol === "https:" ? "wss" : "ws";
  const binding = bound === null ? "" : `&bound=${bound}`;
  return `${scheme}://${location.host}/api/subtitle-jobs/${encodeURIComponent(job)}/screen/socket?run=${encodeURIComponent(run)}${binding}`;
}
