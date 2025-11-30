import type * as React from 'react';

export type Lang = 'node' | 'go' | 'browser';

export interface CodeBlockProps {
  code: string;
  lang: 'rust' | 'js' | 'ts' | 'go' | 'bash' | 'json';
  title?: string;             // 窗口框标题（文件名）
}

export interface LangTabsProps {
  node: React.ReactNode;
  go: React.ReactNode;
  browser: React.ReactNode;   // 三个 pane 内容；当前语言由全局偏好决定
}

export interface DocsLayoutProps {
  children: React.ReactNode;  // 页面内容；h2 必须手写 id，TOC 自动扫描
}

export interface CalloutProps {
  type: 'note' | 'warning' | 'gap'; // gap = known-gap 如实标注（filter 未透出等）
  title?: string;
  children: React.ReactNode;
}

// SearchDemo 数据契约（T7 产物必须严格符合）
export interface DemoHit { id: string; title: string; snippet: string; score: number; }
export interface DemoQuery { q: string; hybrid: DemoHit[]; vector: DemoHit[]; text: DemoHit[]; }
export interface DemoData {
  docs: Array<{ id: string; title: string; body: string }>; // ~30 条中英混合
  queries: DemoQuery[];                                       // ≥6 个预置 query
  provenance: 'vane-node' | 'manual'; // 数据来源：真实库生成 / 手写降级；T9 据此渲染标注文案
}
