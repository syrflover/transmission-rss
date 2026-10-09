import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type ClipboardEvent,
  type CompositionEvent,
  type KeyboardEvent,
  type PointerEvent,
  type ReactNode,
} from "react";

import { Button } from "@/components/ui/button";
import { ApiError } from "@/lib/api";
import { cn } from "@/lib/utils";

import { btnAction, btnNeutral } from "../../collect/channels/styles";
import { closeTab, restartScreen, switchTab } from "../api";
import { CloseIcon, HistoryBackIcon, HistoryForwardIcon, ReloadIcon } from "../icons";
import { toRemote } from "./geometry";
import {
  buttonName,
  composedText,
  draggedButton,
  inputEventBodies,
  keyDownBody,
  keyUpBody,
  modifiersOf,
  nextClickCount,
  wheelPixels,
} from "./keys";
import { textChunks, type EndedReason, type InputBody, type ScreenLimits, type Tab, type TouchPoint } from "./protocol";
import { PageDialogCard } from "./PageDialogCard";
import { showsTabRow, tabLabel, withShown } from "./tabs";
import { isTouchDevice, useRemoteConnection } from "./useRemoteConnection";

const TAB_FAILED = "창을 바꾸지 못했어요. 잠시 뒤 다시 시도해 주세요.";
const RESTART_FAILED = "서버 브라우저를 새로 띄우지 못했어요. 잠시 뒤 다시 시도해 주세요.";
const STALLED_TEXT =
  "페이지가 응답하지 않아요. 잠시 기다리거나, 서버 브라우저를 새로 띄워 게시물을 다시 열 수 있어요. 받은 파일은 남아요.";

/** A round button with an icon, in the shape of the screen's other buttons; it is at least as tall as they are. */
const iconButton = cn(btnNeutral, "size-9 min-h-9 w-9 px-0 max-[720px]:size-10 max-[720px]:min-h-10");

/** `browser` and `run` show only once the move to another page made no step for a while (`move.ts`). */
const ENDED_TEXT: Record<EndedReason, string> = {
  browser: "서버 브라우저가 닫혔어요. 작업 화면을 다시 열면 다시 준비해요.",
  run: "새 화면을 불러오지 못했어요. 다시 열면 다시 준비해요.",
  unreachable: "서버 브라우저에 닿지 못했어요. 작업 화면을 다시 열면 다시 준비해요.",
  stuck: "페이지가 응답하지 않아요. 서버 브라우저를 새로 띄워 게시물을 다시 열 수 있어요. 받은 파일은 남아요.",
  replaced: "다른 기기나 탭에서 이 인증 화면을 열어서 여기 화면은 닫혔어요. 여기서 계속하려면 다시 열어 주세요.",
};

/**
 * The remote screen of a job: the server browser's page drawn into the area,
 * and the pointer, touch and keyboard input of this device relayed to it.
 *
 * - Above the area, a toolbar has back, forward and reload buttons, the host
 *   of the page shown (never its whole address: the server sends only the
 *   host) and, on a touch screen, `키보드`. Back and forward are on as the
 *   server says ({@link Connection.nav}); the server judges every step
 *   again, and never goes back to the blank page it passed through.
 * - Below it, a row of tabs when the run has two or more pages: the page
 *   shown is marked, a tab switches to its page, and every tab but the run's
 *   first has a close control. Switching and closing are asked of the server
 *   (`POST .../screen/switch` and `.../close`) for the binding this screen
 *   shows; the worker moves the screen, which reconnects. From the ask until
 *   the new page is drawn the screen says it changes the window, and when
 *   the worker moves it by itself (to a window the page opened, or from one
 *   that closed) it says it loads the screen ({@link Connection.moving}).
 * - Pointer positions are mapped from the shown box to the frame's own CSS
 *   pixels with {@link toRemote}, so a frame of another size than the area
 *   (another device opened the screen last) is still hit exactly. An input is
 *   stamped with the generation of the frame on screen and is dropped here
 *   while this device's size is unconfirmed ({@link useRemoteConnection}).
 * - The keyboard is a hidden text field. A click (or the `키보드` button on a
 *   touch screen) focuses it; only then do keys, composed text and pastes go
 *   to the remote page, so the other forms of the page are unaffected. A
 *   touch tap does not raise the virtual keyboard by itself.
 * - The area stops the page from scrolling or zooming under a finger or a
 *   wheel, only over itself.
 * - A page that does not answer (the server says so, or ended the socket as
 *   `stuck`) is covered by a note with `새로 띄우기`, which asks the worker
 *   for a new run of the post (`POST .../screen/restart`); the note stays
 *   until the job has another binding.
 * - A dialog the page shows, which no frame draws, is a card over the screen
 *   ({@link PageDialogCard}); the toolbar and the tabs wait until it is
 *   answered.
 */
export function RemoteScreen({
  jobId,
  run,
  bound,
  headingId,
  title,
  opening,
  onReopen,
  limits,
}: {
  jobId: string;
  run: string;
  /** When `run` was bound to the job: another check in the same run is another binding. */
  bound: number | null;
  headingId: string;
  title: string;
  opening: boolean;
  /** Asks for the page and its screen to be prepared again; resolves when the answer is in. */
  onReopen: () => Promise<void>;
  /** What the server's socket takes. */
  limits: ScreenLimits;
}) {
  const area = useRef<HTMLDivElement>(null);
  const image = useRef<HTMLImageElement>(null);
  const field = useRef<HTMLTextAreaElement>(null);
  const refresh = useRef<HTMLButtonElement>(null);
  const [attempt, setAttempt] = useState(0);
  /** A request about a tab is on its way (the tabs wait for the answer). */
  const [asking, setAsking] = useState(false);
  /** The tab a switch was asked for, shown as the page until the server's tabs say so. */
  const [wanted, setWanted] = useState<string | null>(null);
  const [tabError, setTabError] = useState<string | null>(null);
  /** A new run was asked for; the screen of this binding waits for the next one. */
  const [restarting, setRestarting] = useState(false);
  const [restartError, setRestartError] = useState<string | null>(null);
  useEffect(() => {
    setRestarting(false);
    setRestartError(null);
  }, [run, bound]);
  const [touch] = useState(isTouchDevice);
  const [typing, setTyping] = useState(false);
  const conn = useRemoteConnection({ jobId, run, bound, attempt, area, image, limits });
  const { send, session } = conn;
  const tabs = withShown(conn.tabs, wanted);
  // The server's tabs are the truth again once they come.
  useEffect(() => setWanted(null), [conn.tabs]);

  /** Asks the server to switch to or close a tab, for the binding this screen shows. */
  const askTab = async (ask: (jobId: string, run: string, bound: number, target: string) => Promise<void>, tab: Tab) => {
    if (bound === null || asking) return;
    setAsking(true);
    setTabError(null);
    // A switch, or closing the page shown, moves the screen; closing another page does not.
    const takeBack = ask === switchTab || tab.shown ? conn.askMove() : null;
    try {
      await ask(jobId, run, bound, tab.id);
      if (ask === switchTab) setWanted(tab.id);
    } catch (e) {
      takeBack?.();
      setTabError(e instanceof ApiError ? e.message : TAB_FAILED);
    } finally {
      setAsking(false);
    }
  };

  /** Asks the worker for a new run of the post, for the binding this screen shows. */
  const restart = async () => {
    if (bound === null || restarting) return;
    setRestarting(true);
    setRestartError(null);
    try {
      await restartScreen(jobId, run, bound);
    } catch (e) {
      setRestarting(false);
      setRestartError(e instanceof ApiError ? e.message : RESTART_FAILED);
    }
  };

  // --- pointer and touch ---------------------------------------------------------------------

  const fingers = useRef(new Map<number, TouchPoint>());
  const mouseDown = useRef(false);
  const lastClick = useRef<{ at: number; x: number; y: number; count: number } | null>(null);
  const move = useRef<InputBody | null>(null);
  const moveFrame = useRef(0);

  const remotePoint = useCallback(
    (e: { clientX: number; clientY: number }) => {
      const frame = session.current.frame;
      const box = area.current?.getBoundingClientRect();
      return frame && box ? toRemote(e, box, frame) : null;
    },
    [session],
  );

  const flushMove = useCallback(() => {
    cancelAnimationFrame(moveFrame.current);
    moveFrame.current = 0;
    const body = move.current;
    move.current = null;
    if (body !== null) send(body);
  }, [send]);
  const queueMove = (body: InputBody) => {
    move.current = body;
    if (moveFrame.current === 0) moveFrame.current = requestAnimationFrame(flushMove);
  };
  useEffect(() => () => cancelAnimationFrame(moveFrame.current), []);

  const touches = () => [...fingers.current.values()];
  /** The lowest id no finger on the screen has: the server takes small ones. */
  const freeId = () => {
    const used = new Set(touches().map((p) => p.id));
    let id = 0;
    while (used.has(id)) id += 1;
    return id;
  };

  const onPointerDown = (e: PointerEvent<HTMLDivElement>) => {
    const at = remotePoint(e);
    if (at === null) return;
    if (e.pointerType === "touch") {
      const finger = { ...at, id: freeId() };
      if (!send({ type: "touch", event: "touchStart", points: [...touches(), finger] })) return;
      fingers.current.set(e.pointerId, finger);
    } else {
      field.current?.focus({ preventScroll: true });
      const count = nextClickCount(lastClick.current, { at: e.timeStamp, x: at.x, y: at.y });
      lastClick.current = { at: e.timeStamp, x: at.x, y: at.y, count };
      mouseDown.current = send({
        type: "mouse",
        event: "mousePressed",
        ...at,
        button: buttonName(e.button),
        buttons: e.buttons,
        clickCount: count,
        modifiers: modifiersOf(e),
      });
      if (!mouseDown.current) return;
    }
    try {
      e.currentTarget.setPointerCapture(e.pointerId);
    } catch {
      // A pointer that is already gone has nothing to capture.
    }
  };

  const onPointerMove = (e: PointerEvent<HTMLDivElement>) => {
    const at = remotePoint(e);
    if (at === null) return;
    if (e.pointerType === "touch") {
      const finger = fingers.current.get(e.pointerId);
      if (finger === undefined) return;
      fingers.current.set(e.pointerId, { ...at, id: finger.id });
      queueMove({ type: "touch", event: "touchMove", points: touches() });
    } else {
      queueMove({
        type: "mouse",
        event: "mouseMoved",
        ...at,
        button: draggedButton(e.buttons),
        buttons: e.buttons,
        modifiers: modifiersOf(e),
      });
    }
  };

  const onPointerEnd = (e: PointerEvent<HTMLDivElement>) => {
    const cancelled = e.type === "pointercancel";
    if (e.pointerType === "touch") {
      if (!fingers.current.delete(e.pointerId)) return;
      flushMove();
      send({ type: "touch", event: cancelled ? "touchCancel" : "touchEnd", points: cancelled ? [] : touches() });
      if (cancelled) fingers.current.clear();
    } else {
      if (!mouseDown.current) return;
      mouseDown.current = false;
      flushMove();
      const at = remotePoint(e);
      if (at === null) return;
      send({
        type: "mouse",
        event: "mouseReleased",
        ...at,
        button: buttonName(e.button),
        buttons: e.buttons,
        clickCount: lastClick.current?.count ?? 1,
        modifiers: modifiersOf(e),
      });
    }
  };

  // The wheel moves the remote page, not this one; a listener that can stop the page needs `passive: false`.
  useEffect(() => {
    const box = area.current;
    if (box === null) return;
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      const at = remotePoint(e);
      if (at === null) return;
      const { dx, dy } = wheelPixels(e, box.clientHeight);
      send({ type: "mouse", event: "mouseWheel", ...at, deltaX: dx, deltaY: dy, modifiers: modifiersOf(e) });
    };
    box.addEventListener("wheel", onWheel, { passive: false });
    return () => box.removeEventListener("wheel", onWheel);
  }, [remotePoint, send]);

  // --- keyboard ------------------------------------------------------------------------------

  const sendAll = useCallback(
    (bodies: InputBody[]) => {
      for (const body of bodies) send(body);
    },
    [send],
  );

  const onKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    // The way out of the field for a keyboard: Tab and Escape belong to the remote page.
    if (e.key === "Escape" && e.shiftKey) {
      e.preventDefault();
      refresh.current?.focus();
      return;
    }
    const body = keyDownBody(e.nativeEvent);
    if (body === null) return;
    e.preventDefault();
    send(body);
  };
  const onKeyUp = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    const body = keyUpBody(e.nativeEvent);
    if (body === null) return;
    e.preventDefault();
    send(body);
  };
  const onCompositionEnd = (e: CompositionEvent<HTMLTextAreaElement>) => {
    sendAll(composedText(e.data));
    e.currentTarget.value = "";
  };
  const onPaste = (e: ClipboardEvent<HTMLTextAreaElement>) => {
    e.preventDefault();
    sendAll(textChunks(e.clipboardData.getData("text")).map((text) => ({ type: "text", text })));
  };

  // Input events that reach the field without a key: a phone keyboard's text, Enter and Backspace.
  useEffect(() => {
    const input = field.current;
    if (input === null) return;
    const onBeforeInput = (e: InputEvent) => {
      const bodies = inputEventBodies(e);
      if (bodies.length === 0) return;
      e.preventDefault();
      sendAll(bodies);
    };
    input.addEventListener("beforeinput", onBeforeInput);
    return () => input.removeEventListener("beforeinput", onBeforeInput);
  }, [sendAll]);

  const showKeyboard = () => {
    if (document.activeElement === field.current) field.current?.blur();
    else field.current?.focus();
  };

  // --- what is shown -------------------------------------------------------------------------

  const { phase, frame, planned, reason, moving } = conn;
  const live = phase === "live";
  // Also before the first frame: a page that is already stalled when the
  // screen connects sends none until it answers again.
  const stalled = (live || phase === "connecting") && !conn.responding;
  // A dialog the page shows: the person answers it before anything else goes to the page.
  const dialog = (live || phase === "connecting") && !restarting ? conn.dialog : null;
  const covered = !live || stalled || restarting || moving !== null;
  const usable = live && dialog === null && moving === null;
  const aspect = frame ?? planned;
  // A page that ended while the screen was ready: the person asks for it again.
  const reopen = () => {
    void onReopen().then(() => setAttempt((n) => n + 1));
  };

  return (
    <>
      <h2 id={headingId} className="mb-2 text-[17px] font-bold">
        {title}
      </h2>
      <div role="toolbar" aria-label="원격 화면 도구" className="mb-2 flex items-center gap-1.5">
        <Button
          type="button"
          variant="ghost"
          className={iconButton}
          aria-label="뒤로"
          title="뒤로"
          disabled={!usable || !conn.nav?.back}
          onClick={conn.back}
        >
          <HistoryBackIcon className="size-[18px]" />
        </Button>
        <Button
          type="button"
          variant="ghost"
          className={iconButton}
          aria-label="앞으로"
          title="앞으로"
          disabled={!usable || !conn.nav?.forward}
          onClick={conn.forward}
        >
          <HistoryForwardIcon className="size-[18px]" />
        </Button>
        <Button
          ref={refresh}
          type="button"
          variant="ghost"
          className={iconButton}
          aria-label="새로고침"
          title="새로고침"
          disabled={!usable}
          onClick={conn.reload}
        >
          <ReloadIcon className="size-[18px]" />
        </Button>
        <p className="min-w-0 flex-1 truncate px-1.5 text-[13px] text-text-secondary" title={conn.nav?.host ?? undefined}>
          {conn.nav?.host != null && <span className="sr-only">현재 사이트 </span>}
          {conn.nav?.host}
        </p>
        {touch && (
          <Button
            type="button"
            variant="ghost"
            className={cn(btnNeutral, "ml-auto")}
            aria-pressed={typing}
            disabled={!usable}
            onClick={showKeyboard}
          >
            키보드
          </Button>
        )}
      </div>
      {showsTabRow(tabs) && (
        <div role="group" aria-label="열린 창" className="mb-2 flex flex-wrap gap-1.5">
          {tabs.map((tab) => (
            <div
              key={tab.id}
              className={cn(
                "flex max-w-[220px] min-w-0 items-center rounded-full border max-[720px]:max-w-full",
                tab.shown
                  ? "border-focus bg-[color-mix(in_srgb,var(--focus-ring)_12%,transparent)]"
                  : "border-hairline bg-surface-1",
              )}
            >
              <button
                type="button"
                aria-current={tab.shown ? "page" : undefined}
                disabled={asking || !usable}
                className={cn(
                  "min-h-9 min-w-0 flex-1 truncate rounded-full px-3 text-left text-[13px] font-semibold max-[720px]:min-h-10",
                  tab.shown ? "text-focus" : "text-text-primary",
                  !tab.closable && "pr-3",
                  tab.closable && "pr-1",
                )}
                onClick={() => {
                  if (!tab.shown) void askTab(switchTab, tab);
                }}
              >
                {tabLabel(tab)}
              </button>
              {tab.closable && (
                <button
                  type="button"
                  aria-label={`${tabLabel(tab)} 닫기`}
                  title="닫기"
                  disabled={asking || !usable}
                  className="flex size-9 flex-none items-center justify-center rounded-full text-text-secondary hover:text-text-primary max-[720px]:size-10"
                  onClick={() => void askTab(closeTab, tab)}
                >
                  <CloseIcon className="size-[14px]" />
                </button>
              )}
            </div>
          ))}
        </div>
      )}
      {tabError !== null && (
        <p role="alert" className="mb-2 text-[13px] font-semibold text-urgent">
          {tabError}
        </p>
      )}

      <div
        className="relative overflow-hidden rounded-card border border-hairline bg-surface-2 shadow-(--card-shadow) outline-offset-2 focus-within:outline-2 focus-within:outline-focus"
      >
        <div
          ref={area}
          role="group"
          aria-label="서버 브라우저의 원격 화면"
          className={cn("relative block w-full touch-none select-none", !live && "min-h-[300px]")}
          style={aspect ? { aspectRatio: `${aspect.width} / ${aspect.height}` } : undefined}
          onPointerDown={onPointerDown}
          onPointerMove={onPointerMove}
          onPointerUp={onPointerEnd}
          onPointerCancel={onPointerEnd}
          onContextMenu={(e) => e.preventDefault()}
          // Keeps the field focused (and the page from selecting text) when the remote page is clicked.
          onMouseDown={(e) => e.preventDefault()}
        >
          <img
            ref={image}
            alt=""
            draggable={false}
            className={cn(
              "pointer-events-none absolute inset-0 size-full",
              !frame && "invisible",
              (covered || dialog !== null) && "opacity-40",
            )}
          />
          <textarea
            ref={field}
            aria-label="원격 화면에 입력"
            aria-description="이 칸에 초점이 있는 동안 키보드 입력이 서버 브라우저로 가요. Shift와 Esc를 함께 누르면 빠져나가요."
            rows={1}
            autoCapitalize="off"
            autoComplete="off"
            autoCorrect="off"
            spellCheck={false}
            enterKeyHint="enter"
            className="absolute top-0 left-0 size-px resize-none border-0 p-0 text-base opacity-0 outline-none"
            onFocus={() => setTyping(true)}
            onBlur={() => setTyping(false)}
            onKeyDown={onKeyDown}
            onKeyUp={onKeyUp}
            onCompositionEnd={onCompositionEnd}
            onPaste={onPaste}
          />
        </div>

        {dialog !== null && (
          <PageDialogCard
            key={dialog.id}
            dialog={dialog}
            maxText={limits.max_prompt_text}
            onAnswer={(accept, text) => conn.answerDialog(dialog.id, accept, text)}
          />
        )}
        {covered && dialog === null && (
          <div
            role="status"
            className="absolute inset-0 flex flex-col items-center justify-center gap-3 bg-surface-1/60 p-4 text-center text-[13.5px] leading-relaxed text-text-secondary"
          >
            {restarting ? (
              <Loading>서버 브라우저를 새로 띄우고 있어요.</Loading>
            ) : moving !== null && !stalled ? (
              <Loading>{moving === "asked" ? "창을 바꾸는 중이에요." : "화면을 불러오는 중이에요."}</Loading>
            ) : (
              <>
                {phase === "connecting" && !stalled && <Loading>화면을 불러오는 중이에요.</Loading>}
                {phase === "retrying" && <Loading>연결이 끊겨서 다시 연결하고 있어요.</Loading>}
                {phase === "failed" && (
                  <>
                    <p>서버 브라우저에 다시 연결하지 못했어요.</p>
                    <Button type="button" variant="ghost" className={btnAction} onClick={() => setAttempt((n) => n + 1)}>
                      다시 연결
                    </Button>
                  </>
                )}
                {stalled && (
                  <>
                    <p>{STALLED_TEXT}</p>
                    {bound !== null && (
                      <Button type="button" variant="ghost" className={btnAction} onClick={() => void restart()}>
                        새로 띄우기
                      </Button>
                    )}
                  </>
                )}
                {phase === "ended" && reason === "stuck" && (
                  <>
                    <p>{ENDED_TEXT.stuck}</p>
                    <div className="flex flex-wrap justify-center gap-2">
                      {bound !== null && (
                        <Button type="button" variant="ghost" className={btnAction} onClick={() => void restart()}>
                          새로 띄우기
                        </Button>
                      )}
                      <Button type="button" variant="ghost" className={btnNeutral} onClick={() => setAttempt((n) => n + 1)}>
                        다시 연결
                      </Button>
                    </div>
                  </>
                )}
                {phase === "ended" && reason !== "stuck" && (
                  <>
                    <p>{ENDED_TEXT[reason ?? "browser"]}</p>
                    <Button type="button" variant="ghost" className={btnAction} disabled={opening} onClick={reopen}>
                      다시 열기
                    </Button>
                  </>
                )}
                {restartError !== null && (
                  <p role="alert" className="text-[13px] font-semibold text-urgent">
                    {restartError}
                  </p>
                )}
              </>
            )}
          </div>
        )}
      </div>

      <p className="mt-2 text-xs leading-snug text-text-muted">
        {touch
          ? "화면을 눌러 조작하고, 글자는 키보드 버튼으로 입력해요."
          : "화면을 누른 뒤 입력하면 서버 브라우저로 전달돼요. 입력에서 빠져나가려면 Shift와 Esc를 함께 눌러요."}
      </p>
    </>
  );
}

/** A note of the screen that something is on its way: a turning ring before the text. */
function Loading({ children }: { children: ReactNode }) {
  return (
    <p className="flex items-center gap-2">
      <span aria-hidden className="size-4 flex-none animate-spin rounded-full border-2 border-hairline border-t-focus" />
      {children}
    </p>
  );
}
