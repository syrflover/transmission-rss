import { useSyncExternalStore } from "react";
import { Link, Navigate, useParams } from "react-router-dom";

import { cn } from "@/lib/utils";

import { ScreenFrame } from "./ScreenFrame";
import { BackIcon } from "./settings/icons";
import { CollectionPanel } from "./settings/collection/CollectionPanel";
import { ImportPanel } from "./settings/import/ImportPanel";
import { useImportFlow } from "./settings/import/useImportFlow";
import { findItem, SETTINGS_ITEMS, type SettingsItem } from "./settings/items";
import { BTN, CARD } from "./settings/parts";
import { SettingsList } from "./settings/SettingsList";

const NARROW = "(max-width: 720px)";

/** True on phone widths, where the list and an item are separate screens. */
function useNarrow(): boolean {
  return useSyncExternalStore(
    (notify) => {
      const query = window.matchMedia(NARROW);
      query.addEventListener("change", notify);
      return () => query.removeEventListener("change", notify);
    },
    () => window.matchMedia(NARROW).matches,
    () => false,
  );
}

function EmptyItem({ item }: { item: SettingsItem }) {
  return (
    <div className="flex flex-col gap-4 p-5 max-[720px]:p-4">
      <h2 id="panel-title" tabIndex={-1} className="m-0 text-xl font-bold outline-none">
        {item.title}
      </h2>
      <p className="rounded-card border border-dashed border-hairline px-4 py-6 text-center text-[13.5px] leading-relaxed text-text-muted">
        {item.empty}
      </p>
    </div>
  );
}

/**
 * Settings as a list and detail. On a PC the list is on the left and the open
 * item on the right; on a phone an item is its own screen and its back button
 * returns to the list. Only the collect folder and the import are built so
 * far; the other items show why they are empty.
 */
export function SettingsScreen() {
  const { "*": rest } = useParams();
  const id = rest?.split("/")[0] || undefined;
  const narrow = useNarrow();
  const importFlow = useImportFlow();

  if (id && !findItem(id)) return <Navigate to="/settings" replace />;

  const open = id ? findItem(id) : undefined;
  // On a PC the first item is open until another one is chosen.
  const shown = open ?? (narrow ? undefined : SETTINGS_ITEMS[0]);
  const listOnly = narrow && !open;

  return (
    <ScreenFrame title="설정">
      <div className="grid grid-cols-[minmax(0,380px)_minmax(0,1fr)] items-start gap-6 max-[720px]:grid-cols-1">
        <SettingsList
          current={shown?.id ?? null}
          importFlow={importFlow}
          className={cn(narrow && open && "hidden")}
        />
        {shown && !listOnly && (
          <section aria-labelledby="panel-title" className="flex min-w-0 flex-col gap-3">
            {narrow && (
              <div>
                <Link to="/settings" className={BTN.plain}>
                  <BackIcon className="size-4" />
                  설정
                </Link>
              </div>
            )}
            <div className={CARD}>
              {shown.id === "import" ? (
                <ImportPanel flow={importFlow} />
              ) : shown.id === "collection" ? (
                <CollectionPanel />
              ) : (
                <EmptyItem item={shown} />
              )}
            </div>
          </section>
        )}
      </div>
    </ScreenFrame>
  );
}
