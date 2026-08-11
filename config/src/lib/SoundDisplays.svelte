<script lang="ts">
	import { store } from './store.svelte';
	import Slider from '$ui/Slider.svelte';
	import Checkbox from '$ui/Checkbox.svelte';
	import Card from '$ui/Card.svelte';
	import Button from '$ui/Button.svelte';
	import type { Volume } from './types';

	const volumeSliders: { key: keyof Volume; label: string; description: string }[] = [
		{
			key: 'video',
			label: 'Video volume',
			description: "Master volume for a video popup's embedded audio track."
		},
		{
			key: 'audio',
			label: 'Audio volume',
			description: 'Master volume for standalone audio the pack or mode plays.'
		}
	];
</script>

<div class="flex-1 overflow-y-auto">
	<div class="mx-auto flex w-full max-w-4xl flex-col gap-6 p-8">
		<header class="max-w-2xl">
			<h1 class="ui-page-title">Sound &amp; displays</h1>
			<p class="text-muted mt-1.5 mb-0 text-sm">
				Control how loudly media plays and which monitors popup windows may use.
			</p>
		</header>

		<section class="flex flex-col gap-2">
			<h2 class="ui-section-title">Volume</h2>
			<p class="text-muted text-xs">
				Master volume, applied on top of whatever volume the pack or mode requests for a track.
			</p>
			<div class="grid grid-cols-2 gap-3">
				{#each volumeSliders as slider (slider.key)}
					<Card class="flex flex-col gap-3 p-4">
						<div class="flex items-center justify-between">
							<span class="text-text text-sm font-medium">{slider.label}</span>
							<span
								class="bg-bg text-text rounded px-2 py-1 font-mono text-[11px] font-semibold tabular-nums"
							>
								{Math.round((store.config?.volume[slider.key] ?? 0) * 100)}%
							</span>
						</div>
						<p class="text-muted m-0 text-xs">{slider.description}</p>
						<Slider
							ariaLabel={`${slider.label} volume`}
							min={0}
							max={1}
							step={0.01}
							value={store.config?.volume[slider.key] ?? 0}
							oninput={(value) => store.previewVolume(slider.key, value)}
							onchange={() => store.saveConfig()}
						/>
					</Card>
				{/each}
			</div>
		</section>

		<section class="border-border flex flex-col gap-2 border-t pt-6">
			<h2 class="ui-section-title">Monitors</h2>
			<p class="text-muted text-xs">Choose where popup media may appear.</p>
			{#if store.monitorsLoading}
				<Card class="border-dashed !border-[var(--ui-border-strong)] p-6 text-center">
					<p class="text-muted m-0 text-xs" role="status">Detecting monitors…</p>
				</Card>
			{:else if store.monitorsError}
				<Card
					class="flex flex-col items-center gap-2 border-dashed !border-[var(--ui-border-strong)] p-6 text-center"
				>
					<p class="text-text m-0 text-xs font-semibold">Monitors couldn’t be detected</p>
					<p class="text-muted m-0 max-w-md text-xs">{store.monitorsError}</p>
					<Button class="mt-1" size="compact" onclick={() => store.loadMonitors()}>Try again</Button
					>
				</Card>
			{:else if store.monitors.length > 0}
				<Card class="divide-border divide-y">
					{#each store.monitors as monitor (monitor.id)}
						<label
							class="hover:bg-surface-2 flex cursor-pointer items-center gap-3 px-4 py-3 transition-colors first:rounded-t-md last:rounded-b-md"
						>
							<Checkbox
								checked={!monitor.disabled}
								ariaLabel={monitor.name}
								onchange={(checked) => store.setMonitorEnabled(monitor.id, checked)}
							/>
							<span class="text-text min-w-0 flex-1 truncate text-sm">{monitor.name}</span>
							<span class="text-muted shrink-0 font-mono text-[11px]">
								{monitor.width}×{monitor.height}
							</span>
							{#if monitor.primary}
								<span
									class="border-border bg-bg text-muted rounded-full border px-2 py-0.5 text-[10px] font-semibold"
									>Primary</span
								>
							{/if}
						</label>
					{/each}
				</Card>
			{:else}
				<Card class="border-dashed !border-[var(--ui-border-strong)] p-6 text-center">
					<p class="text-muted m-0 text-xs">No monitors were detected.</p>
				</Card>
			{/if}
		</section>
	</div>
</div>
