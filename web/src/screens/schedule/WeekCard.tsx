import { useState, type ComponentType, type ComponentProps } from "react";
import { Link } from "react-router-dom";

import { cn } from "@/lib/utils";

import { LockIcon } from "../collect/icons";
import { CheckIcon, MinusIcon, QuestionIcon } from "../library/icons";
import { workPath } from "../library/api";
import type { Card, SubtitleState, VideoState } from "./api";
import { DownloadIcon, SubtitleIcon } from "./icons";

type SvgIcon = ComponentType<ComponentProps<"svg">>;

/**
 * How a status line looks. Only the ones that ask something of the user, and
 * the video being received, are emphasised; a received file is green; waiting
 * for a release or a creator is quiet.
 */
type Tone = "ok" | "active" | "quiet" | "check" | "urgent";

interface Line {
  text: string;
  tone: Tone;
  icon: SvgIcon;
}

const VIDEO: Record<VideoState, Line> = {
  received: { text: "영상 받음", tone: "ok", icon: CheckIcon },
  downloading: { text: "영상 받는 중", tone: "active", icon: DownloadIcon },
  waiting: { text: "영상 대기", tone: "quiet", icon: MinusIcon },
  upcoming: { text: "방영 전", tone: "quiet", icon: MinusIcon },
  paused: { text: "받기 멈춤", tone: "quiet", icon: MinusIcon },
};

const SUBTITLE: Record<SubtitleState, Line> = {
  received: { text: "자막 받음", tone: "ok", icon: CheckIcon },
  waiting: { text: "자막 대기", tone: "quiet", icon: MinusIcon },
  // The states the subtitle side will fill.
  downloading: { text: "자막 받는 중", tone: "quiet", icon: DownloadIcon },
  auth_required: { text: "인증 필요", tone: "urgent", icon: LockIcon },
  episode_unconfirmed: { text: "회차 확인 필요", tone: "check", icon: QuestionIcon },
};

const TONE: Record<Tone, string> = {
  ok: "text-text-secondary [&>svg]:text-ok",
  active: "rounded-full border border-focus px-2 py-px font-bold text-focus",
  quiet: "text-text-muted",
  check: "rounded-full border border-focus px-2 py-px font-bold text-focus",
  urgent: "rounded-full border border-urgent px-2 py-px font-bold text-urgent",
};

function StatusLine({ line, detail }: { line: Line; detail?: string | null }) {
  return (
    <p className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-0.5 text-[12.5px] leading-snug">
      <span className={cn("inline-flex min-w-0 items-center gap-1", TONE[line.tone])} data-tone={line.tone}>
        <line.icon className="size-3 flex-none" />
        {line.text}
      </span>
      {detail && (
        <span className="inline-flex min-w-0 items-center gap-1 text-text-muted">
          <SubtitleIcon className="size-3 flex-none" />
          <span className="min-w-0 break-words">{detail}</span>
        </span>
      )}
    </p>
  );
}

/** The cover when the card's season has one, over an empty slot that stays when it has none. */
function CoverSlot({ url }: { url: string | null }) {
  const [failed, setFailed] = useState(false);
  return (
    <span
      aria-hidden="true"
      data-testid="cover-slot"
      className="relative block aspect-[2/3] w-12 flex-none overflow-hidden rounded-lg bg-surface-3 shadow-(--poster-shadow)"
    >
      {url && !failed && (
        <img
          key={url}
          src={url}
          alt=""
          loading="lazy"
          decoding="async"
          onError={() => setFailed(true)}
          className="absolute inset-0 size-full object-cover"
        />
      )}
    </span>
  );
}

/** Where a card opens: the work when its season is connected, otherwise the subscription's rule. */
export const cardPath = (card: Card) =>
  card.work_id ? workPath(card.work_id) : `/collect/rules?rule=${encodeURIComponent(card.rule_id)}`;

/**
 * One subscribed anime's episode of the week. The whole card is the link, and
 * hovering lightens its background (no underline on the title).
 */
export function WeekCard({ card }: { card: Card }) {
  return (
    <li className="min-w-0">
      <Link
        to={cardPath(card)}
        className="flex h-full min-w-0 gap-3 rounded-card bg-surface-2 p-2.5 text-text-primary no-underline hover:bg-surface-1 hover:no-underline dark:bg-surface-1 dark:hover:bg-surface-3"
      >
        <CoverSlot url={card.cover_url} />
        <span className="flex min-w-0 flex-1 flex-col gap-1">
          <span className="line-clamp-2 min-w-0 text-sm leading-snug font-semibold">{card.title}</span>
          <span className="flex min-w-0 flex-wrap items-baseline gap-x-2.5 text-[12.5px] leading-snug">
            <span className="text-text-muted">{card.time ?? "시각 미정"}</span>
            {card.episode !== null && <strong className="font-bold">{card.episode}화</strong>}
          </span>
          <StatusLine line={VIDEO[card.video]} />
          {card.subtitle && (
            <StatusLine line={SUBTITLE[card.subtitle]} detail={card.subtitle === "waiting" ? card.creator : null} />
          )}
        </span>
      </Link>
    </li>
  );
}
