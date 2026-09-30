/**
 * Shared class strings of the channels tab. Buttons follow the common rules:
 * moving between screens is neutral, state-changing actions get an accent
 * border and text, and a filled button is only the screen's main action or the
 * final button of an in-place confirmation. Red is for irreversible actions.
 */
const button =
  "h-auto min-h-9 rounded-full px-[15px] text-[13px] font-semibold max-[720px]:min-h-10";

export const btnNeutral = `${button} border border-hairline bg-surface-1 text-text-primary hover:border-text-secondary hover:bg-surface-2 dark:hover:bg-surface-2`;

export const btnAction = `${button} border border-[color-mix(in_srgb,var(--focus-ring)_50%,transparent)] bg-surface-1 text-focus hover:border-focus hover:bg-[color-mix(in_srgb,var(--focus-ring)_12%,transparent)] dark:hover:bg-[color-mix(in_srgb,var(--focus-ring)_12%,transparent)]`;

export const btnPrimary = `${button} border border-focus bg-focus text-primary-foreground hover:bg-focus hover:brightness-110 dark:hover:bg-focus`;

export const btnDanger = `${button} border border-[color-mix(in_srgb,var(--accent-urgent)_50%,transparent)] bg-surface-1 text-urgent hover:border-urgent hover:bg-[color-mix(in_srgb,var(--accent-urgent)_10%,transparent)] dark:hover:bg-[color-mix(in_srgb,var(--accent-urgent)_10%,transparent)]`;

export const btnDangerSolid = `${button} border border-urgent bg-urgent text-urgent-ink hover:bg-urgent hover:brightness-110 dark:hover:bg-urgent`;

/** Text inputs: surface fill, hairline border, 16px on phones so iOS does not zoom. */
export const inputClass =
  "min-h-10 rounded-[10px] border-hairline bg-surface-2 px-3 py-2 text-sm text-text-primary shadow-none hover:border-text-muted focus-visible:border-focus focus-visible:ring-0 focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-focus dark:bg-surface-2 max-[720px]:min-h-11 max-[720px]:text-base";

export const labelClass = "text-[12.5px] font-semibold text-text-secondary";

export const hintClass = "text-xs leading-[1.45] text-text-muted";
