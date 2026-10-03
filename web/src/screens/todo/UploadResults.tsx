import type { ArchiveType, DroppedFile, JobFile, UploadKind } from "./api";
import { ItemBadge } from "./badges";
import { FORMAT_LABEL, sizeText } from "./format";

const KIND_LABEL: Record<UploadKind, string> = {
  subtitle: "자막",
  font: "폰트",
  archive: "압축 파일",
};

const ARCHIVE_LABEL: Record<ArchiveType, string> = {
  zip: "ZIP",
  rar: "RAR",
  "7z": "7z",
  gz: "gzip",
  bz2: "bzip2",
  xz: "xz",
  tar: "tar",
};

const ORDER: UploadKind[] = ["subtitle", "font", "archive"];

/**
 * `올린 파일` of an upload job, and `받은 파일` of a find job: the files it kept, grouped by what their content
 * was judged to be, and `뺀 파일` with each name and the reason. A ZIP is kept
 * whole; which of its files are subtitles and fonts waits for the analysis.
 */
export function UploadResults({
  files,
  dropped,
  empty = "남은 파일이 없어요.",
}: {
  files: readonly JobFile[];
  dropped: readonly DroppedFile[];
  /** What it says when no file was kept. */
  empty?: string;
}) {
  const groups = ORDER.map((kind) => ({ kind, files: files.filter((f) => f.kind === kind) })).filter((g) => g.files.length > 0);
  return (
    <div className="flex flex-col gap-3">
      {groups.length === 0 ? (
        <p className="text-[13px] text-text-muted">{empty}</p>
      ) : (
        groups.map((group) => (
          <section key={group.kind} className="flex min-w-0 flex-col gap-1.5">
            <h3 className="text-[14px] font-bold">
              {KIND_LABEL[group.kind]} <span className="font-semibold text-text-muted">{group.files.length}개</span>
            </h3>
            {group.kind === "archive" && (
              <p className="text-xs leading-snug text-text-muted">
                압축 파일은 묶음으로 받았어요. 안의 분류는 묶음 분석을 기다려요. 나뉜 압축 파일은 조각마다 따로 받아요.
              </p>
            )}
            <ul className="m-0 flex list-none flex-col gap-2 rounded-card border border-hairline bg-surface-1 px-3.5 py-3 shadow-(--card-shadow)">
              {group.files.map((file, i) => (
                <FileLine key={`${file.name}:${i}`} file={file} />
              ))}
            </ul>
          </section>
        ))
      )}
      {dropped.length > 0 && (
        <section className="flex min-w-0 flex-col gap-1.5">
          <h3 className="text-[14px] font-bold">
            뺀 파일 <span className="font-semibold text-text-muted">{dropped.length}개</span>
          </h3>
          <ul className="m-0 flex list-none flex-col gap-2 rounded-card border border-hairline bg-surface-1 px-3.5 py-3 shadow-(--card-shadow)">
            {dropped.map((file, i) => (
              <li key={`${file.name}:${i}`} className="flex min-w-0 flex-col gap-0.5">
                <span className="min-w-0 text-[13px] leading-snug font-semibold [overflow-wrap:anywhere]">{file.name}</span>
                <span className="text-xs leading-snug text-text-secondary">{file.reason}</span>
              </li>
            ))}
          </ul>
        </section>
      )}
    </div>
  );
}

function FileLine({ file }: { file: JobFile }) {
  // A font and a subtitle that is only stored have no format of their own to name.
  const format =
    file.kind === "archive"
      ? `${file.archive !== null ? ARCHIVE_LABEL[file.archive] : "압축 파일"} · 묶음 분석을 기다려요`
      : file.kind === "font"
        ? "폰트"
        : file.format === "other"
          ? "보관만 하는 형식"
          : file.format !== null
            ? FORMAT_LABEL[file.format]
            : null;
  const facts = [file.size !== null ? sizeText(file.size) : null, format].filter((fact): fact is string => fact !== null);
  return (
    <li className="flex min-w-0 flex-col gap-1">
      <div className="flex min-w-0 flex-wrap items-center gap-x-2.5 gap-y-1">
        <span className="min-w-0 text-[13px] leading-snug font-semibold [overflow-wrap:anywhere]">{file.name}</span>
        {facts.length > 0 && <span className="text-xs whitespace-nowrap text-text-muted">{facts.join(" · ")}</span>}
        {file.state !== "done" && <ItemBadge shown={file.state === "receiving" ? "receive" : file.state} />}
      </div>
      {file.path !== null && <p className="text-xs leading-snug text-text-muted [overflow-wrap:anywhere]">{file.path}</p>}
    </li>
  );
}
