import { useState } from "react";
import { Link } from "react-router-dom";

import { refreshTodoCount } from "@/app/todo-count";
import { Button } from "@/components/ui/button";
import { ApiError } from "@/lib/api";
import { store } from "@/lib/cached";

import { btnAction } from "../collect/channels/styles";
import { filesPath } from "../library/storage";
import { checkVideo, fetchTodos, type VideoCheckTodo } from "./api";
import { KEYS } from "./poll";

/** Where a video's `회차 확인 필요` card goes: the work's 파일 card, which lists the video. */
export function videoCheckPath(todo: VideoCheckTodo): string {
  return filesPath(`/library/${encodeURIComponent(todo.work?.id ?? "")}`);
}

/**
 * The buttons of a video's `회차 확인 필요` card: `파일 보기` opens the work's 파일 card, and `확인함` says the person
 * has seen the video, so it is no longer asked about. Whatever the answer, the to-dos and the menu count are read
 * again: a video that changed since is asked about anew (its card stays, saying why), one renamed or moved is gone.
 * `onRefresh` then reads again what the page shows (the work detail's 파일 card); the message under the buttons is only
 * about the check, so a refresh that fails says nothing here.
 */
export function VideoCheckActions({ todo, onRefresh }: { todo: VideoCheckTodo; onRefresh?: () => Promise<void> }) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const work = todo.work;

  const check = async () => {
    if (busy || work === null) return;
    setBusy(true);
    setError(null);
    try {
      await checkVideo(work.id, todo.path, todo.seen);
    } catch (e) {
      setError(e instanceof ApiError ? e.message : "확인함을 보내지 못했어요. 잠시 뒤 다시 시도해 주세요.");
    }
    try {
      store(KEYS.todos, await fetchTodos());
    } catch {
      // The screen's polling reads the list again.
    }
    refreshTodoCount();
    try {
      await onRefresh?.();
    } catch {
      // The page keeps what it showed until its next read.
    }
    setBusy(false);
  };

  return (
    <div className="flex min-w-0 flex-col items-end gap-1.5">
      {error !== null && (
        <p role="alert" className="m-0 text-[12.5px] leading-snug font-semibold text-urgent">
          {error}
        </p>
      )}
      <div className="flex flex-wrap justify-end gap-1.5">
        {work !== null && (
          <Button asChild variant="ghost" className={btnAction}>
            <Link to={videoCheckPath(todo)}>파일 보기</Link>
          </Button>
        )}
        <Button
          type="button"
          variant="ghost"
          className={btnAction}
          disabled={busy || work === null}
          onClick={() => void check()}
        >
          확인함
        </Button>
      </div>
    </div>
  );
}
