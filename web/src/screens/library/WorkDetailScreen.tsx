import type { MouseEvent } from "react";
import { Link, useLocation, useNavigate, useParams } from "react-router-dom";

import { useCached } from "@/lib/cached";

import { EmptyState, ScreenFrame } from "../ScreenFrame";
import { loadWorks, WORKS_KEY, type LibraryWorkList } from "./api";
import { FROM_LIBRARY } from "./WorkItem";

/**
 * A work's page. Only its name for now: the seasons, episodes and files come
 * with the work detail ticket (0014). The way back is the browser's own back
 * step when the work was opened from the library, so the list is found as it
 * was (search, filter, scroll position).
 */
export function WorkDetailScreen() {
  const { workId = "" } = useParams();
  const location = useLocation();
  const navigate = useNavigate();
  const works = useCached<LibraryWorkList>(WORKS_KEY, loadWorks, "작품을 불러오지 못했어요.");
  const work = works.data?.works.find((w) => w.id === workId);
  const fromLibrary = (location.state as typeof FROM_LIBRARY | null)?.from === FROM_LIBRARY.from;

  const back = (event: MouseEvent<HTMLAnchorElement>) => {
    if (!fromLibrary) return;
    event.preventDefault();
    navigate(-1);
  };

  const backLink = (
    <Link
      to="/library"
      onClick={back}
      className="inline-flex min-h-9 items-center text-[13px] font-semibold text-text-secondary underline underline-offset-4 hover:text-text-primary"
    >
      라이브러리로
    </Link>
  );

  if (work) {
    return (
      <ScreenFrame title={work.name.normalize("NFC")}>
        {backLink}
      </ScreenFrame>
    );
  }
  return (
    <ScreenFrame title="작품">
      {works.data !== undefined ? (
        <div className="flex flex-col items-start gap-3">
          <EmptyState>이 작품을 찾지 못했어요. 폴더가 감시 폴더에서 빠졌을 수 있어요.</EmptyState>
          {backLink}
        </div>
      ) : (
        <div className="flex flex-col items-start gap-3">
          {works.error !== null && (
            <p role="alert" className="text-[13px] font-semibold text-urgent">
              {works.error}
            </p>
          )}
          {backLink}
        </div>
      )}
    </ScreenFrame>
  );
}
