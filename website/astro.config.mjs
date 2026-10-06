// @ts-check
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';
import { fontLinks } from './src/fonts';

const group = (label, pt, directory) => ({
  label,
  translations: { 'pt-BR': pt },
  items: [{ autogenerate: { directory } }],
});

export default defineConfig({
  site: 'https://carmy-pi.vercel.app',
  integrations: [
    starlight({
      title: 'Carmy',
      description:
        'Composable Agent Runtime for Managed Yield: deterministic, safe execution infrastructure for AI agents in Rust.',
      favicon: '/favicon.png',
      defaultLocale: 'root',
      locales: {
        root: { label: 'English', lang: 'en' },
        'pt-br': { label: 'Português do Brasil', lang: 'pt-BR' },
      },
      social: [
        { icon: 'github', label: 'GitHub', href: 'https://github.com/igorvieira/Carmy' },
        { icon: 'heart', label: 'Sponsor', href: 'https://github.com/sponsors/igorvieira' },
        // { icon: 'discord', label: 'Discord', href: 'https://discord.gg/...' },
      ],
      editLink: { baseUrl: 'https://github.com/igorvieira/Carmy/edit/main/website/' },
      head: fontLinks.map((attrs) => ({ tag: 'link', attrs })),
      components: { SiteTitle: './src/components/SiteTitle.astro' },
      customCss: ['./src/styles/carmy.css'],
      sidebar: [
        group('Getting started', 'Primeiros passos', 'getting-started'),
        group('Guides', 'Guias', 'guides'),
        group('Transports', 'Transportes', 'transports'),
        group('Reference', 'Referência', 'reference'),
        { label: 'Sponsor', translations: { 'pt-BR': 'Apoie' }, link: '/sponsor/' },
      ],
    }),
  ],
});
