import { useCallback, useEffect, useRef, useState, type ClipboardEvent, type CompositionEvent, type KeyboardEvent, type PointerEvent } from "react";

import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

import { btnAction, btnNeutral } from "../../collect/channels/styles";
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
import { textChunks, type EndedReason, type InputBody, type TouchPoint } from "./protocol";
import { isTouchDevice, useRemoteConnection } from "./useRemoteConnection";

const ENDED_TEXT: Record<EndedReason, string> = {
  browser: "서버 브라우저가 닫혔어요. 작업 화면을 다시 열면 다시 준비해요.",
  run: "이 작업의 서버 브라우저 실행이 바뀌었어요. 작업 상태를 다시 읽고 있어요.",
  unreachable: "서버 브라우저에 닿지 못했어요. 작업 화면을 다시 열면 다시 준비해요.",
  replaced: "다른 기기나 탭에서 이 인증 화면을 열어서 여기 화면은 닫혔어요. 여기서 계속하려면 다시 열어 주세요.",
};

/**
 * The remote screen of a job: the server browser's page drawn into the area,
 * and the pointer, touch and keyboard input of this device relayed to it.
 *
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
 */
export function RemoteScreen({
  jobId,
  run,
  bound,
  headingId,
  title,
  opening,
  onReopen,
  actions,
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
  /** More controls of the screen, before its own (a find job's `이 창 닫기`). */
  actions?: React.ReactNode;
}) {
  const area = useRef<HTMLDivElement>(null);
  const image = useRef<HTMLImageElement>(null);
  const field = useRef<HTMLTextAreaElement>(null);
  const refresh = useRef<HTMLButtonElement>(null);
  const [attempt, setAttempt] = useState(0);
  const [touch] = useState(isTouchDevice);
  const [typing, setTyping] = useState(false);
  const conn = useRemoteConnection({ jobId, run, bound, attempt, area, image });
  const { send, session } = conn;

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

  const { phase, frame, planned, reason } = conn;
  const live = phase === "live";
  const aspect = frame ?? planned;
  // A page that ended while the screen was ready: the person asks for it again.
  const reopen = () => {
    void onReopen().then(() => setAttempt((n) => n + 1));
  };

  return (
    <>
      <div className="mb-3 flex flex-wrap items-center gap-x-2.5 gap-y-1.5 max-[720px]:mb-2">
        <h2 id={headingId} className="text-[17px] font-bold">
          {title}
        </h2>
        <div className="ml-auto flex items-center gap-2">
          {actions}
          {touch && (
            <Button
              type="button"
              variant="ghost"
              className={btnNeutral}
              aria-pressed={typing}
              disabled={!live}
              onClick={showKeyboard}
            >
              키보드
            </Button>
          )}
          <Button ref={refresh} type="button" variant="ghost" className={btnNeutral} disabled={!live} onClick={conn.reload}>
            새로고침
          </Button>
        </div>
      </div>

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
            className={cn("pointer-events-none absolute inset-0 size-full", !frame && "invisible", phase !== "live" && "opacity-40")}
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

        {phase !== "live" && (
          <div
            role="status"
            className="absolute inset-0 flex flex-col items-center justify-center gap-3 bg-surface-1/60 p-4 text-center text-[13.5px] leading-relaxed text-text-secondary"
          >
            {phase === "connecting" && <p>화면을 불러오는 중이에요.</p>}
            {phase === "retrying" && <p>연결이 끊겨서 다시 연결하고 있어요.</p>}
            {phase === "failed" && (
              <>
                <p>서버 브라우저에 다시 연결하지 못했어요.</p>
                <Button type="button" variant="ghost" className={btnAction} onClick={() => setAttempt((n) => n + 1)}>
                  다시 연결
                </Button>
              </>
            )}
            {phase === "ended" && (
              <>
                <p>{ENDED_TEXT[reason ?? "browser"]}</p>
                <Button type="button" variant="ghost" className={btnAction} disabled={opening} onClick={reopen}>
                  다시 열기
                </Button>
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
