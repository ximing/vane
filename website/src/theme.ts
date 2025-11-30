/*
 * Theme controller for the Vane docs site.
 *
 * - The initial theme is applied before first paint by the synchronous inline
 *   script in index.html (anti-FOUC); initTheme() re-applies it idempotently
 *   and starts following system theme changes while no manual preference is
 *   stored.
 * - A manual choice (toggleTheme/setTheme) is persisted in localStorage under
 *   'vane-theme' and takes precedence over prefers-color-scheme.
 */

export type Theme = 'light' | 'dark';

const STORAGE_KEY = 'vane-theme';

const media: MediaQueryList = window.matchMedia('(prefers-color-scheme: dark)');

function readStoredTheme(): Theme | null {
  try {
    const value = localStorage.getItem(STORAGE_KEY);
    return value === 'light' || value === 'dark' ? value : null;
  } catch {
    return null;
  }
}

function systemTheme(): Theme {
  return media.matches ? 'dark' : 'light';
}

function applyTheme(theme: Theme): void {
  document.documentElement.setAttribute('data-theme', theme);
}

/** Current effective theme (stored preference wins, else system). */
export function getTheme(): Theme {
  return readStoredTheme() ?? systemTheme();
}

/**
 * Apply the effective theme to <html data-theme> and follow system theme
 * changes until the user makes a manual choice. Safe to call more than once.
 */
export function initTheme(): void {
  applyTheme(getTheme());
  media.addEventListener('change', () => {
    if (readStoredTheme() === null) {
      applyTheme(systemTheme());
    }
  });
}

/** Persist and apply an explicit theme choice. */
export function setTheme(theme: Theme): void {
  try {
    localStorage.setItem(STORAGE_KEY, theme);
  } catch {
    // localStorage unavailable (private mode etc.) — apply for this session only.
  }
  applyTheme(theme);
}

/** Flip the current theme, persist the choice, and return the new theme. */
export function toggleTheme(): Theme {
  const next: Theme = getTheme() === 'dark' ? 'light' : 'dark';
  setTheme(next);
  return next;
}
