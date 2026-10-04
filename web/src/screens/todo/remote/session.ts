import { sameViewport } from "./geometry.ts";
import type { ServerMessage, Viewport } from "./protocol.ts";

/**
 * What one socket knows about the remote page's size and frames, to judge an
 * input before it is sent. The server drops an input made on another
 * generation or while the size changes; this keeps the device from sending
 * what it already knows would be dropped, and from stamping an input with a
 * generation other than the frame it was made on.
 */
export interface ScreenSession {
  /** The newest generation the server has told (a size change, a frame or a dropped input). */
  gen: number | null;
  /** The frame on screen: the one a person's pointer is over. */
  frame: { gen: number; width: number; height: number } | null;
  /** The size this device sent and the server has not confirmed yet. */
  pending: Viewport | null;
}

export const NEW_SESSION: ScreenSession = { gen: null, frame: null, pending: null };

const newest = (a: number | null, b: number) => (a === null ? b : Math.max(a, b));

/** This device sent its size. */
export function sentViewport(session: ScreenSession, viewport: Viewport): ScreenSession {
  return { ...session, pending: viewport };
}

/**
 * The session after a message of the server. A frame counts once it is on
 * screen, so pass a frame here when it is drawn.
 */
export function receive(session: ScreenSession, message: ServerMessage): ScreenSession {
  switch (message.type) {
    case "viewport": {
      const { gen, width, height, dpr, touch } = message;
      // The confirmation of this device's own size ends the wait; another device's size does not.
      const confirmed = session.pending !== null && sameViewport(session.pending, { width, height, dpr, touch });
      return { ...session, gen: newest(session.gen, gen), pending: confirmed ? null : session.pending };
    }
    case "frame":
      return {
        ...session,
        gen: newest(session.gen, message.gen),
        frame: { gen: message.gen, width: message.width, height: message.height },
      };
    case "dropped":
      return { ...session, gen: newest(session.gen, message.gen) };
    // The page's state and the tabs are not the session's: the screen keeps them.
    case "nav":
    case "tabs":
    case "page":
    case "ended":
      return session;
  }
}

/**
 * The generation to stamp an input with, which is the generation of the frame
 * it was made on; `null` when the input has to be dropped: no frame yet, this
 * device's size not confirmed yet, or the frame on screen is of an older size
 * than the page now has.
 */
export function inputGen(session: ScreenSession): number | null {
  const { frame, gen, pending } = session;
  if (frame === null || pending !== null || gen === null || frame.gen !== gen) return null;
  return frame.gen;
}
