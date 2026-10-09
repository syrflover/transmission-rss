import { Link } from "react-router-dom";

import { useCached } from "@/lib/cached";
import { sizeText } from "@/lib/size";

import { STORAGE_KEY, workPath } from "../../library/api";
import { cleanableText, filesPath, kindTexts, type StorageOverview, type StorageWork } from "../../library/storage.ts";
import { findItem } from "../items";
import { BTN, Facts, Tag } from "../parts";
import { loadStorage } from "./api";

const LOAD_FAILED = "파일 용량을 불러오지 못했어요.";

/** One work: its name and stored size, what the size is made of, and how many files can be cleaned. The row opens its 파일 card. */
function WorkRow({ work }: { work: StorageWork }) {
  const kinds = kindTexts(work.kinds);
  const cleanable = cleanableText(work.cleanable);
  return (
    <Link
      to={filesPath(workPath(work.id))}
      className="flex min-h-14 flex-col gap-2 rounded-card border border-hairline bg-surface-2 p-3.5 hover:bg-surface-3 focus-visible:outline-2 focus-visible:outline-focus"
    >
      <span className="flex flex-wrap items-baseline justify-between gap-x-4 gap-y-1">
        <span className="min-w-0 flex-1 basis-48 text-sm font-bold [overflow-wrap:anywhere]">{work.name}</span>
        <span className="text-[13px] font-semibold text-text-secondary">{sizeText(work.total)}</span>
      </span>
      {(kinds.length > 0 || cleanable !== null) && (
        <Facts>
          {kinds.map((kind) => (
            <span key={kind.kind} className="text-[13px] text-text-secondary">
              {kind.text}
            </span>
          ))}
          {cleanable !== null && <Tag tone="pending">{cleanable}</Tag>}
        </Facts>
      )}
    </Link>
  );
}

/**
 * `파일 용량·정리`: the works that have stored files or a cover, the biggest first, each with its size per kind and
 * how many stored subtitles can be cleaned. A row opens that work's 파일 card; nothing is deleted here, because a
 * clean is done one file at a time there (docs/specs/library.md, 보관 파일의 정리).
 */
export function StoragePanel() {
  const storage = useCached<StorageOverview>(STORAGE_KEY, loadStorage, LOAD_FAILED);
  const data = storage.data;
  return (
    <div className="flex flex-col gap-5 p-5 max-[720px]:p-4">
      <header className="flex flex-col gap-1.5">
        <h2 id="panel-title" tabIndex={-1} className="m-0 text-xl font-bold outline-none">
          파일 용량·정리
        </h2>
        <p className="max-w-[62ch] text-[13.5px] leading-relaxed text-text-secondary">
          작품마다 보관한 자막, 폰트, 첨부, 표지의 용량이에요. 정리는 여기서 하지 않고, 작품을 열어 파일 카드에서 파일 하나씩 해요.
        </p>
      </header>

      {data === undefined && storage.error !== null && (
        <div className="flex flex-col items-start gap-2.5">
          <p role="alert" className="text-[13px] font-semibold text-urgent">
            {storage.error}
          </p>
          <button type="button" className={BTN.plain} onClick={storage.reload}>
            재시도
          </button>
        </div>
      )}
      {data === undefined && storage.error === null && storage.slow && <p className="text-[13px] text-text-muted">불러오는 중이에요.</p>}
      {data !== undefined &&
        (data.works.length === 0 ? (
          <p className="rounded-card border border-dashed border-hairline px-4 py-6 text-center text-[13.5px] leading-relaxed text-text-muted">
            {findItem("storage")?.empty}
          </p>
        ) : (
          <ul className="m-0 flex list-none flex-col gap-2.5 p-0">
            {data.works.map((work) => (
              <li key={work.id} className="min-w-0">
                <WorkRow work={work} />
              </li>
            ))}
          </ul>
        ))}
    </div>
  );
}
