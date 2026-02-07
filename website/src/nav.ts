/**
 * Sidebar navigation tree for DocsLayout (plan §2.2).
 * Static config — pages register themselves in routes.ts; this file only
 * describes how they are grouped/labelled in the docs sidebar.
 */

export interface NavItem {
  label: string;
  path: string;
}

export interface NavSection {
  title: string;
  items: NavItem[];
}

export const docsNav: NavSection[] = [
  {
    title: 'Getting Started',
    items: [{ label: 'Quick Start', path: '/quickstart' }],
  },
  {
    title: 'Guides',
    items: [
      { label: 'Hybrid Search', path: '/guides/hybrid-search' },
      { label: 'Tokenizers', path: '/guides/tokenizers' },
      { label: 'Custom Dict & Reindex', path: '/guides/reindex' },
      { label: 'Persistence & Visibility', path: '/guides/persistence' },
      { label: 'Web Integration', path: '/guides/web-integration' },
    ],
  },
  {
    title: 'API Reference',
    items: [
      { label: 'Overview', path: '/api/overview' },
      { label: 'open', path: '/api/open' },
      { label: 'collection', path: '/api/collection' },
      { label: 'documents', path: '/api/documents' },
      { label: 'search', path: '/api/search' },
      { label: 'maintenance', path: '/api/maintenance' },
      { label: 'errors', path: '/api/errors' },
    ],
  },
  {
    title: 'Examples',
    items: [{ label: 'Examples', path: '/examples' }],
  },
];
