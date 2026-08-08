import { Layout as OriginalLayout } from '@rspress/core/theme-original';
import { HomeEntryPoints, HomeIntro } from './components/Home';
import './index.css';

export * from '@rspress/core/theme-original';

const AUTHOR_URL = 'https://github.com/giancarlosisasi';
const REPO_URL = 'https://github.com/giancarlosisasi/rune';

function Footer() {
  return (
    <footer className="rune-footer">
      <div className="rune-footer__inner">
        <nav className="rune-footer__links">
          <a href={REPO_URL}>GitHub</a>
          <a href={`${REPO_URL}/blob/main/LICENSE`}>License</a>
          <a href={`${REPO_URL}/issues`}>Issues</a>
        </nav>
        <span>
          Built by{' '}
          <a href={AUTHOR_URL} rel="me author">
            Giancarlos Isasi
          </a>
        </span>
      </div>
      <p className="rune-footer__note">
        Rune is an independent open-source project. It is not affiliated with npm, Inc., the
        OpenJS Foundation, or Vercel.
      </p>
    </footer>
  );
}

export function Layout() {
  return (
    <OriginalLayout
      afterHero={<HomeIntro />}
      afterFeatures={<HomeEntryPoints />}
      bottom={<Footer />}
    />
  );
}
