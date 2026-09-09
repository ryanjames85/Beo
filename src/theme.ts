export type Theme = "light" | "dark" | "system";

const STORAGE_KEY = "beo-theme";

export function getStoredTheme(): Theme {
  const stored = localStorage.getItem(STORAGE_KEY);
  return stored === "light" || stored === "dark" ? stored : "system";
}

// "system" is applied by simply *not* setting data-theme — the CSS's
// prefers-color-scheme media query then follows the OS live, with no JS
// listener needed to react to the user changing their OS theme later.
export function applyTheme(theme: Theme) {
  if (theme === "system") {
    delete document.documentElement.dataset.theme;
  } else {
    document.documentElement.dataset.theme = theme;
  }
  localStorage.setItem(STORAGE_KEY, theme);
}
