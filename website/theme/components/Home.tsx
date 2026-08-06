import './home.css';

/**
 * The home page renders only its frontmatter, so the install command, the core mechanic
 * and the entry links live here and mount through the home-only Layout slots.
 */

const K = ({ children }: { children: React.ReactNode }) => (
  <span className="rune-tok--keyword">{children}</span>
);
const S = ({ children }: { children: React.ReactNode }) => (
  <span className="rune-tok--string">{children}</span>
);
const C = ({ children }: { children: React.ReactNode }) => (
  <span className="rune-tok--comment">{children}</span>
);

export function HomeIntro() {
  return (
    <section className="rune-home">
      <div className="rune-home__inner">
        <div className="rune-panel rune-panel--wide">
          <div className="rune-panel__title">install</div>
          <pre className="rune-panel__body">
            <code>pnpm add -D @giancarlosio/rune</code>
          </pre>
        </div>

        <h2 className="rune-home__heading">One definition, every package</h2>
        <p className="rune-home__lede">
          The command lives in one file at the repository root. Each package references it by
          name, and that reference never changes again.
        </p>

        <div className="rune-home__pair">
          <div className="rune-panel">
            <div className="rune-panel__title">rune.config.ts</div>
            <pre className="rune-panel__body">
              <code>
                {`import { defineConfig } from '@giancarlosio/rune'\n\n`}
                <K>export default</K>
                {` defineConfig({\n  scripts: {\n    test: {\n      command: `}
                <S>{`'vitest run --coverage --reporter=dot'`}</S>
                {`,\n      description: `}
                <S>{`'Run unit tests'`}</S>
                {`,\n    },\n  },\n})`}
              </code>
            </pre>
          </div>
          <div className="rune-panel">
            <div className="rune-panel__title">packages/*/package.json</div>
            <pre className="rune-panel__body">
              <code>
                {`{\n  "scripts": {\n    "test": `}
                <S>"rune run test"</S>
                {`\n  }\n}\n\n`}
                <C>{`// changing a flag is one edit at the root,`}</C>
                {`\n`}
                <C>{`// not one edit per package`}</C>
              </code>
            </pre>
          </div>
        </div>
      </div>
    </section>
  );
}

const ENTRY_POINTS = [
  {
    title: 'Installation',
    body: 'Add Rune to a repository and write the first config.',
    href: '/guide/installation',
  },
  {
    title: 'Script types',
    body: 'Every field a script entry accepts, and which ones combine.',
    href: '/config/scripts',
  },
  {
    title: 'Commands',
    body: 'run, list, inspect, init and cache, with their flags.',
    href: '/cli/',
  },
];

export function HomeEntryPoints() {
  return (
    <section className="rune-home">
      <div className="rune-home__inner">
        <h2 className="rune-home__heading">Start here</h2>
        <div className="rune-home__cards">
          {ENTRY_POINTS.map((entry) => (
            <a className="rune-card" href={entry.href} key={entry.href}>
              <span className="rune-card__title">{entry.title}</span>
              <span className="rune-card__body">{entry.body}</span>
            </a>
          ))}
        </div>
      </div>
    </section>
  );
}
