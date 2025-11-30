#!/usr/bin/env node
/**
 * Generates dist/sitemap.xml from the central route table in src/routes.ts.
 * Runs as the last step of `npm run build` (after the 404.html copy).
 * Host is pinned to the GitHub Pages deployment URL.
 */
import { readFileSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const ROOT = join(dirname(fileURLToPath(import.meta.url)), '..');
const HOST = 'https://ximing.github.io/vane';

const source = readFileSync(join(ROOT, 'src/routes.ts'), 'utf8');

// Extract every `path: '...'` entry from the exported routes array.
const paths = [...source.matchAll(/path:\s*'([^']+)'/g)].map((m) => m[1]);
if (paths.length === 0) {
  throw new Error('gen-sitemap: no routes found in src/routes.ts');
}

const today = new Date().toISOString().slice(0, 10);
const urls = paths
  .map((p) => {
    const loc = p === '/' ? `${HOST}/` : `${HOST}${p}`;
    return [
      '  <url>',
      `    <loc>${loc}</loc>`,
      `    <lastmod>${today}</lastmod>`,
      '  </url>',
    ].join('\n');
  })
  .join('\n');

const xml = `<?xml version="1.0" encoding="UTF-8"?>
<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
${urls}
</urlset>
`;

writeFileSync(join(ROOT, 'dist/sitemap.xml'), xml);
console.log(`gen-sitemap: wrote ${paths.length} routes to dist/sitemap.xml`);
