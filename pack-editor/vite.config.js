import { defineConfig } from 'vite';
import { sveltekit } from '@sveltejs/kit/vite';
import tailwindcss from '@tailwindcss/vite';

const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
export default defineConfig(async () => ({
	// Svelte must own its virtual `?svelte&type=style` modules before Tailwind transforms the
	// extracted CSS. In dev/HMR, the reverse order can feed the complete component source to
	// Tailwind as CSS for shared components outside this app root.
	plugins: [sveltekit(), tailwindcss()],

	// Tests run against a DOM, and against the *client* build of Svelte.
	//
	// Both are needed for anything that uses runes. Without `conditions: ['browser']` the Svelte
	// plugin compiles for the server, where `$effect` is a no-op — so an effect-driven module looks
	// inert and its tests pass by never running. And runes only compile in files the plugin owns,
	// which means `*.svelte.ts`; a test for such a module is therefore `*.test.svelte.ts`, which
	// vitest's default `include` does not match.
	test: {
		environment: 'happy-dom',
		include: ['src/**/*.{test,spec}.{js,ts}', 'src/**/*.{test,spec}.svelte.{js,ts}']
	},
	resolve: {
		conditions: process.env.VITEST ? ['browser'] : undefined
	},

	// Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
	//
	// 1. prevent Vite from obscuring rust errors
	clearScreen: false,
	// 2. tauri expects a fixed port, fail if that port is not available
	server: {
		// Shared Svelte primitives live beside this app at ../shared-ui.
		fs: {
			allow: ['..']
		},
		port: 1421,
		strictPort: true,
		host: host || false,
		hmr: host
			? {
					protocol: 'ws',
					host,
					port: 1421
				}
			: undefined,
		watch: {
			// 3. tell Vite to ignore watching `src-tauri`
			ignored: ['**/src-tauri/**']
		}
	}
}));
