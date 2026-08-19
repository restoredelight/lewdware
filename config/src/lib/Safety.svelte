<script lang="ts">
	import { clampScroll } from '$ui/scroll';
	import { onMount } from 'svelte';
	import { store } from './store.svelte';
	import { api } from './api';
	import { MODIFIER_KEYS, formatKey } from './keys';
	import Toggle from '$ui/Toggle.svelte';
	import Card from '$ui/Card.svelte';
	import Button from '$ui/Button.svelte';
	import RadioGroup from '$ui/RadioGroup.svelte';
	import { taskFeedback } from '$ui/taskFeedback.svelte.js';
	import type { Capabilities, Key, WallpaperSupportDto } from './types';

	// Read once on mount rather than polled: it runs a real snapshot against the desktop, and the
	// answer only changes across a session switch, by which point this page is being re-opened.
	let support = $state<WallpaperSupportDto | null>(null);
	let preview = $state<string | null>(null);
	let picking = $state(false);
	let adoptingImage = $state(false);
	let inputMonitoringGranted = $state(true);
	let inputMonitoringPromptFailed = $state(false);
	let recording = $state(false);

	const restore = $derived(store.config?.wallpaper.restore ?? { kind: 'original' as const });
	const restoreImage = $derived(restore.kind === 'image' ? restore.path : null);

	// `support` being null means the probe failed; assume the original is restorable rather than
	// pushing the user into picking an image they may not need.
	const canRestoreOriginal = $derived(support?.can_restore_original ?? true);

	// What the "Change wallpaper" row actually reports. A permission that is switched on but can
	// never take effect should not claim to be "Allowed".
	const wallpaperUsable = $derived(canRestoreOriginal || restore.kind === 'image');

	const panicKeyDisplay = $derived(
		recording ? 'Press a key…' : store.config ? formatKey(store.config.panic_button) : ''
	);

	onMount(async () => {
		support = await api.wallpaperSupport().catch(() => null);
		await checkInputMonitoringGranted();
	});

	$effect(() => {
		const path = restoreImage;
		if (!path) {
			preview = null;
			return;
		}
		let cancelled = false;
		api
			.wallpaperRestorePreview(path)
			.then((url) => {
				if (!cancelled) preview = url;
			})
			.catch(() => {
				if (!cancelled) preview = null;
			});
		return () => {
			cancelled = true;
		};
	});

	async function checkInputMonitoringGranted() {
		try {
			inputMonitoringGranted = await api.inputMonitoringGranted();
		} catch (err) {
			taskFeedback.warning(
				'input-monitoring',
				`Couldn’t check Input Monitoring permission: ${String(err)}`
			);
		}
	}

	async function openInputMonitoringSettings() {
		try {
			const granted = await api.requestInputMonitoring();
			if (granted) {
				inputMonitoringGranted = true;
				taskFeedback.dismiss('input-monitoring');
			} else {
				inputMonitoringPromptFailed = true;
			}
		} catch (err) {
			inputMonitoringPromptFailed = true;
			taskFeedback.error(
				'input-monitoring',
				`Couldn’t request Input Monitoring permission: ${String(err)}`
			);
		}
	}

	function handleKeyDown(e: KeyboardEvent) {
		if (!recording) return;
		if (MODIFIER_KEYS.has(e.key)) return;

		e.preventDefault();

		store.setPanicButton({
			name: e.key === ' ' ? 'Space' : e.key,
			code: e.code,
			modifiers: {
				ctrl: e.ctrlKey,
				alt: e.altKey,
				shift: e.shiftKey,
				meta: e.metaKey
			}
		} satisfies Key);

		recording = false;
	}

	// Picking the image option must never leave the choice empty, so it adopts the bundled
	// near-black placeholder straight away -- visible in the preview, and obviously a placeholder,
	// which is the nudge to replace it with something deliberate.
	async function chooseImageOption() {
		if (restore.kind === 'image') return;
		const path = await api.defaultRestoreImage().catch(() => null);
		if (path) store.setWallpaperRestore({ kind: 'image', path });
	}

	async function pickImage() {
		picking = true;
		try {
			const path = await api.pickRestoreImage(() => (adoptingImage = true));
			if (path) store.setWallpaperRestore({ kind: 'image', path });
		} catch {
			// The dialog was dismissed or the copy failed; leave the current choice alone.
		} finally {
			adoptingImage = false;
			picking = false;
		}
	}

	const toggles: { key: keyof Capabilities; label: string; description: string }[] = [
		{
			key: 'set_wallpaper',
			label: 'Change wallpaper',
			description: 'Allow the pack/mode to set your desktop wallpaper.'
		},
		{
			key: 'open_links',
			label: 'Open links',
			description: 'Allow the pack/mode to open links in your browser.'
		},
		{
			key: 'send_notifications',
			label: 'Show notifications',
			description: 'Allow the pack/mode to show desktop notifications.'
		}
	];
</script>

<div class="flex-1 overflow-y-auto" use:clampScroll>
	<div class="mx-auto flex w-full max-w-4xl flex-col gap-6 p-8">
		<header class="max-w-2xl">
			<h1 class="ui-page-title">Safety</h1>
			<p class="text-muted mt-1.5 mb-0 text-sm">
				Control how to stop a running session and what packs and modes may do outside their windows.
			</p>
		</header>

		<!-- Panic key -->
		<section class="flex flex-col gap-2">
			<h2 class="ui-section-title">Panic key</h2>
			<p class="text-muted text-xs">Pressing this key combination closes the app immediately.</p>
			{#if !inputMonitoringGranted}
				<div
					class="flex flex-col gap-2 rounded-md border border-[var(--ui-warning-border)] bg-[var(--ui-warning-bg)] px-3 py-2 text-sm text-[var(--ui-warning)]"
				>
					<div class="flex items-center gap-3">
						<span>The panic key requires Input Monitoring permission.</span>
						<Button
							size="compact"
							variant="secondary"
							class="ml-auto"
							onclick={openInputMonitoringSettings}>Open Settings</Button
						>
					</div>
					{#if inputMonitoringPromptFailed}
						<p class="text-xs">
							The permission prompt could not be shown (the app may need to be signed). To enable
							manually: open <button
								onclick={() => api.openInputMonitoringSettings()}
								class="underline transition-colors hover:text-white"
								>System Settings → Privacy &amp; Security → Input Monitoring</button
							>
							and add Lewdware, then restart the app.
						</p>
					{/if}
				</div>
			{/if}
			<Card class="flex items-center justify-between gap-4 p-4">
				<div>
					<h3 class="text-text m-0 text-sm font-medium">Current shortcut</h3>
					<p class="text-muted m-0 mt-1 text-xs">
						Click the shortcut, then press the new key combination.
					</p>
				</div>
				<button
					type="button"
					aria-label={recording
						? 'Recording panic key'
						: `Change panic key, currently ${panicKeyDisplay}`}
					class="min-w-40 cursor-pointer rounded-md border-2 px-4 py-2 text-sm font-semibold transition-colors select-none
             {recording
						? 'bg-accent/10 border-accent text-accent-foreground'
						: 'bg-bg border-border text-text hover:border-[var(--ui-border-strong)]'}"
					onclick={() => (recording = true)}
					onkeydown={handleKeyDown}
					onblur={() => (recording = false)}
				>
					{#if recording}<span class="font-normal italic">Press a key…</span>{:else}<kbd
							>{panicKeyDisplay}</kbd
						>{/if}
				</button>
			</Card>
		</section>

		<!-- Permissions -->
		<section class="border-border flex flex-col gap-2 border-t pt-6">
			<h2 class="ui-section-title">Permissions</h2>
			<p class="text-muted text-xs">Control what the running pack or mode is allowed to do.</p>
			<Card class="divide-border divide-y">
				{#each toggles as toggle (toggle.key)}
					{@const allowed = store.config?.capabilities[toggle.key] ?? false}
					{@const usable = toggle.key !== 'set_wallpaper' || wallpaperUsable}
					<div class="flex items-center gap-4 px-4 py-3">
						<div class="min-w-0 flex-1">
							<h3 class="text-text m-0 text-sm font-medium">{toggle.label}</h3>
							<p class="text-muted m-0 mt-1 text-xs">{toggle.description}</p>
						</div>
						<span class="text-xs font-medium {allowed && usable ? 'text-text' : 'text-muted'}"
							>{!allowed ? 'Denied' : usable ? 'Allowed' : 'Unavailable'}</span
						>
						<Toggle
							checked={allowed}
							ariaLabel={`Allow ${toggle.label.toLowerCase()}`}
							onchange={(checked) => store.setCapability(toggle.key, checked)}
						/>
					</div>

					{#if toggle.key === 'set_wallpaper' && allowed}
						<div class="bg-bg flex flex-col gap-3 px-4 py-3">
							<div>
								<h4 class="text-text m-0 text-xs font-medium">
									When Lewdware stops, set my wallpaper to
								</h4>
							</div>

							<RadioGroup
								ariaLabel="What to put the wallpaper back to"
								value={restore.kind}
								options={[
									{
										value: 'original',
										label: 'Whatever it was before',
										description: canRestoreOriginal ? undefined : 'Unavailable on this desktop',
										disabled: !canRestoreOriginal
									},
									{ value: 'image', label: 'This image' }
								]}
								onchange={(kind) =>
									kind === 'original'
										? store.setWallpaperRestore({ kind: 'original' })
										: chooseImageOption()}
							/>

							{#if restore.kind === 'image'}
								<div class="flex items-center gap-3 pl-9">
									<div
										class="border-border bg-bg h-14 w-24 shrink-0 overflow-hidden rounded border"
									>
										{#if preview}
											<img
												src={preview}
												alt="Wallpaper restored when Lewdware stops"
												class="h-full w-full object-cover"
											/>
										{/if}
									</div>
									<div class="min-w-0 flex-1">
										<p class="text-muted m-0 truncate font-mono text-[11px]" title={restoreImage}>
											{restoreImage}
										</p>
										{#if !preview}
											<p class="text-muted m-0 mt-1 text-xs">
												This image can’t be read any more. Choose another.
											</p>
										{/if}
									</div>
									<Button
										size="compact"
										disabled={picking}
										loading={adoptingImage}
										onclick={pickImage}>Choose…</Button
									>
								</div>
							{/if}
						</div>
					{/if}
				{/each}
			</Card>
		</section>
	</div>
</div>
