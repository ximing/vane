import type { ComponentType } from 'react';
import Home from './pages/Home';
import QuickStart from './pages/QuickStart';
import HybridSearch from './pages/guides/HybridSearch';
import Tokenizers from './pages/guides/Tokenizers';
import Reindex from './pages/guides/Reindex';
import Persistence from './pages/guides/Persistence';
import WebIntegration from './pages/guides/WebIntegration';
import ApiOverview from './pages/api/Overview';
import ApiOpen from './pages/api/Open';
import ApiCollection from './pages/api/Collection';
import ApiDocuments from './pages/api/Documents';
import ApiSearch from './pages/api/Search';
import ApiMaintenance from './pages/api/Maintenance';
import ApiErrors from './pages/api/Errors';
import Examples from './pages/Examples';

export interface RouteDef {
  path: string;
  name: string;
  Component: ComponentType;
}

/**
 * Central route table. Reused by the sitemap generator (T18) —
 * keep every public page listed here.
 */
export const routes: RouteDef[] = [
  { path: '/', name: 'Home', Component: Home },
  { path: '/quickstart', name: 'Quick Start', Component: QuickStart },
  { path: '/guides/hybrid-search', name: 'Hybrid Search', Component: HybridSearch },
  { path: '/guides/tokenizers', name: 'Tokenizers', Component: Tokenizers },
  { path: '/guides/reindex', name: 'Custom Dict & Reindex', Component: Reindex },
  { path: '/guides/persistence', name: 'Persistence & Visibility', Component: Persistence },
  { path: '/guides/web-integration', name: 'Web Integration (vite/webpack)', Component: WebIntegration },
  { path: '/api/overview', name: 'API Overview', Component: ApiOverview },
  { path: '/api/open', name: 'open', Component: ApiOpen },
  { path: '/api/collection', name: 'collection', Component: ApiCollection },
  { path: '/api/documents', name: 'documents', Component: ApiDocuments },
  { path: '/api/search', name: 'search', Component: ApiSearch },
  { path: '/api/maintenance', name: 'maintenance', Component: ApiMaintenance },
  { path: '/api/errors', name: 'Error Codes', Component: ApiErrors },
  { path: '/examples', name: 'Examples', Component: Examples },
];
