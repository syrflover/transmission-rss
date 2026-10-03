/**
 * The messages of a job's remote screen socket (`screen_api.rs` in `trss-web`
 * is the contract). Everything here is pure: the screen's component only
 * moves these between the socket and the page.
 */

/** What a device reports about the area that shows the remote page. */
export interface Viewport {
  /** CSS pixels. */
  width: number;
  height: number;
  /** Device pixel ratio. */
  dpr: number;
  touch: boolean;
}

/**
 * Why the server ended a socket: the page or its connection is gone, the job's binding changed, the run could not be
 * reached, or too many screens are open on the binding already.
 */
export type EndedReason = "browser" | "run" | "unreachable" | "replaced";

export type ServerMessage =
  /** A change of size is done (or, on connecting, the size the page has now); `gen` names the frames of it. */
  | ({ type: "viewport"; gen: number } & Viewport)
  /** A JPEG frame of generation `gen`; `width` and `height` are the remote page's CSS pixels. */
  | { type: "frame"; gen: number; width: number; height: number; data: string }
  /** An input was dropped; `gen` is the generation the page is at. */
  | { type: "dropped"; gen: number }
  | { type: "ended"; reason: EndedReason };

const isNumber = (value: unknown): value is number => typeof value === "number" && Number.isFinite(value);

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
    case "ended":
      return m.reason === "browser" || m.reason === "run" || m.reason === "unreachable" || m.reason === "replaced"
        ? { type: "ended", reason: m.reason }
        : null;
    default:
      return null;
  }
}

export type MouseEventName = "mousePressed" | "mouseReleased" | "mouseMoved" | "mouseWheel";
export type MouseButton = "none" | "left" | "middle" | "right" | "back" | "forward";
export type TouchEventName = "touchStart" | "touchMove" | "touchEnd" | "touchCancel";
export type KeyEventName = "keyDown" | "keyUp" | "rawKeyDown" | "char";

export interface TouchPoint {
  x: number;
  y: number;
  id: number;
}

/** An input as the device makes it, before it is stamped with the generation of the frame it was made on. */
export type InputBody =
  | {
      type: "mouse";
      event: MouseEventName;
      x: number;
      y: number;
      button?: MouseButton;
      buttons?: number;
      clickCount?: number;
      modifiers?: number;
      deltaX?: number;
      deltaY?: number;
    }
  | { type: "touch"; event: TouchEventName; points: TouchPoint[]; modifiers?: number }
  | {
      type: "key";
      event: KeyEventName;
      key?: string;
      code?: string;
      text?: string;
      keyCode?: number;
      modifiers?: number;
    }
  | { type: "text"; text: string };

export type ClientMessage =
  | ({ type: "viewport" } & Viewport)
  | { type: "reload" }
  | (InputBody & { gen: number });

/** The server drops a message over this many bytes by closing the socket. */
export const MAX_MESSAGE_BYTES = 64 * 1024;
/** The most characters of one `text` input the server takes. */
export const MAX_TEXT = 2000;

/** The text frame to send, or `null` when it would be too large for the socket. */
export function encode(message: ClientMessage): string | null {
  const text = JSON.stringify(message);
  return new TextEncoder().encode(text).length <= MAX_MESSAGE_BYTES ? text : null;
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
