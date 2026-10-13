import { useId } from "react";

import { chooseTheme, THEME_LABEL, THEME_ORDER, useThemePreference, type ThemePreference } from "@/lib/theme";

const radio = "mt-0.5 size-[18px] flex-none accent-focus";
const option =
  "flex min-w-0 cursor-pointer items-start gap-3 rounded-[10px] border border-hairline-soft bg-surface-2 px-3.5 py-3 has-[:checked]:border-focus has-[:focus-visible]:outline-2 has-[:focus-visible]:outline-offset-2 has-[:focus-visible]:outline-focus";

const NOTE: Record<ThemePreference, string> = {
  system: "기기의 라이트·다크 설정을 따라가요.",
  light: "늘 밝은 화면이에요.",
  dark: "늘 어두운 화면이에요.",
};

/** `화면 모드`: 시스템·라이트·다크, applied at once and kept in this browser only. */
export function ThemePanel() {
  const preference = useThemePreference();
  const uid = useId();

  return (
    <div className="flex flex-col gap-5 p-5 max-[720px]:p-4">
      <header className="flex flex-col gap-1.5">
        <h2 id="panel-title" tabIndex={-1} className="m-0 text-xl font-bold outline-none">
          화면 모드
        </h2>
        <p className="max-w-[62ch] text-[13.5px] leading-relaxed text-text-secondary">
          고른 모드는 이 기기의 브라우저에만 저장되고 다른 기기에는 적용되지 않아요.
        </p>
      </header>

      <fieldset className="m-0 flex min-w-0 flex-col gap-3 border-0 p-0">
        <legend className="sr-only">화면 모드</legend>
        {THEME_ORDER.map((mode) => (
          <label key={mode} className={option}>
            <input
              type="radio"
              name={`${uid}-theme`}
              className={radio}
              checked={preference === mode}
              onChange={() => chooseTheme(mode)}
            />
            <span className="flex min-w-0 flex-col gap-0.5">
              <span className="text-[14.5px] font-semibold">{THEME_LABEL[mode]}</span>
              <span className="text-[13px] leading-normal text-text-secondary">{NOTE[mode]}</span>
            </span>
          </label>
        ))}
      </fieldset>
    </div>
  );
}
