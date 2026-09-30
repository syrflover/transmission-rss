import { useEffect, type ReactNode } from "react";

import { APP_NAME } from "@/app/menu";

interface ScreenFrameProps {
  title: string;
  /** The screen's content. */
  children: ReactNode;
}

/** Sets the browser tab title to "<screen> - TRSS" while the screen is mounted. */
export function usePageTitle(title: string) {
  useEffect(() => {
    document.title = `${title} - ${APP_NAME}`;
    return () => {
      document.title = APP_NAME;
    };
  }, [title]);
}

/** Common page frame: the screen title, then whatever the screen shows. */
export function ScreenFrame({ title, children }: ScreenFrameProps) {
  usePageTitle(title);

  return (
    <section className="pb-4">
      <div className="flex flex-wrap items-baseline gap-x-4 gap-y-1 pt-7 pb-3.5 max-[720px]:pt-[18px] max-[720px]:pb-3">
        <h1 className="text-2xl font-bold tracking-[-0.005em]">{title}</h1>
      </div>
      {children}
    </section>
  );
}

/** Empty state: why nothing is shown and what makes content appear, in one or two sentences. */
export function EmptyState({ children }: { children: ReactNode }) {
  return (
    <p className="rounded-card border border-dashed border-hairline px-4 py-6 text-center text-[13.5px] leading-relaxed text-text-muted">
      {children}
    </p>
  );
}
