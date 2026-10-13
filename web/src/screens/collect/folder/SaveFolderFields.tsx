import { useEffect, useId, useRef, useState, type FocusEvent } from "react";

import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { useCached } from "@/lib/cached";
import { cn } from "@/lib/utils";

import { KEYS } from "../cache";
import { hintClass, inputClass, labelClass } from "../channels/styles";
import { optionClass } from "../subs/add/draft";
import { fetchFolderWorks, fetchSuggestion, type FolderSuggestion, type FolderWork } from "./api";
import {
  folderOf,
  matchingWorks,
  parseDirectory,
  parseSeason,
  workProblem,
  type AnimeOfFolder,
  type Asked,
} from "./saveFolder";

/** The folder the fields hold now, or why they hold none yet (the folder is then empty). */
export interface FolderChange {
  directory: string;
  problem: string | null;
}

/**
 * The save folder as a work field and a season field, `<work>/Season NN`.
 * The work field offers the works of the collect and archive folders as it is
 * typed in; picking one fills the season with the server's suggestion. With a
 * `release`, a folder still empty is filled from it once, when the fields
 * open. A rule screen (`allowPath`) can also take the folder as a path typed
 * whole, and a folder in another form opens that way.
 *
 * The fields keep what is typed themselves: a parent that sets the folder from
 * outside (a revert, a newer version) mounts them again with a new `key`.
 * Every change, and the opening itself, is told through `onChange`.
 */
export function SaveFolderFields({
  directory,
  onChange,
  release = null,
  anime = null,
  allowPath = false,
  label = null,
}: {
  directory: string;
  onChange: (change: FolderChange) => void;
  /** The release's work the folder is suggested from while it is empty. */
  release?: string | null;
  /** The anime the folder is for: its start date places the airing, else today does. */
  anime?: AnimeOfFolder | null;
  allowPath?: boolean;
  /** A visible name for the two fields together. */
  label?: string | null;
}) {
  const uid = useId();
  const works = useCached<FolderWork[]>(KEYS.folderWorks, fetchFolderWorks, "라이브러리 작품을 불러오지 못했어요.");
  const [opened] = useState(() => parseDirectory(directory));
  const [mode, setMode] = useState<"split" | "path">(() =>
    opened === null && allowPath && directory.trim() !== "" ? "path" : "split",
  );
  const [work, setWork] = useState(() => opened?.work ?? (allowPath ? "" : directory.trim()));
  const [season, setSeason] = useState(() => (opened ? String(opened.season) : "1"));
  const [path, setPath] = useState(directory);
  const [reason, setReason] = useState<string | null>(null);
  const [suggesting, setSuggesting] = useState(false);
  const [listing, setListing] = useState(false);
  // Once the person typed or picked, a late suggestion from the release is not applied.
  const touched = useRef(false);
  const asking = useRef<AbortController | null>(null);
  const onChangeRef = useRef(onChange);
  onChangeRef.current = onChange;
  const animeRef = useRef(anime);
  animeRef.current = anime;

  const ask = (asked: Asked, apply: (found: FolderSuggestion) => void) => {
    asking.current?.abort();
    const controller = new AbortController();
    asking.current = controller;
    setSuggesting(true);
    fetchSuggestion(asked, animeRef.current, controller.signal)
      .then(
        (found) => {
          if (!controller.signal.aborted && found !== null) apply(found);
        },
        // No suggestion is no harm: the fields stay as they are.
        () => undefined,
      )
      .finally(() => {
        if (!controller.signal.aborted) setSuggesting(false);
      });
  };
  const stopAsking = () => {
    asking.current?.abort();
    setSuggesting(false);
  };

  useEffect(() => {
    // The parent learns from the start whether the fields hold a folder.
    if (mode === "split") onChangeRef.current(folderOf(work, season));
    if (release !== null && directory.trim() === "") {
      ask({ release }, (found) => {
        if (touched.current) return;
        setWork(found.work);
        setSeason(String(found.season));
        setReason(found.reason);
        onChangeRef.current({ directory: found.directory, problem: null });
      });
    }
    return () => asking.current?.abort();
    // Only when the fields open: another release or anime mounts them again.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const typeWork = (text: string) => {
    touched.current = true;
    stopAsking();
    setWork(text);
    setReason(null);
    setListing(true);
    onChange(folderOf(text, season));
  };
  const typeSeason = (text: string) => {
    touched.current = true;
    stopAsking();
    setSeason(text);
    setReason(null);
    onChange(folderOf(work, text));
  };
  const pick = (picked: FolderWork) => {
    touched.current = true;
    setWork(picked.name);
    setReason(null);
    setListing(false);
    onChange(folderOf(picked.name, season));
    ask({ work: picked.name }, (found) => {
      setSeason(String(found.season));
      setReason(found.reason);
      onChangeRef.current(folderOf(picked.name, String(found.season)));
    });
  };
  const toPath = () => {
    stopAsking();
    const made = folderOf(work, season);
    const text = made.problem === null ? made.directory : path;
    setPath(text);
    setMode("path");
    onChange({ directory: text, problem: null });
  };
  const toSplit = () => {
    stopAsking();
    const parsed = parseDirectory(path.trim());
    const nextWork = parsed?.work ?? path.trim().split(/[\\/]/).find((part) => part !== "" && part !== ".") ?? "";
    const nextSeason = parsed ? String(parsed.season) : season;
    setWork(nextWork);
    setSeason(nextSeason);
    setReason(null);
    setMode("split");
    onChange(folderOf(nextWork, nextSeason));
  };
  const leave = (event: FocusEvent<HTMLDivElement>) => {
    if (!event.currentTarget.contains(event.relatedTarget as Node | null)) setListing(false);
  };

  const all = works.data ?? [];
  const chosen = all.find((w) => w.name === work.trim()) ?? null;
  const matches = chosen === null ? matchingWorks(all, work) : [];
  const made = folderOf(work, season);
  const listId = `${uid}-works`;

  return (
    <div role="group" aria-labelledby={label ? `${uid}-label` : undefined} className="flex min-w-0 flex-col gap-1.5">
      {label && (
        <span id={`${uid}-label`} className={labelClass}>
          {label}
        </span>
      )}
      {mode === "split" ? (
        <>
          <div className="grid min-w-0 grid-cols-[minmax(0,1fr)_76px] gap-2.5" onBlur={leave}>
            <div className="flex min-w-0 flex-col gap-1.5">
              <Label htmlFor={`${uid}-work`} className={labelClass}>
                작품
              </Label>
              <Input
                id={`${uid}-work`}
                value={work}
                onChange={(e) => typeWork(e.target.value)}
                onFocus={() => setListing(true)}
                onKeyDown={(e) => {
                  if (e.key === "Escape") setListing(false);
                }}
                role="combobox"
                aria-controls={listing && matches.length > 0 ? listId : undefined}
                aria-expanded={listing && matches.length > 0}
                aria-autocomplete="list"
                aria-invalid={workProblem(work) !== null}
                autoComplete="off"
                spellCheck={false}
                placeholder="작품 폴더 이름"
                className={inputClass}
              />
            </div>
            <div className="flex min-w-0 flex-col gap-1.5">
              <Label htmlFor={`${uid}-season`} className={labelClass}>
                시즌
              </Label>
              <Input
                id={`${uid}-season`}
                value={season}
                onChange={(e) => typeSeason(e.target.value)}
                inputMode="numeric"
                autoComplete="off"
                aria-invalid={parseSeason(season) === null}
                className={`${inputClass} font-mono`}
              />
            </div>
            {listing && matches.length > 0 && (
              <ul id={listId} aria-label="라이브러리 작품" className="col-span-2 m-0 flex list-none flex-col gap-1.5 p-0">
                {matches.map((w) => (
                  <li key={w.name}>
                    <button
                      type="button"
                      // Keeps the focus where it is, so the list does not close before the click lands.
                      onMouseDown={(e) => e.preventDefault()}
                      onClick={() => pick(w)}
                      className={cn(optionClass, "border-hairline-soft py-2")}
                    >
                      <span className="min-w-0 text-[13.5px] leading-snug font-semibold break-words">{w.title}</span>
                      <span className="flex min-w-0 flex-wrap gap-x-2 text-xs text-text-muted">
                        {w.title !== w.name && <span className="min-w-0 font-mono break-all">{w.name}</span>}
                        {w.archived && <span>보관 폴더</span>}
                      </span>
                    </button>
                  </li>
                ))}
              </ul>
            )}
          </div>
          <p className={made.problem !== null && !suggesting ? `${hintClass} font-semibold text-urgent` : hintClass} aria-live="polite">
            {made.problem !== null
              ? suggesting
                ? "저장 폴더를 제안하는 중이에요."
                : made.problem
              : chosen !== null
                ? `${chosen.archived ? "보관 폴더에 있는 " : ""}‘${chosen.title}’ 작품의 시즌 폴더에 받아요.`
                : "라이브러리에 없는 이름이라 새 작품 폴더를 만들어요."}
          </p>
          {reason !== null && <p className={hintClass}>{reason}</p>}
        </>
      ) : (
        <>
          <Label htmlFor={`${uid}-path`} className={labelClass}>
            저장 폴더 경로
          </Label>
          <Input
            id={`${uid}-path`}
            value={path}
            onChange={(e) => {
              setPath(e.target.value);
              onChange({ directory: e.target.value, problem: null });
            }}
            placeholder="작품 제목/Season 01"
            autoComplete="off"
            spellCheck={false}
            className={`${inputClass} font-mono`}
          />
        </>
      )}
      {allowPath && (
        <button
          type="button"
          onClick={mode === "split" ? toPath : toSplit}
          className="self-start text-[12.5px] font-semibold text-focus underline underline-offset-2"
        >
          {mode === "split" ? "경로 직접 적기" : "작품과 시즌으로 적기"}
        </button>
      )}
    </div>
  );
}
