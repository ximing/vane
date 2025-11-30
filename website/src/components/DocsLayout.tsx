import { useEffect, useRef, useState } from 'react';
import { NavLink, useLocation } from 'react-router-dom';
import TopBar from './TopBar';
import { docsNav } from '../nav';
import type { DocsLayoutProps } from './contracts';
import './DocsLayout.css';

interface TocEntry {
  id: string;
  text: string;
  level: 2 | 3;
}

function tocEqual(a: TocEntry[], b: TocEntry[]): boolean {
  return (
    a.length === b.length &&
    a.every((entry, i) => {
      const other = b[i];
      return (
        other !== undefined &&
        entry.id === other.id &&
        entry.text === other.text &&
        entry.level === other.level
      );
    })
  );
}

export default function DocsLayout({ children }: DocsLayoutProps) {
  const contentRef = useRef<HTMLDivElement>(null);
  const [toc, setToc] = useState<TocEntry[]>([]);
  const [activeId, setActiveId] = useState<string | null>(null);
  const [navOpen, setNavOpen] = useState(false);
  const { pathname } = useLocation();

  // Close the mobile drawer whenever the route changes.
  useEffect(() => {
    setNavOpen(false);
  }, [pathname]);

  // TOC: scan the rendered content for h2[id]/h3[id], rescan on DOM mutations
  // (e.g. async content). Page authors only write heading ids — no manual TOC.
  useEffect(() => {
    const container = contentRef.current;
    if (!container) return;

    const scan = () => {
      const headings = container.querySelectorAll('h2[id], h3[id]');
      const entries: TocEntry[] = Array.from(headings, (h) => ({
        id: h.id,
        text: h.textContent ?? '',
        level: h.tagName === 'H3' ? 3 : 2,
      }));
      setToc((prev) => (tocEqual(prev, entries) ? prev : entries));
    };

    scan();
    const observer = new MutationObserver(scan);
    observer.observe(container, { childList: true, subtree: true, characterData: true });
    return () => observer.disconnect();
  }, []);

  // Highlight the current section while scrolling.
  useEffect(() => {
    const container = contentRef.current;
    if (!container || toc.length === 0) return;

    const headings = toc
      .map((entry) => container.querySelector<HTMLElement>(`#${CSS.escape(entry.id)}`))
      .filter((h): h is HTMLElement => h !== null);
    if (headings.length === 0) return;

    const observer = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) {
          if (entry.isIntersecting) {
            setActiveId(entry.target.id);
          }
        }
      },
      // Trigger zone: just below the topbar, upper quarter of the viewport.
      { rootMargin: '-64px 0px -75% 0px', threshold: 0 },
    );
    headings.forEach((h) => observer.observe(h));

    // A short final section can never reach the trigger zone; when the page
    // is scrolled to the bottom, fall back to highlighting the last heading.
    const onScroll = () => {
      if (window.scrollY + window.innerHeight >= document.documentElement.scrollHeight - 2) {
        const last = headings[headings.length - 1];
        if (last) setActiveId(last.id);
      }
    };
    window.addEventListener('scroll', onScroll, { passive: true });

    return () => {
      observer.disconnect();
      window.removeEventListener('scroll', onScroll);
    };
  }, [toc]);

  return (
    <div className="docs-layout">
      <TopBar onMenuToggle={() => setNavOpen((open) => !open)} menuOpen={navOpen} />
      <div className="docs-body">
        <aside className={navOpen ? 'docs-sidebar docs-sidebar--open' : 'docs-sidebar'}>
          <nav className="docs-nav" aria-label="Documentation">
            {docsNav.map((section) => (
              <div className="docs-nav__section" key={section.title}>
                <p className="docs-nav__title">{section.title}</p>
                <ul className="docs-nav__list">
                  {section.items.map((item) => (
                    <li key={item.path}>
                      <NavLink
                        to={item.path}
                        className={({ isActive }) =>
                          isActive ? 'docs-nav__link docs-nav__link--active' : 'docs-nav__link'
                        }
                      >
                        {item.label}
                      </NavLink>
                    </li>
                  ))}
                </ul>
              </div>
            ))}
          </nav>
        </aside>
        {navOpen && (
          <div
            className="docs-backdrop"
            onClick={() => setNavOpen(false)}
            aria-hidden="true"
          />
        )}
        <div className="docs-content" ref={contentRef}>
          {children}
        </div>
        <aside className="docs-toc">
          {toc.length > 0 && (
            <nav aria-label="On this page">
              <p className="docs-toc__title">On this page</p>
              <ul className="docs-toc__list">
                {toc.map((entry) => (
                  <li
                    key={entry.id}
                    className={entry.level === 3 ? 'docs-toc__item docs-toc__item--nested' : 'docs-toc__item'}
                  >
                    <a
                      href={`#${entry.id}`}
                      className={
                        activeId === entry.id ? 'docs-toc__link docs-toc__link--active' : 'docs-toc__link'
                      }
                    >
                      {entry.text}
                    </a>
                  </li>
                ))}
              </ul>
            </nav>
          )}
        </aside>
      </div>
    </div>
  );
}
