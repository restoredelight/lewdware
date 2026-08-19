<script lang="ts">
	import { clampScroll } from '$ui/scroll';
	import { store } from './store.svelte';
	import { api } from './api';
	import { taskFeedback } from '$ui/taskFeedback.svelte.js';
	import Slider from '$ui/Slider.svelte';
	import Card from '$ui/Card.svelte';
	import Button from '$ui/Button.svelte';
	import IconButton from '$ui/IconButton.svelte';
	import Select from '$ui/Select.svelte';
	import { ArrowPath, Icon } from '$icons';
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

	// The list is only fetched when this page is shown -- see `store.audioDevices`.
	$effect(() => {
		if (store.audioDevices.length === 0 && !store.audioDevicesLoading && !store.audioDevicesError)
			void store.loadAudioDevices();
	});

	// `''` rather than `null`, because `Select` deals in strings. Mapped back at the boundary.
	const SYSTEM_DEFAULT = '';

	const chosen = $derived(store.config?.audio_device ?? null);
	const selected = $derived(chosen?.id ?? SYSTEM_DEFAULT);

	// What "System default" currently resolves to, so the choice isn't abstract.
	const systemDefaultName = $derived(store.audioDevices.find((d) => d.is_default)?.name ?? null);

	// A device saved on another machine, or one that has since been unplugged. The engine falls back
	// to the default for that session and keeps the setting, so this is worth saying plainly rather
	// than silently showing a blank picker.
	const missingDevice = $derived(
		selected !== SYSTEM_DEFAULT &&
			!store.audioDevicesLoading &&
			!store.audioDevicesError &&
			store.audioDevices.length > 0 &&
			!store.audioDevices.some((device) => device.id === selected)
	);

	const options = $derived([
		{ value: SYSTEM_DEFAULT, label: 'System default' },
		// Connected devices are always labelled from the live list rather than from anything stored,
		// so a device renamed since it was chosen shows its current name.
		...store.audioDevices.map((device) => ({ value: device.id, label: device.name })),
		// A saved device that isn't in the list keeps an entry of its own, so the picker shows that
		// something *is* chosen instead of falling through to `Select`'s "Select…" placeholder and
		// reading as though nothing were. Named from the choice we stored -- this is the one case
		// that name exists for. Disabled, since it can't be picked afresh.
		...(missingDevice && chosen
			? [{ value: chosen.id, label: `${chosen.name} (not connected)`, disabled: true }]
			: [])
	]);

	let testing = $state(false);

	async function testAudio() {
		testing = true;
		taskFeedback.dismiss('test-audio');
		try {
			const result = await api.testAudioDevice(selected === SYSTEM_DEFAULT ? null : selected);
			if (result.fell_back) {
				taskFeedback.warning(
					'test-audio',
					'That output isn’t available — the sound played on the system default'
				);
			} else {
				// Hearing it is the confirmation; success is silent.
				taskFeedback.success('test-audio');
			}
		} catch (err) {
			taskFeedback.error('test-audio', `Test sound failed: ${err}`);
		} finally {
			testing = false;
		}
	}
</script>

<div class="flex-1 overflow-y-auto" use:clampScroll>
	<div class="mx-auto flex w-full max-w-4xl flex-col gap-6 p-8">
		<header class="max-w-2xl">
			<h1 class="ui-page-title">Audio</h1>
			<p class="text-muted mt-1.5 mb-0 text-sm">
				Choose where sound comes out, and how loudly media plays.
			</p>
		</header>

		<section class="flex flex-col gap-2">
			<h2 class="ui-section-title">Output device</h2>
			<p class="text-muted text-xs">
				Which output popups and sounds play through. Applies to sounds started from now on; a
				session already running keeps the device it opened with.
			</p>

			<Card class="flex flex-col gap-3 p-4">
				{#if store.audioDevicesError}
					<div class="flex flex-col items-center gap-2 py-2 text-center">
						<p class="text-text m-0 text-xs font-semibold">Audio outputs couldn’t be detected</p>
						<p class="text-muted m-0 max-w-md text-xs">{store.audioDevicesError}</p>
						<Button class="mt-1" size="compact" onclick={() => store.loadAudioDevices()}>
							Try again
						</Button>
					</div>
				{:else}
					<div class="flex items-end gap-2">
						<Select
							class="min-w-0 flex-1"
							label="Output device"
							hideLabel
							value={selected}
							{options}
							disabled={store.audioDevicesLoading}
							onchange={(value) =>
								store.setAudioDevice(
									value === SYSTEM_DEFAULT
										? null
										: {
												id: value,
												name:
													store.audioDevices.find((device) => device.id === value)?.name ?? value
											}
								)}
						/>
						<IconButton
							label="Rescan outputs"
							size="normal"
							onclick={() => store.loadAudioDevices()}
							disabled={store.audioDevicesLoading}
						>
							<span class="block h-4 w-4"><Icon src={ArrowPath} mini /></span>
						</IconButton>
					</div>

					<div class="flex items-center justify-between gap-3">
						<p class="text-muted m-0 font-mono text-[11px]">
							{#if store.audioDevicesLoading}
								Detecting outputs…
							{:else if missingDevice && chosen}
								<span class="text-[var(--color-warning)]">
									{chosen.name} isn’t connected — sound falls back to the system default
								</span>
							{:else if selected === SYSTEM_DEFAULT && systemDefaultName}
								Currently {systemDefaultName}
							{:else if store.audioDevices.length > 0}
								{store.audioDevices.length}
								{store.audioDevices.length === 1 ? 'output' : 'outputs'} available
							{/if}
						</p>
						<Button size="compact" loading={testing} onclick={testAudio}>Test sound</Button>
					</div>
				{/if}
			</Card>
		</section>

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
	</div>
</div>
