<script lang="ts">
	// Shared by both apps, which differ only in the name they call themselves and in how they
	// reach the backend. The Tauri calls stay at the call site so nothing in `shared-ui/` depends
	// on Tauri: `check` answers a download URL, or `null` when the installed version is current.
	import { onMount } from 'svelte';
	import { Icon, XMark } from '$icons';

	let {
		appName,
		check,
		open
	}: {
		appName: string;
		check: () => Promise<string | null>;
		open: (url: string) => void;
	} = $props();

	let downloadUrl = $state<string | null>(null);
	let dismissed = $state(false);

	onMount(async () => {
		try {
			downloadUrl = await check();
		} catch {
			// Network failure or offline — silently ignore
		}
	});
</script>

{#if downloadUrl && !dismissed}
	<div class="banner">
		<span class="message">A new version of {appName} is available.</span>
		<button class="download" onclick={() => open(downloadUrl!)}>Download update</button>
		<button
			class="dismiss"
			aria-label="Dismiss"
			onclick={() => {
				dismissed = true;
			}}
		>
			<Icon src={XMark} mini size="16px" />
		</button>
	</div>
{/if}

<style>
	.banner {
		display: flex;
		align-items: center;
		padding: 8px 16px;
		border-bottom: 1px solid color-mix(in srgb, var(--ui-accent) 30%, transparent);
		background: color-mix(in srgb, var(--ui-accent) 10%, transparent);
		color: var(--ui-text);
		font-size: 14px;
		gap: 12px;
	}

	.message {
		flex: 1;
	}

	.download,
	.dismiss {
		padding: 0;
		border: 0;
		background: transparent;
		color: inherit;
		font: inherit;
		cursor: pointer;
	}

	.download {
		font-weight: 500;
		text-decoration: underline;
	}

	.download:hover {
		color: var(--ui-accent-foreground);
	}

	.dismiss {
		display: flex;
		margin-left: 4px;
		opacity: 0.6;
	}

	.dismiss:hover {
		opacity: 1;
	}
</style>
