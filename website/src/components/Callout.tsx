import type { CalloutProps } from './contracts';
import './Callout.css';

const DEFAULT_TITLES: Record<CalloutProps['type'], string> = {
  note: 'Note',
  warning: 'Warning',
  gap: 'Known gap',
};

export default function Callout({ type, title, children }: CalloutProps) {
  return (
    <aside className={`callout callout--${type}`}>
      <p className="callout__title">{title ?? DEFAULT_TITLES[type]}</p>
      <div className="callout__body">{children}</div>
    </aside>
  );
}
