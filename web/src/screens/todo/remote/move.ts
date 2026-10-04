/**
 * When the remote screen says it moves to another page. Pure, so the rules
 * are tested without a page.
 *
 * A move starts when a person picks another tab or closes the one shown
 * (`asked`), or when the server ends the screen (`server`): `run` when the
 * worker moved it (as to a window the page opened), `browser` when its page
 * went (a page closed, by a person or by itself, goes before the worker moves
 * the screen to the newest page left). It lasts until the first frame of
 * another socket is drawn, so the screen says one thing from the tap to the
 * new page, instead of the old page, an end and a connection in turn. The
 * worker looks for a switch every second and the web for a new binding every
 * two, so a move takes a few seconds. A browser that is really gone shows as
 * such once the move makes no step, or sooner when the job's screen says it
 * closed.
 */
import type { EndedReason } from "./protocol";

/** How long a move is shown without a step: past it, the screen shows what its socket says (a switch the worker did not make, a job not read). */
export const MOVE_LIMIT_MS = 10_000;

export interface Move<S> {
  /** A person asked for it, rather than the server ending the screen. */
  kind: "asked" | "server";
  /** The socket the move was asked on while it is open; `null` once it ended. */
  from: S | null;
}

export type MoveEvent<S> =
  /** A person asked for another page on the socket `on`. */
  | { type: "asked"; on: S }
  /** That ask failed: the screen stays where it was. */
  | { type: "refused"; on: S }
  | { type: "ended"; on: S; reason: EndedReason }
  /** The first frame of the socket `on` was drawn. */
  | { type: "drawn"; on: S }
  /** The socket closed on its own: the screen says it reconnects instead. */
  | { type: "lost" }
  /** `move` went {@link MOVE_LIMIT_MS} without a step. */
  | { type: "expired"; move: Move<S> };

/** The move after `event`; `null` when the screen does not move. */
export function nextMove<S>(move: Move<S> | null, event: MoveEvent<S>): Move<S> | null {
  switch (event.type) {
    case "asked":
      return { kind: "asked", from: event.on };
    case "refused":
      // An ask the server already answered by ending the binding is on its way.
      return move !== null && move.kind === "asked" && move.from === event.on ? null : move;
    case "ended":
      // Another end (no browser to reach, the page stuck, another device) says its own thing.
      if (event.reason !== "run" && event.reason !== "browser") return null;
      return { kind: move?.kind ?? "server", from: null };
    case "drawn":
      return move !== null && move.from !== event.on ? null : move;
    case "lost":
      return null;
    case "expired":
      return move === event.move ? null : move;
  }
}
