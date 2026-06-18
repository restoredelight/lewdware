// @ts-check
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';

export default defineConfig({
	integrations: [
		starlight({
			title: 'Lewdware',
			social: [{ icon: 'github', label: 'GitHub', href: 'https://github.com/restoredelight/lewdware' }],
			sidebar: [
				{
					label: 'Download',
					items: [
						{ label: 'Lewdware', slug: 'download' },
						{ label: 'Pack Editor', slug: 'download/pack-editor' },
					],
				},
				{
					label: 'Reference',
					items: [
						{ label: 'Lua API', link: '/reference/lua-api/' },
						{ label: 'Mode Config', slug: 'reference/mode-config' },
					],
				},
			],
		}),
	],
	site: "https://lewdware.net",
});
