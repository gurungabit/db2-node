import { defineConfig } from 'vitepress'

export default defineConfig({
  title: 'db2-node',
  description: 'A zero-dependency DB2 driver for Node.js, written in Rust and speaking DRDA directly',
  base: '/db2-node/',
  cleanUrls: true,
  lastUpdated: true,
  themeConfig: {
    nav: [
      { text: 'Guide', link: '/getting-started/' },
      { text: 'API', link: '/api-reference/' },
      { text: 'Data Types', link: '/data-types/' },
      { text: 'Architecture', link: '/architecture/' },
      { text: 'Contributing', link: '/contributing/' },
    ],
    sidebar: [
      {
        text: 'db2-node',
        items: [
          { text: 'Overview', link: '/' },
          { text: 'Getting Started', link: '/getting-started/' },
          { text: 'API Reference', link: '/api-reference/' },
          { text: 'Data Types', link: '/data-types/' },
          { text: 'Architecture', link: '/architecture/' },
          { text: 'Protocol', link: '/protocol/' },
          { text: 'Contributing', link: '/contributing/' },
        ],
      },
    ],
    search: {
      provider: 'local',
    },
    socialLinks: [
      { icon: 'github', link: 'https://github.com/gurungabit/db2-node' },
    ],
    editLink: {
      pattern: 'https://github.com/gurungabit/db2-node/edit/main/docs/:path',
      text: 'Edit this page on GitHub',
    },
    footer: {
      message: 'Released under the MIT License.',
      copyright: 'Copyright © 2026 db2-node contributors',
    },
  },
})
