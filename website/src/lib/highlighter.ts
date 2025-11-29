/*
 * Shiki highlighter — loaded exclusively via dynamic import() from
 * CodeBlock.tsx so Vite splits it into its own lazy chunk and the entry
 * bundle never pays for grammars/themes.
 *
 * - shiki/core + the JavaScript RegExp engine (no oniguruma WASM).
 * - Fine-grained per-language/per-theme imports so only the 6 documented
 *   languages (rust/js/ts/go/bash/json) and the two themes are bundled.
 * - Dual themes in a single pass with `defaultColor: false`: tokens come
 *   out as `--shiki-light` / `--shiki-dark` CSS variables, so switching
 *   data-theme re-colors via CSS only and never re-runs highlighting
 *   (see CodeBlock.css).
 */
import { createHighlighterCore, type HighlighterCore } from 'shiki/core';
import { createJavaScriptRegexEngine } from 'shiki/engine/javascript';

import langRust from 'shiki/langs/rust.mjs';
import langJs from 'shiki/langs/javascript.mjs';
import langTs from 'shiki/langs/typescript.mjs';
import langGo from 'shiki/langs/go.mjs';
import langBash from 'shiki/langs/bash.mjs';
import langJson from 'shiki/langs/json.mjs';

import themeGithubLight from 'shiki/themes/github-light.mjs';
import themeGithubDark from 'shiki/themes/github-dark.mjs';

import type { CodeBlockProps } from '../components/contracts';

export type CodeBlockLang = CodeBlockProps['lang'];

let highlighterPromise: Promise<HighlighterCore> | null = null;

function getHighlighter(): Promise<HighlighterCore> {
  highlighterPromise ??= createHighlighterCore({
    themes: [themeGithubLight, themeGithubDark],
    langs: [langRust, langJs, langTs, langGo, langBash, langJson],
    // `forgiving: true` tolerates grammar constructs the JS engine cannot
    // compile by skipping those patterns instead of throwing.
    engine: createJavaScriptRegexEngine({ forgiving: true }),
  });
  return highlighterPromise;
}

/** Highlight `code` once, emitting CSS variables for both themes. */
export async function highlightCode(
  code: string,
  lang: CodeBlockLang,
): Promise<string> {
  const highlighter = await getHighlighter();
  return highlighter.codeToHtml(code, {
    lang,
    themes: { light: 'github-light', dark: 'github-dark' },
    defaultColor: false,
  });
}
