import './Footer.css';

const REPO_URL = 'https://github.com/ximing/vane';

const FOOTER_LINKS: Array<{ label: string; href: string }> = [
  { label: 'GitHub', href: REPO_URL },
  { label: 'SPEC', href: `${REPO_URL}/blob/main/docs/SPEC.md` },
  { label: 'REQUIREMENTS', href: `${REPO_URL}/blob/main/docs/REQUIREMENTS.md` },
  { label: 'Apache-2.0', href: `${REPO_URL}/blob/main/LICENSE` },
];

export default function Footer() {
  return (
    <footer className="footer">
      <div className="footer__inner">
        <span className="footer__brand">vane</span>
        <nav className="footer__links" aria-label="Footer">
          {FOOTER_LINKS.map(({ label, href }) => (
            <a
              key={label}
              className="footer__link"
              href={href}
              target="_blank"
              rel="noopener noreferrer"
            >
              {label}
            </a>
          ))}
        </nav>
        <span className="footer__note">Released under the Apache-2.0 license.</span>
      </div>
    </footer>
  );
}
