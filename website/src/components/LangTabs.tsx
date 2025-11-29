import { useEffect, useState } from 'react';
import type { ReactNode } from 'react';
import type { Lang, LangTabsProps } from './contracts';
import './LangTabs.css';

const STORAGE_KEY = 'vane-lang';
const EVENT_NAME = 'vane-lang-change';

const LANGS: Lang[] = ['node', 'go', 'browser'];

const LABELS: Record<Lang, string> = {
  node: 'Node.js',
  go: 'Go',
  browser: 'Browser',
};

function isLang(value: unknown): value is Lang {
  return value === 'node' || value === 'go' || value === 'browser';
}

function readStoredLang(): Lang {
  try {
    const stored = window.sessionStorage.getItem(STORAGE_KEY);
    if (isLang(stored)) return stored;
  } catch {
    // sessionStorage unavailable (private mode etc.) — fall through to default
  }
  return 'node';
}

function publishLang(next: Lang) {
  try {
    window.sessionStorage.setItem(STORAGE_KEY, next);
  } catch {
    // sessionStorage unavailable — still broadcast the in-page switch
  }
  window.dispatchEvent(new CustomEvent<Lang>(EVENT_NAME, { detail: next }));
}

export default function LangTabs({ node, go, browser }: LangTabsProps) {
  const [lang, setLang] = useState<Lang>(readStoredLang);

  useEffect(() => {
    const onChange = (event: Event) => {
      const next = (event as CustomEvent<Lang>).detail;
      if (isLang(next)) setLang(next);
    };
    window.addEventListener(EVENT_NAME, onChange);
    return () => window.removeEventListener(EVENT_NAME, onChange);
  }, []);

  const panes: Record<Lang, ReactNode> = { node, go, browser };

  return (
    <div className="lang-tabs">
      <div className="lang-tabs__bar" role="tablist" aria-label="Runtime language">
        {LANGS.map((l) => (
          <button
            key={l}
            type="button"
            role="tab"
            aria-selected={l === lang}
            className={
              l === lang ? 'lang-tabs__tab lang-tabs__tab--active' : 'lang-tabs__tab'
            }
            onClick={() => publishLang(l)}
          >
            {LABELS[l]}
          </button>
        ))}
      </div>
      <div className="lang-tabs__pane" role="tabpanel">
        {panes[lang]}
      </div>
    </div>
  );
}
