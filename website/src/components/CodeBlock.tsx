import { useEffect, useRef, useState } from 'react';
import type { CodeBlockProps } from './contracts';
import './CodeBlock.css';

/**
 * macOS-style window frame around a syntax-highlighted code block.
 *
 * The shiki module is pulled in with a dynamic import() (lazy chunk) on
 * first render; until it resolves — or if it fails — an unhighlighted
 * monospace <pre> with identical metrics is shown, so the container never
 * jumps. Highlighted output uses CSS variables for both themes, so a
 * data-theme switch re-colors without re-running shiki.
 */
export default function CodeBlock({ code, lang, title }: CodeBlockProps) {
  const [html, setHtml] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const copyTimer = useRef<number | undefined>(undefined);

  useEffect(() => {
    let cancelled = false;
    import('../lib/highlighter')
      .then((mod) => mod.highlightCode(code, lang))
      .then((out) => {
        if (!cancelled) setHtml(out);
      })
      .catch(() => {
        // Highlighter unavailable (chunk failed, unsupported grammar…) —
        // keep the plain-text fallback instead of breaking the page.
      });
    return () => {
      cancelled = true;
    };
  }, [code, lang]);

  useEffect(() => {
    return () => {
      window.clearTimeout(copyTimer.current);
    };
  }, []);

  async function copy() {
    try {
      await navigator.clipboard.writeText(code);
    } catch {
      return;
    }
    setCopied(true);
    window.clearTimeout(copyTimer.current);
    copyTimer.current = window.setTimeout(() => setCopied(false), 1600);
  }

  return (
    <figure className="code-block">
      <figcaption className="code-block__bar">
        <span className="code-block__dots" aria-hidden="true">
          <i className="code-block__dot code-block__dot--close" />
          <i className="code-block__dot code-block__dot--min" />
          <i className="code-block__dot code-block__dot--max" />
        </span>
        {title !== undefined && (
          <span className="code-block__title">{title}</span>
        )}
        <span className="code-block__lang">{lang}</span>
        <button
          type="button"
          className="code-block__copy"
          onClick={copy}
          aria-live="polite"
        >
          {copied ? 'Copied' : 'Copy'}
        </button>
      </figcaption>
      <div className="code-block__body">
        {html !== null ? (
          <div
            className="code-block__highlight"
            // Trusted input: shiki emits fully escaped HTML from our own code strings.
            dangerouslySetInnerHTML={{ __html: html }}
          />
        ) : (
          <pre className="code-block__plain">
            <code>{code}</code>
          </pre>
        )}
      </div>
    </figure>
  );
}
