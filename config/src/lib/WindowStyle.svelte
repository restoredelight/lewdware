<script lang="ts">
	import { clampScroll } from '$ui/scroll';
	import { store } from './store.svelte';
	import Card from '$ui/Card.svelte';
	import Select from '$ui/Select.svelte';
	import ThemePreview from './ThemePreview.svelte';

	// A card is current either by its own name or by the concrete look it resolves to. Someone
	// whose config pins `breeze`, for example, should see the merged "KDE Plasma" card selected.
	const selectedTheme = $derived(
		store.themeCatalogue.themes.find(
			(theme) => theme.name === store.config?.theme || theme.resolves_to === store.config?.theme
		) ?? null
	);

	// `auto` previews the appearance the engine will actually use. An unavailable or neutral
	// system answer falls back to light in both places.
	const previewAppearance = $derived(
		store.config?.appearance === 'light' || store.config?.appearance === 'dark'
			? store.config.appearance
			: (store.themeCatalogue.system_appearance ?? 'light')
	);

	const darkUnavailable = $derived(
		selectedTheme !== null && !selectedTheme.supports_dark && previewAppearance === 'dark'
	);
</script>

<div class="flex-1 overflow-y-auto" use:clampScroll>
	<div class="mx-auto flex w-full max-w-4xl flex-col gap-6 p-8">
		<header class="max-w-2xl">
			<h1 class="ui-page-title">Window style</h1>
			<p class="text-muted mt-1.5 mb-0 text-sm">
				Choose how popup frames, buttons and text fields look. A mode can still choose a style for a
				particular window where the look is part of what it&rsquo;s doing.
			</p>
		</header>

		<section class="flex flex-col gap-2">
			<div class="flex items-end justify-between gap-4">
				<div>
					<h2 class="ui-section-title">Theme</h2>
					<p class="text-muted mt-1 mb-0 text-xs">
						Each preview is live &mdash; hover its close button, press a button, or type in the
						field.
					</p>
				</div>
				<Select
					class="w-44"
					size="compact"
					label="Light or dark"
					value={store.config?.appearance ?? ''}
					options={store.themeCatalogue.appearances.map((appearance) => ({
						value: appearance.name,
						label: appearance.label
					}))}
					onchange={(value) => store.setAppearance(value)}
				/>
			</div>

			<Card class="flex flex-col gap-4 p-4">
				<div class="grid grid-cols-[repeat(auto-fill,minmax(270px,1fr))] gap-3">
					{#each store.themeCatalogue.themes as theme (theme.name)}
						{@const selected = theme === selectedTheme}
						<div
							class="border-border bg-bg flex flex-col gap-2 rounded-sm border p-3 transition-colors"
							class:!border-[var(--ui-accent)]={selected}
						>
							<div class="flex items-baseline justify-between gap-2">
								<label class="flex cursor-pointer items-center gap-2 text-sm">
									<input
										type="radio"
										name="window-theme"
										class="accent-[var(--ui-accent)]"
										checked={selected}
										onchange={() => store.setTheme(theme.name)}
									/>
									<span class="text-text font-medium">{theme.label}</span>
								</label>
								{#if theme.matches_system}
									<span class="text-muted font-mono text-[10px]">
										{theme.name === 'native-retro' ? 'your system, retro' : 'matches your system'}
									</span>
								{:else if !theme.supports_dark}
									<span class="text-muted font-mono text-[10px]">light only</span>
								{/if}
							</div>
							<!-- The preview is decoration for the radio control. It remains interactive because
							     trying the widgets is the point of the preview, not a second selection mechanism. -->
							<ThemePreview
								look={previewAppearance === 'dark' ? theme.dark : theme.light}
								title={theme.label}
							/>
						</div>
					{/each}
				</div>

				{#if darkUnavailable}
					<p class="text-muted m-0 text-xs">
						{selectedTheme?.label} has no dark version and is always drawn light.
					</p>
				{:else if selectedTheme?.matches_system}
					<p class="text-muted m-0 text-xs">
						Windows are drawn to look like this machine&rsquo;s own, so a pack you share won&rsquo;t
						look the same on someone else&rsquo;s.
					</p>
				{/if}
			</Card>
		</section>
	</div>
</div>
