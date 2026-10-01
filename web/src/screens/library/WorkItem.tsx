import { memo } from "react";
import { Link } from "react-router-dom";

import { cn } from "@/lib/utils";

import { workPath } from "./api";
import { QuestionIcon } from "./icons";
import { subtitleLine, videoLine, type Work } from "./model";
import type { ViewKey } from "./prefs";

/**
 * Where a work opens from the library; the detail screen reads it to go back
 * with the browser's own back step (which keeps the list's scroll position).
 */
export const FROM_LIBRARY = { from: "library" } as const;

/**
 * The cover until artwork exists: the first letter of the title on a block
 * tinted by the title, so the same work looks the same every time. Decorative
 * (the title is next to it as text). An `imageUrl` (no work has one yet) is
 * read lazily, only once the cover is near the viewport, and covers the
 * placeholder, which stays underneath while it loads or if it fails.
 */
export function Cover({
  work,
  className,
  letterClass,
  imageUrl,
}: {
  work: Pick<Work, "hue" | "initial">;
  className: string;
  letterClass: string;
  imageUrl?: string | null;
}) {
  return (
    <span
      aria-hidden="true"
      className={cn("relative flex flex-none items-center justify-center overflow-hidden rounded-lg shadow-(--poster-shadow)", className)}
      style={{
        backgroundColor: `color-mix(in srgb, hsl(${work.hue} 52% 52%) 30%, var(--surface-2))`,
        color: `color-mix(in srgb, hsl(${work.hue} 70% 62%) 78%, var(--text-primary))`,
      }}
    >
      <span className={cn("font-bold select-none", letterClass)}>{work.initial}</span>
      {imageUrl && (
        <img src={imageUrl} alt="" loading="lazy" decoding="async" className="absolute inset-0 size-full object-cover" />
      )}
    </span>
  );
}

/** A subtitle file the scan could not attach to an episode: shown beside, not inside, the confirmed range. */
function CheckBadge() {
  return (
    <span className="inline-flex flex-none items-center gap-1 rounded-full border border-focus px-1.5 py-px text-[11.5px] leading-tight font-semibold whitespace-nowrap text-focus">
      <QuestionIcon className="size-3" />
      자막 확인 필요
    </span>
  );
}

function VideoLine({ work }: { work: Work }) {
  return (
    <p className={cn("min-w-0 text-[12.5px] leading-snug", work.missing ? "font-semibold text-focus" : "text-text-secondary")}>
      {videoLine(work)}
    </p>
  );
}

function SubtitleLine({ work }: { work: Work }) {
  const quiet = work.missing || work.subtitle.length === 0;
  return (
    <p className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 text-[12.5px] leading-snug">
      <span className={quiet ? "text-text-muted" : "text-text-secondary"}>{subtitleLine(work)}</span>
      {work.subtitle_check_needed && <CheckBadge />}
    </p>
  );
}

/**
 * The whole item is the link to the work. Hovering lightens the item's
 * background (no underline on the title).
 */
const item =
  "block rounded-card bg-surface-2 text-text-primary no-underline hover:bg-surface-1 hover:no-underline dark:bg-surface-1 dark:hover:bg-surface-3";

export const GridItem = memo(function GridItem({ work }: { work: Work }) {
  return (
    <li className="min-w-0">
      <Link to={workPath(work.id)} state={FROM_LIBRARY} className={cn(item, "flex h-full flex-col gap-2.5 p-2.5")}>
        <Cover work={work} className="aspect-[2/3] w-full" letterClass="text-5xl" />
        <span className="flex min-w-0 flex-col gap-1 px-0.5 pb-0.5">
          <span className="line-clamp-2 min-w-0 text-sm leading-snug font-semibold">{work.title}</span>
          <VideoLine work={work} />
          <SubtitleLine work={work} />
        </span>
      </Link>
    </li>
  );
});

export const ListItem = memo(function ListItem({ work }: { work: Work }) {
  return (
    <li className="min-w-0">
      <Link
        to={workPath(work.id)}
        state={FROM_LIBRARY}
        className={cn(
          item,
          "grid items-center gap-x-4 gap-y-1 px-3 py-2.5",
          "grid-cols-[48px_minmax(0,3fr)_minmax(7rem,1.4fr)_minmax(9rem,2.2fr)]",
          "max-[720px]:grid-cols-[48px_minmax(0,1fr)] max-[720px]:gap-x-3",
        )}
      >
        <Cover work={work} className="h-[72px] w-12 rounded-md max-[720px]:row-span-3" letterClass="text-xl" />
        <span className="line-clamp-2 min-w-0 text-[14.5px] leading-snug font-semibold">{work.title}</span>
        <VideoLine work={work} />
        <SubtitleLine work={work} />
      </Link>
    </li>
  );
});

/** The grid or the list of works. */
export function WorkItems({ works, view }: { works: readonly Work[]; view: ViewKey }) {
  return view === "grid" ? (
    <ul
      aria-label="작품 목록"
      className="m-0 grid list-none grid-cols-[repeat(auto-fill,minmax(150px,1fr))] gap-3 p-0 max-[720px]:grid-cols-[repeat(auto-fill,minmax(136px,1fr))] max-[720px]:gap-2.5"
    >
      {works.map((work) => (
        <GridItem key={work.id} work={work} />
      ))}
    </ul>
  ) : (
    <ul aria-label="작품 목록" className="m-0 flex list-none flex-col gap-1.5 p-0">
      {works.map((work) => (
        <ListItem key={work.id} work={work} />
      ))}
    </ul>
  );
}
