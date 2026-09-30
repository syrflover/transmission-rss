import { Button } from "@/components/ui/button";
import { ThemeDarkIcon, ThemeLightIcon, ThemeSystemIcon } from "@/components/icons";
import {
  cycleTheme,
  nextTheme,
  THEME_LABEL,
  THEME_LABEL_WITH_RO,
  useThemePreference,
  type ThemePreference,
} from "@/lib/theme";

const ICON = {
  system: ThemeSystemIcon,
  light: ThemeLightIcon,
  dark: ThemeDarkIcon,
} satisfies Record<ThemePreference, unknown>;

/**
 * One button that cycles 시스템 → 라이트 → 다크. It shows the current mode and
 * names the next one for assistive technology; on phones only the icon shows.
 */
export function ThemeToggle() {
  const preference = useThemePreference();
  const Icon = ICON[preference];
  const label = THEME_LABEL[preference];

  return (
    <Button
      type="button"
      variant="outline"
      onClick={cycleTheme}
      aria-label={`화면 모드 ${label}, 누르면 ${THEME_LABEL_WITH_RO[nextTheme(preference)]} 변경`}
      title={`화면 모드 ${label}`}
      className="h-[34px] gap-[7px] rounded-full border-hairline bg-surface-1 px-3 text-xs font-semibold text-text-secondary shadow-none hover:border-text-muted hover:bg-surface-1 hover:text-text-primary max-[720px]:w-[34px] max-[720px]:px-0 dark:border-hairline dark:bg-surface-1 dark:hover:border-text-muted dark:hover:bg-surface-1"
    >
      <Icon className="size-[15px]" />
      <span className="min-w-[3em] text-left max-[720px]:sr-only">{label}</span>
    </Button>
  );
}
