// @ts-check
import { defineConfig } from "astro/config";
import starlight from "@astrojs/starlight";

export default defineConfig({
	integrations: [
		starlight({
			title: "Lewdware",
			customCss: ['./src/styles/custom.css'],
			social: [
				{
					icon: "github",
					label: "GitHub",
					href: "https://github.com/restoredelight/lewdware",
				},
			],
			sidebar: [
				{
					label: "Download",
					items: [
						{ label: "Lewdware", slug: "download" },
						{ label: "Pack Editor", slug: "download/pack-editor" },
						{ label: "Packs", slug: "download/packs" },
					],
				},
				{
					label: "User Guides",
					items: [
						{ label: "Get started", slug: "user-guides/get-started"},
						{ label: "Comparison to Edgeware++", slug: "user-guides/comparison-to-edgeware"},
					],
				},
				{
					label: "Developer Guides",
					items: [
						{ label: "Create a pack", slug: "dev-guides/create-pack"},
						{ label: "Create a mode", slug: "dev-guides/create-mode"},
					],
				},
				{
					label: "Reference",
					items: [
						{ label: "Lua API", link: "reference/lua-api/" },
						{ label: "Mode Config", slug: "reference/mode-config" },
					],
				},
			],
		}),
	],
	site: "https://lewdware.net",
});
