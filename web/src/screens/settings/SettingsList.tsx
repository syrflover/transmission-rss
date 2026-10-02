import { Fragment } from "react";
import { Link } from "react-router-dom";

import { useCached } from "@/lib/cached";
import { cn } from "@/lib/utils";
import { KEYS } from "@/screens/collect/cache";

import { collectionSummary, loadCollection, type Collection } from "./collection/api";
import { foldersSummary, loadWatchFolders, type WatchFolderList } from "./folders/api";
import { ChevronIcon } from "./icons";
import { SETTINGS_ITEMS, type SettingsItem, type SettingsItemId } from "./items";
import { Facts, Tag } from "./parts";
import { loadPolicy, policySummary, POLICY_KEY, type Policy } from "./policy/api";
import type { ImportFlow } from "./import/useImportFlow";

/** The collect folder row: the two folders' names, or that none is chosen yet. */
function CollectionFacts() {
  // Shared with the panel: a save there updates this row at once.
  const { data } = useCached<Collection>(KEYS.collection, loadCollection, "");
  const summary = data ? collectionSummary(data) : null;
  return (
    <Facts>
      {data === undefined ? (
        <Tag>&nbsp;</Tag>
      ) : summary === null ? (
        <Tag tone="warn">정하지 않음</Tag>
      ) : (
        <span className="min-w-0 text-[13px] break-words text-text-secondary">{summary}</span>
      )}
    </Facts>
  );
}

/** The watch folders row: how many folders are registered, or that none is. */
function FoldersFacts() {
  // Shared with the panel: adding or unregistering a folder there updates this row at once.
  const { data } = useCached<WatchFolderList>(KEYS.watchFolders, loadWatchFolders, "");
  const summary = data ? foldersSummary(data) : null;
  return (
    <Facts>
      {data === undefined ? (
        <Tag>&nbsp;</Tag>
      ) : summary === null ? (
        <Tag tone="warn">등록한 폴더 없음</Tag>
      ) : (
        <span className="text-[13px] text-text-secondary">{summary}</span>
      )}
    </Facts>
  );
}

/** The common policy row: the format order, the idle time and the concurrent jobs. */
function PolicyFacts() {
  // Shared with the panel: a save there updates this row at once.
  const { data } = useCached<Policy>(POLICY_KEY, loadPolicy, "");
  return (
    <Facts>
      {data === undefined ? (
        <Tag>&nbsp;</Tag>
      ) : (
        <span className="min-w-0 text-[13px] break-words text-text-secondary">{policySummary(data)}</span>
      )}
    </Facts>
  );
}

/** What the row shows without opening the item: its current value. */
function RowFacts({ id, importFlow }: { id: SettingsItemId; importFlow: ImportFlow }) {
  if (id === "policy") return <PolicyFacts />;
  if (id === "collection") return <CollectionFacts />;
  if (id === "folders") return <FoldersFacts />;
  if (id === "import") {
    return (
      <Facts>
        <Tag>기존 YAML</Tag>
        {importFlow.step !== "pick" && <Tag tone="pending">{importFlow.step === "review" ? "검토 중" : "마침"}</Tag>}
      </Facts>
    );
  }
  return (
    <Facts>
      <Tag>준비 중</Tag>
    </Facts>
  );
}

function Row({
  item,
  current,
  importFlow,
}: {
  item: SettingsItem;
  current: boolean;
  importFlow: ImportFlow;
}) {
  const Icon = item.icon;
  return (
    <Link
      to={`/settings/${item.id}`}
      aria-current={current ? "true" : undefined}
      className={cn(
        "relative flex items-center gap-3 rounded-card border px-3.5 py-3 shadow-(--card-shadow)",
        current
          ? "border-focus bg-surface-3 before:absolute before:inset-y-2 before:left-0 before:w-[3px] before:rounded-full before:bg-focus"
          : "border-hairline bg-surface-1 hover:bg-surface-2",
      )}
    >
      <span className="grid size-9 flex-none place-items-center rounded-lg bg-surface-2 text-text-secondary">
        <Icon className="size-[18px]" />
      </span>
      <span className="flex min-w-0 flex-1 flex-col gap-1">
        <span className={cn("text-sm", current ? "font-bold" : "font-semibold")}>{item.title}</span>
        <RowFacts id={item.id} importFlow={importFlow} />
      </span>
      <ChevronIcon className="size-4 flex-none text-text-muted" />
    </Link>
  );
}

/** The item list: groups, each item with its current value. */
export function SettingsList({
  current,
  importFlow,
  className,
}: {
  current: SettingsItemId | null;
  importFlow: ImportFlow;
  className?: string;
}) {
  const groups: string[] = [];
  for (const item of SETTINGS_ITEMS) if (!groups.includes(item.group)) groups.push(item.group);

  return (
    <nav aria-label="설정 항목" className={cn("flex flex-col gap-2.5", className)}>
      {groups.map((group) => (
        <Fragment key={group}>
          <h2 className="m-0 mt-3 px-0.5 text-[13px] font-bold text-text-secondary first:mt-0">{group}</h2>
          {SETTINGS_ITEMS.filter((item) => item.group === group).map((item) => (
            <Row key={item.id} item={item} current={current === item.id} importFlow={importFlow} />
          ))}
        </Fragment>
      ))}
    </nav>
  );
}
