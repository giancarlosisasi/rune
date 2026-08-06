import path from 'node:path';
import { defineConfig, type UserConfig } from '@rspress/core';

const SITE_URL = 'https://rune.giancarlosisasi.dev';
const AUTHOR_URL = 'https://github.com/giancarlosio';
const REPO_URL = 'https://github.com/giancarlosio/rune';

const structuredData = {
  '@context': 'https://schema.org',
  '@type': 'SoftwareApplication',
  name: 'Rune',
  applicationCategory: 'DeveloperApplication',
  operatingSystem: 'Windows, macOS, Linux',
  url: SITE_URL,
  codeRepository: REPO_URL,
  description:
    'A script runner for JavaScript and TypeScript monorepos. Commands are defined once in rune.config.ts and referenced by name from every package.',
  author: {
    '@type': 'Person',
    name: 'Giancarlos Isasi',
    url: AUTHOR_URL,
    sameAs: [AUTHOR_URL],
  },
};

const head: UserConfig['head'] = [
  ['meta', { name: 'author', content: 'Giancarlos Isasi' }],
  ['link', { rel: 'author', href: AUTHOR_URL }],
];

const config: UserConfig = {
  root: 'docs',
  base: '/',
  lang: 'en',
  title: 'Rune',
  description:
    'A script runner for JavaScript and TypeScript monorepos. Define a command once, reference it by name from every package.',
  icon: '/favicon.svg',
  logo: { light: '/mark-light.svg', dark: '/mark-dark.svg' },
  logoText: 'rune',
  outDir: 'doc_build',
  head,
  markdown: {
    link: {
      checkDeadLinks: true,
      checkAnchors: true,
    },
    globalComponents: [
      path.resolve(__dirname, 'theme/components/Cards.tsx'),
      path.resolve(__dirname, 'theme/components/Card.tsx'),
    ],
  },
  route: {
    cleanUrls: true,
  },
  search: {
    mode: 'local',
  },
  i18nSource: (defaults) => ({
    ...defaults,
    outlineTitle: { ...defaults.outlineTitle, en: 'On this page' },
  }),
  themeConfig: {
    socialLinks: [{ icon: 'github', mode: 'link', content: REPO_URL }],
    enableContentAnimation: false,
    enableAppearanceAnimation: false,
  },
  builderConfig: {
    // `head` entries are tag-plus-attributes only, and JSON-LD needs a body.
    html: {
      tags: [
        {
          tag: 'script',
          attrs: { type: 'application/ld+json' },
          children: JSON.stringify(structuredData),
          head: true,
        },
      ],
    },
  },
};

export default defineConfig(config);
