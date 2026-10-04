import { useId } from "react";

import { Button } from "@/components/ui/button";

import { btnAction } from "../collect/channels/styles";
import type { JobScreen } from "./api";
import { RemoteScreen } from "./remote/RemoteScreen";
import type { ScreenPrepare } from "./useScreenPrepare";

const TITLE = "인증";
const FIND_TITLE = "직접 찾기";

/**
 * The job's check on the site, done on the server browser's page from here
 * (`docs/specs/jobs.md`, 작업 화면 안의 인증과 브라우저 수명): the remote screen
 * when a browser run shows the check, else what the screen is waiting for. A
 * closed screen is prepared again by `다시 열기`, which is the person asking;
 * nothing here asks for a browser by itself. A find job (`find`) shows the
 * creator's posts there for the person to browse, under `직접 찾기`; `children`
 * come under the screen (its `받기 끝내기`). The screen has the same browser
 * controls for either: back, forward, reload, the host of the page and the
 * tabs of the run ({@link RemoteScreen}).
 */
export function JobAuth({
  jobId,
  screen,
  prepare,
  find = false,
  children,
}: {
  jobId: string;
  screen: JobScreen;
  prepare: ScreenPrepare;
  find?: boolean;
  children?: React.ReactNode;
}) {
  const headingId = useId();
  const title = find ? FIND_TITLE : TITLE;
  return (
    <section aria-labelledby={headingId} className="mt-6 max-[720px]:mt-4">
      {screen.state === "ready" && screen.run !== null ? (
        <RemoteScreen
          jobId={jobId}
          run={screen.run}
          bound={screen.bound}
          headingId={headingId}
          title={title}
          opening={prepare.opening}
          onReopen={prepare.open}
        />
      ) : (
        <>
          <h2 id={headingId} className="mb-3 text-[17px] font-bold max-[720px]:mb-2">
            {title}
          </h2>
          <Notice screen={screen} prepare={prepare} find={find} />
        </>
      )}
      {prepare.error !== null && (
        <p role="alert" className="mt-2 text-[13px] font-semibold text-urgent">
          {prepare.error}
        </p>
      )}
      {children}
    </section>
  );
}

function Notice({ screen, prepare, find }: { screen: JobScreen; prepare: ScreenPrepare; find: boolean }) {
  const box =
    "flex flex-col items-center gap-3 rounded-card border border-dashed border-hairline px-4 py-6 text-center text-[13.5px] leading-relaxed text-text-secondary";
  switch (screen.state) {
    case "closed":
      return (
        <div className={box}>
          <p className="m-0">{screen.note ?? "서버 브라우저가 닫혀 있어요. 작업 화면을 다시 열면 다시 준비해요."}</p>
          <Button type="button" variant="ghost" className={btnAction} disabled={prepare.opening} onClick={() => void prepare.open()}>
            다시 열기
          </Button>
        </div>
      );
    case "unavailable":
      return (
        <p className={box}>{screen.note ?? "이 웹 서버는 서버 브라우저에 연결돼 있지 않아 원격 화면을 보여줄 수 없어요."}</p>
      );
    default:
      return (
        <p role="status" className={box}>
          {find
            ? "서버 브라우저에서 제작자의 게시물을 열고 있어요. 준비되면 이 자리에 나타나요."
            : "서버 브라우저에서 인증 화면을 준비하고 있어요. 준비되면 이 자리에 나타나요."}
        </p>
      );
  }
}
