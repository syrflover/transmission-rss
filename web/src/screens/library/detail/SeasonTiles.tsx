import { useRef, useState } from "react";

import { Sheet, SheetContent, SheetTrigger } from "@/components/ui/sheet";
import { cn } from "@/lib/utils";

import type { WorkSeason } from "../api";
import { CheckIcon } from "../icons";
import { summarize, type SeasonSummary } from "./model";

interface SeasonTilesProps {
  seasons: readonly WorkSeason[];
  /** The work's folder is gone: the lines are what was recorded, not what is there. */
  missing: boolean;
  selected: number;
  onSelect: (season: number) => void;
  /** A phone shows the current season and a sheet to change it. */
  phone: boolean;
}

const tileBox =
  "relative flex w-full min-w-0 flex-col gap-1.5 rounded-card border border-hairline bg-surface-1 px-3.5 py-3 text-left shadow-(--card-shadow)";

const tileSelected =
  "border-focus bg-[color-mix(in_srgb,var(--focus-ring)_10%,var(--surface-1))] shadow-[inset_0_0_0_1px_var(--focus-ring)]";

/**
 * What a tile says. The box, the label's weight and the check's slot are the
 * same whether or not the season is chosen, so choosing one never changes a
 * tile's size: only the border colour, the tint and the check's visibility do.
 */
function TileBody({ summary, selected }: { summary: SeasonSummary; selected: boolean }) {
  return (
    <>
      <span className="flex items-center justify-between gap-2">
        <span className="text-base font-bold">시즌 {summary.number}</span>
        <CheckIcon className={cn("size-[18px] flex-none text-focus", !selected && "invisible")} />
      </span>
      <span className="text-[12.5px] leading-snug text-text-secondary">{summary.video}</span>
      <span className="text-[12.5px] leading-snug text-text-secondary">{summary.subtitle}</span>
    </>
  );
}

function Tile({
  summary,
  selected,
  onSelect,
}: {
  summary: SeasonSummary;
  selected: boolean;
  onSelect: () => void;
}) {
  return (
    <button
      type="button"
      aria-pressed={selected}
      onClick={onSelect}
      className={cn(tileBox, "cursor-pointer hover:border-text-muted hover:bg-surface-2 dark:hover:bg-surface-2", selected && tileSelected)}
    >
      <TileBody summary={summary} selected={selected} />
    </button>
  );
}

/**
 * The seasons of a work. Wide: every season is a tile, the chosen one marked
 * with a check. Phone: the chosen season's tile and `시즌 변경`, which opens a
 * sheet with every season; choosing one closes it, and so does Escape, a press
 * on the backdrop or the close button, with focus back on `시즌 변경`.
 */
export function SeasonTiles({ seasons, missing, selected, onSelect, phone }: SeasonTilesProps) {
  const [open, setOpen] = useState(false);
  const list = useRef<HTMLDivElement>(null);
  const summaries = seasons.map((season) => summarize(season, missing));
  const current = summaries.find((s) => s.number === selected) ?? summaries[0];

  if (!phone) {
    return (
      <div role="group" aria-label="시즌" className="grid grid-cols-[repeat(auto-fit,minmax(200px,1fr))] gap-3">
        {summaries.map((summary) => (
          <Tile
            key={summary.number}
            summary={summary}
            selected={summary.number === selected}
            onSelect={() => onSelect(summary.number)}
          />
        ))}
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-2.5">
      <div className={cn(tileBox, tileSelected)} aria-label="현재 시즌" role="group">
        <TileBody summary={current} selected />
      </div>
      <Sheet open={open} onOpenChange={setOpen}>
        <SheetTrigger className="inline-flex min-h-11 w-full items-center justify-center rounded-full border border-hairline bg-surface-1 px-[15px] text-[13px] font-semibold text-text-primary hover:border-text-secondary hover:bg-surface-2">
          시즌 변경
        </SheetTrigger>
        <SheetContent
          title="시즌 변경"
          onOpenAutoFocus={(event) => {
            // The chosen season, so the keyboard starts where the choice is.
            const chosen = list.current?.querySelector<HTMLElement>("[aria-pressed='true']");
            if (chosen) {
              event.preventDefault();
              chosen.focus();
            }
          }}
        >
          <div ref={list} role="group" aria-label="시즌" className="flex flex-col gap-2.5">
            {summaries.map((summary) => (
              <Tile
                key={summary.number}
                summary={summary}
                selected={summary.number === selected}
                onSelect={() => {
                  onSelect(summary.number);
                  setOpen(false);
                }}
              />
            ))}
          </div>
        </SheetContent>
      </Sheet>
    </div>
  );
}
