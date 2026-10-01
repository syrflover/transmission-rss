import { useEffect, type ReactNode } from "react";

import { APP_NAME } from "@/app/menu";

interface ScreenFrameProps {
  title: string;
  /** The screen's content. */
  children: ReactNode;
  /** Next to the title, such as a button that moves within the screen. */
  actions?: ReactNode;
  /** Under the title: facts about what the screen shows. */
  meta?: ReactNode;
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
export function ScreenFrame({ title, children, actions, meta }: ScreenFrameProps) {
  usePageTitle(title);

  return (
    <section className="pb-4">
      <div
        className={
          meta
            ? "flex flex-wrap items-center gap-x-4 gap-y-1 pt-7 pb-1.5 max-[720px]:pt-[18px]"
            : "flex flex-wrap items-baseline gap-x-4 gap-y-1 pt-7 pb-3.5 max-[720px]:pt-[18px] max-[720px]:pb-3"
        }
      >
        <h1 className="text-2xl font-bold tracking-[-0.005em]">{title}</h1>
        {actions}
      </div>
      {meta && <div className="pb-3.5 max-[720px]:pb-3">{meta}</div>}
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
