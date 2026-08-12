<script lang="ts">
	import { clampScroll } from '$ui/scroll';
	import { store } from './store.svelte';
	import Checkbox from '$ui/Checkbox.svelte';
	import Card from '$ui/Card.svelte';
	import Button from '$ui/Button.svelte';
	import MonitorAreas from './MonitorAreas.svelte';
	import { isFullRegion } from './types';
</script>

<div class="flex-1 overflow-y-auto" use:clampScroll>
	<div class="mx-auto flex w-full max-w-4xl flex-col gap-6 p-8">
		<header class="max-w-2xl">
			<h1 class="ui-page-title">Monitors</h1>
			<p class="text-muted mt-1.5 mb-0 text-sm">
				Choose which screens popup windows may use, and how much of each one. Drag out an area on a
				screen to keep popups inside it; the rest of that screen is left alone.
			</p>
		</header>

		<section class="flex flex-col gap-2">
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
				<MonitorAreas
					monitors={store.monitors}
					onchange={(id, region) => store.setMonitorRegion(id, region)}
				/>
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
							{#if !monitor.disabled && !isFullRegion(monitor.region)}
								<span class="text-muted shrink-0 font-mono text-[11px]">
									{Math.round(monitor.region.width * 100)}% × {Math.round(
										monitor.region.height * 100
									)}% area
								</span>
							{/if}
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
