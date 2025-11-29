import { useEffect, useState } from 'react';
import { Link, NavLink } from 'react-router-dom';
import { getTheme, initTheme, toggleTheme, type Theme } from '../theme';
import './TopBar.css';

const GITHUB_URL = 'https://github.com/ximing/vane';

interface TopBarLink {
  label: string;
  to: string;
}

const TOPBAR_LINKS: TopBarLink[] = [
  { label: 'Docs', to: '/quickstart' },
  { label: 'Guides', to: '/guides/hybrid-search' },
  { label: 'API', to: '/api/overview' },
  { label: 'Examples', to: '/examples' },
];

export interface TopBarProps {
  /** When provided, a hamburger button is shown on mobile (≤900px). */
  onMenuToggle?: () => void;
  menuOpen?: boolean;
}

// initTheme() installs a prefers-color-scheme listener — guard against
// repeated mounts (every page renders a TopBar).
let themeInitialized = false;

function initThemeOnce(): void {
  if (!themeInitialized) {
    themeInitialized = true;
    initTheme();
  }
}

function GitHubIcon() {
  return (
    <svg viewBox="0 0 24 24" width="20" height="20" fill="currentColor" aria-hidden="true">
      <path d="M12 .297c-6.63 0-12 5.373-12 12 0 5.303 3.438 9.8 8.205 11.385.6.113.82-.258.82-.577 0-.285-.01-1.04-.015-2.04-3.338.724-4.042-1.61-4.042-1.61C4.422 18.07 3.633 17.7 3.633 17.7c-1.087-.744.084-.729.084-.729 1.205.084 1.838 1.236 1.838 1.236 1.07 1.835 2.809 1.305 3.495.998.108-.776.417-1.305.76-1.605-2.665-.3-5.466-1.332-5.466-5.93 0-1.31.465-2.38 1.235-3.22-.135-.303-.54-1.523.105-3.176 0 0 1.005-.322 3.3 1.23.96-.267 1.98-.399 3-.405 1.02.006 2.04.138 3 .405 2.28-1.552 3.285-1.23 3.285-1.23.645 1.653.24 2.873.12 3.176.765.84 1.23 1.91 1.23 3.22 0 4.61-2.805 5.625-5.475 5.92.42.36.81 1.096.81 2.22 0 1.606-.015 2.896-.015 3.286 0 .315.21.69.825.57C20.565 22.092 24 17.592 24 12.297c0-6.627-5.373-12-12-12" />
    </svg>
  );
}

function SunIcon() {
  return (
    <svg
      viewBox="0 0 24 24"
      width="18"
      height="18"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      aria-hidden="true"
    >
      <circle cx="12" cy="12" r="4" />
      <path d="M12 2v2M12 20v2M4.93 4.93l1.41 1.41M17.66 17.66l1.41 1.41M2 12h2M20 12h2M4.93 19.07l1.41-1.41M17.66 6.34l1.41-1.41" />
    </svg>
  );
}

function MoonIcon() {
  return (
    <svg
      viewBox="0 0 24 24"
      width="18"
      height="18"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z" />
    </svg>
  );
}

function MenuIcon({ open }: { open: boolean }) {
  return (
    <svg
      viewBox="0 0 24 24"
      width="20"
      height="20"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      aria-hidden="true"
    >
      {open ? <path d="M6 6l12 12M18 6L6 18" /> : <path d="M4 7h16M4 12h16M4 17h16" />}
    </svg>
  );
}

export default function TopBar({ onMenuToggle, menuOpen = false }: TopBarProps) {
  const [theme, setTheme] = useState<Theme>(() => getTheme());

  useEffect(() => {
    initThemeOnce();
    setTheme(getTheme());
  }, []);

  return (
    <header className="topbar">
      {onMenuToggle !== undefined && (
        <button
          type="button"
          className="topbar__menu"
          onClick={onMenuToggle}
          aria-label={menuOpen ? 'Close navigation' : 'Open navigation'}
          aria-expanded={menuOpen}
        >
          <MenuIcon open={menuOpen} />
        </button>
      )}
      <Link to="/" className="topbar__logo">
        vane
      </Link>
      <nav className="topbar__nav" aria-label="Primary">
        {TOPBAR_LINKS.map(({ label, to }) => (
          <NavLink
            key={to}
            to={to}
            className={({ isActive }) =>
              isActive ? 'topbar__link topbar__link--active' : 'topbar__link'
            }
          >
            {label}
          </NavLink>
        ))}
      </nav>
      <div className="topbar__actions">
        <a
          className="topbar__icon-btn"
          href={GITHUB_URL}
          target="_blank"
          rel="noopener noreferrer"
          aria-label="GitHub repository"
        >
          <GitHubIcon />
        </a>
        <button
          type="button"
          className="topbar__icon-btn"
          onClick={() => setTheme(toggleTheme())}
          aria-label={theme === 'dark' ? 'Switch to light theme' : 'Switch to dark theme'}
        >
          {theme === 'dark' ? <SunIcon /> : <MoonIcon />}
        </button>
      </div>
    </header>
  );
}
