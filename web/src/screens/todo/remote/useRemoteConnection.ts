import { useCallback, useEffect, useRef, useState, type RefObject } from "react";

import { store } from "@/lib/cached";

import { fetchJob, type JobDetail } from "../api";
import { KEYS } from "../poll";
import { deviceViewport, sameViewport } from "./geometry";
import { encode, parseServerMessage, socketUrl, type ClientMessage, type EndedReason, type InputBody, type ServerMessage, type Viewport } from "./protocol";
import { afterEnd, retryDelay, sameBinding } from "./reconnect";
import { inputGen, NEW_SESSION, receive, sentViewport, type ScreenSession } from "./session";

/** How long the device's size must stay put before it is sent again. */
const RESIZE_MS = 250;

export type Phase =
  /** The socket is opening, or open and no frame has come yet. */
  | "connecting"
  | "live"
  /** The socket closed on its own: the job is read again before it is tried again. */
  | "retrying"
  /** Every attempt failed: the person asks again. */
  | "failed"
  /** The server ended the socket (`reason`). */
  | "ended";

export interface Connection {
  phase: Phase;
  reason: EndedReason | null;
  /** The size of the frame on screen, in the remote page's CSS pixels. */
  frame: { width: number; height: number } | null;
  /** The size this device asked the page to be laid out for. */
  planned: Viewport | null;
  /** The session the inputs are judged by; read it when an input is made. */
  session: RefObject<ScreenSession>;
  /** Sends an input stamped with the generation of the frame on screen; `false` when it had to be dropped. */
  send: (body: InputBody) => boolean;
  reload: () => void;
}

/** Whether this device is a touch screen: its main pointer is a finger. */
export function isTouchDevice(): boolean {
  return typeof window.matchMedia === "function" && window.matchMedia("(pointer: coarse)").matches;
}

async function readJob(id: string): Promise<JobDetail | null> {
  try {
    const job = await fetchJob(id);
    store(KEYS.job(id), job);
    return job;
  } catch {
    return null;
  }
}

/**
 * One job's remote screen socket for the binding it has (the run, bound at
 * `bound`), drawn into `image`. It reports this device's size when it opens
 * and when the area changes, keeps the {@link ScreenSession}, and reconnects
 * when the socket closes on its own, reading the job first and never asking
 * for a run (only the page's opening does that). A socket the server ended is
 * not connected again to the same binding; another binding, of the same run
 * too (another check of the job), comes through new `run` or `bound`.
 * `attempt` connects again on demand.
 */
export function useRemoteConnection(options: {
  jobId: string;
  run: string | null;
  bound: number | null;
  attempt: number;
  area: RefObject<HTMLElement | null>;
  image: RefObject<HTMLImageElement | null>;
}): Connection {
  const { jobId, run, bound, attempt, area, image } = options;
  const [phase, setPhase] = useState<Phase>("connecting");
  const [reason, setReason] = useState<EndedReason | null>(null);
  const [frame, setFrame] = useState<{ width: number; height: number } | null>(null);
  const [planned, setPlanned] = useState<Viewport | null>(null);
  /** Counts the retries after a read of the job said to connect to the same run again. */
  const [round, setRound] = useState(0);
  const session = useRef<ScreenSession>(NEW_SESSION);
  const socket = useRef<WebSocket | null>(null);
  /** Failed attempts in a row, for the binding they were made for; a frame starts the count over. */
  const failures = useRef<{ run: string | null; bound: number | null; attempt: number; count: number }>({
    run: null,
    bound: null,
    attempt,
    count: 0,
  });

  useEffect(() => {
    if (run === null) return;
    const binding = { run, bound };
    // Another binding, or the person asking again, starts the count over; a retry of the same does not.
    const was = failures.current;
    if (was.run !== run || was.bound !== bound || was.attempt !== attempt) failures.current = { run, bound, attempt, count: 0 };
    let closed = false;
    let ended: EndedReason | null = null;
    let retry: ReturnType<typeof setTimeout> | undefined;
    let resize: ReturnType<typeof setTimeout> | undefined;
    let raf = 0;
    let waiting: Extract<ServerMessage, { type: "frame" }> | null = null;
    let sent: Viewport | null = null;
    let shownSize: { width: number; height: number } | null = null;

    session.current = NEW_SESSION;
    setPhase("connecting");
    setReason(null);
    setFrame(null);
    const ws = new WebSocket(socketUrl(window.location, jobId, run, bound));
    socket.current = ws;

    const report = (force: boolean) => {
      const box = area.current;
      if (box === null || ws.readyState !== WebSocket.OPEN) return;
      const touch = isTouchDevice();
      const next = deviceViewport({
        areaWidth: box.clientWidth,
        // A virtual keyboard shortens the window of a touch screen, not its screen.
        deviceHeight: touch ? window.screen.height : window.innerHeight,
        dpr: window.devicePixelRatio,
        touch,
      });
      if (!force && sent !== null && sameViewport(sent, next)) return;
      const text = encode({ type: "viewport", ...next });
      if (text === null) return;
      sent = next;
      session.current = sentViewport(session.current, next);
      setPlanned(next);
      ws.send(text);
    };
    const resized = () => {
      clearTimeout(resize);
      resize = setTimeout(() => report(false), RESIZE_MS);
    };

    // The newest frame wins: drawn at most once per animation frame.
    const draw = () => {
      raf = 0;
      const next = waiting;
      waiting = null;
      const img = image.current;
      if (next === null || img === null) return;
      img.src = `data:image/jpeg;base64,${next.data}`;
      session.current = receive(session.current, next);
      failures.current.count = 0;
      if (shownSize === null || shownSize.width !== next.width || shownSize.height !== next.height) {
        shownSize = { width: next.width, height: next.height };
        setFrame(shownSize);
      }
      setPhase("live");
    };

    const later = (after: () => void) => {
      failures.current.count += 1;
      const delay = retryDelay(failures.current.count);
      if (delay === null) {
        setPhase("failed");
        void readJob(jobId);
        return;
      }
      setPhase("retrying");
      retry = setTimeout(after, delay);
    };
    // The socket closed on its own: read the job, and connect again to the binding it still has.
    const reconnect = () => {
      later(async () => {
        const job = await readJob(jobId);
        if (closed) return;
        if (job === null) return reconnect();
        const outcome = afterEnd({ waiting: job.state === "waiting" && job.wait === "auth", screen: job.screen }, null);
        // Another binding comes through the new `run` or `bound`; the same one is tried again here.
        if (outcome.kind === "connect" && sameBinding(outcome.binding, binding)) setRound((n) => n + 1);
      });
    };

    ws.onopen = () => report(true);
    ws.onmessage = (event) => {
      if (typeof event.data !== "string") return;
      const message = parseServerMessage(event.data);
      if (message === null) return;
      switch (message.type) {
        case "frame":
          waiting = message;
          if (raf === 0) raf = requestAnimationFrame(draw);
          break;
        case "ended":
          ended = message.reason;
          break;
        default:
          session.current = receive(session.current, message);
      }
    };
    ws.onclose = () => {
      if (closed) return;
      cancelAnimationFrame(raf);
      raf = 0;
      if (ended !== null) {
        setReason(ended);
        setPhase("ended");
        // The binding changed or the browser is gone: the job says which (a new binding connects through `run` and `bound`).
        void readJob(jobId);
        return;
      }
      reconnect();
    };

    const watcher = typeof ResizeObserver === "function" && area.current !== null ? new ResizeObserver(resized) : null;
    if (watcher !== null && area.current !== null) watcher.observe(area.current);
    window.addEventListener("resize", resized);
    window.addEventListener("orientationchange", resized);

    return () => {
      closed = true;
      clearTimeout(retry);
      clearTimeout(resize);
      cancelAnimationFrame(raf);
      watcher?.disconnect();
      window.removeEventListener("resize", resized);
      window.removeEventListener("orientationchange", resized);
      ws.onopen = ws.onmessage = ws.onclose = null;
      ws.close();
      if (socket.current === ws) socket.current = null;
    };
  }, [jobId, run, bound, attempt, round, area, image]);

  const send = useCallback((body: InputBody): boolean => {
    const ws = socket.current;
    const gen = inputGen(session.current);
    if (ws === null || ws.readyState !== WebSocket.OPEN || gen === null) return false;
    const text = encode({ ...body, gen } as ClientMessage);
    if (text === null) return false;
    ws.send(text);
    return true;
  }, []);

  const reload = useCallback(() => {
    const ws = socket.current;
    const text = encode({ type: "reload" });
    if (ws !== null && ws.readyState === WebSocket.OPEN && text !== null) ws.send(text);
  }, []);

  return { phase, reason, frame, planned, session, send, reload };
}
