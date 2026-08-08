<script lang="ts">
	import { onDestroy, onMount } from 'svelte';
	import { api } from './api.js';
	import { store } from './store.svelte.js';
	import TagPicker from './TagPicker.svelte';
	import TextPoolEditor from './TextPoolEditor.svelte';
	import ContentGroupsEditor from './ContentGroupsEditor.svelte';
	import WebLinksEditor from './WebLinksEditor.svelte';
	import Tabs from '$ui/Tabs.svelte';
	import Select, { type SelectOption } from '$ui/Select.svelte';
	import {
		flushBehaviourSave,
		initializeBehaviourHistory,
		scheduleBehaviourSave
	} from './behaviourSave.svelte.js';

	type Tab =
		| 'groups'
		| 'captions'
		| 'prompts'
		| 'notifications'
		| 'subliminals'
		| 'web_links'
		| 'wallpaper'
		| 'appearance';

	// The engine's theme names, in the order it offers them. `native`/`native-retro` are
	// deliberately absent: they are the *user's* answer to "match my system", and a pack declaring
	// one would be telling every machine to look like itself. A pack names a look or says nothing.
	const themes: SelectOption[] = [
		{ value: '', label: 'No preference — leave it to the user' },
		{ value: 'plain', label: 'Plain' },
		{ value: 'fluent', label: 'Windows 11' },
		{ value: 'redmond', label: 'Windows 95' },
		{ value: 'aqua', label: 'macOS' },
		{ value: 'adwaita', label: 'GNOME' },
		{ value: 'platinum', label: 'Mac OS 9' }
	];

	const tabs = $derived<{ id: Tab; label: string; group: string; badge?: number }[]>([
		{
			id: 'groups',
			label: 'Content Groups',
			group: 'Organization',
			badge: store.behaviour?.content.content_groups.length
		},
		{
			id: 'captions',
			label: 'Captions',
			group: 'Messages',
			badge: store.behaviour?.content.captions.length
		},
		{
			id: 'prompts',
			label: 'Prompts',
			group: 'Messages',
			badge: store.behaviour?.content.prompts.length
		},
		{
			id: 'notifications',
			label: 'Notifications',
			group: 'Messages',
			badge: store.behaviour?.content.notifications.length
		},
		{
			id: 'subliminals',
			label: 'Subliminals',
			group: 'Messages',
			badge: store.behaviour?.content.subliminals.length
		},
		{
			id: 'web_links',
			label: 'Web Links',
			group: 'Other',
			badge: store.behaviour?.content.web_links.length
		},
		{ id: 'wallpaper', label: 'Wallpaper & Splash', group: 'Other' },
		{ id: 'appearance', label: 'Window Style', group: 'Other' }
	]);

	const sectionInfo: Record<Tab, { title: string; description: string }> = {
		groups: {
			title: 'Content Groups',
			description:
				'Create collections people can enable or disable. Media and messages with any of a group’s tags belong to that collection.'
		},
		captions: {
			title: 'Captions',
			description:
				'Short messages shown alongside popup media. Tagged captions are only used with media carrying a matching tag.'
		},
		prompts: { title: 'Prompts', description: 'Questions that ask the user for a typed response.' },
		notifications: {
			title: 'Notifications',
			description: 'Messages displayed as desktop notifications.'
		},
		subliminals: {
			title: 'Subliminals',
			description: 'Brief text overlays flashed during a session.'
		},
		web_links: {
			title: 'Web Links',
			description: 'Links the experience may open in the user’s browser.'
		},
		wallpaper: {
			title: 'Wallpaper & Splash',
			description: 'Choose which tagged media can be used as wallpaper or as the startup image.'
		},
		appearance: {
			title: 'Window Style',
			description:
				'The look you designed this pack around. Applies when someone runs it with the Sequence mode, which follows the pack author’s design — they can still override it.'
		}
	};

	let activeTab = $state<Tab>('groups');
	let narrowWindow = $state(false);
	let panel = $state<HTMLDivElement>();

	// WebKitGTK doesn't reliably clamp scrollTop when the panel's content shrinks,
	// leaving a shorter tab blank and unscrollable.
	$effect(() => {
		activeTab;
		panel?.scrollTo(0, 0);
	});

	onMount(() => {
		const query = window.matchMedia('(max-width: 700px)');
		const update = () => (narrowWindow = query.matches);
		update();
		query.addEventListener('change', update);
		return () => query.removeEventListener('change', update);
	});

	onMount(async () => {
		if (store.behaviour === null) store.behaviour = await api.getBehaviour();
		initializeBehaviourHistory(store.behaviour);
	});

	onDestroy(() => {
		flushBehaviourSave();
	});
</script>

<div class="flex min-h-0 w-full flex-1 flex-col">
	{#if store.behaviour === null}
		<p class="text-muted p-6 text-sm">Loading…</p>
	{:else}
		<div class="flex min-h-0 flex-1 max-[700px]:flex-col">
			<aside
				class="border-border bg-surface w-48 shrink-0 border-r p-3 max-[900px]:w-40 max-[700px]:w-full max-[700px]:border-r-0 max-[700px]:border-b max-[700px]:py-0"
			>
				<Tabs
					{tabs}
					active={activeTab}
					orientation={narrowWindow ? 'horizontal' : 'vertical'}
					onselect={(id) => (activeTab = id as Tab)}
				/>
			</aside>

			<div class="min-w-0 flex-1 overflow-y-auto p-6 max-[700px]:p-4" bind:this={panel}>
				<div class="mx-auto w-full max-w-[800px]">
					<div class="mb-5 max-w-2xl">
						<h2 class="ui-page-title">{sectionInfo[activeTab].title}</h2>
						<p class="text-muted mt-1 text-sm">{sectionInfo[activeTab].description}</p>
					</div>
					{#if activeTab === 'groups'}
						<ContentGroupsEditor />
					{:else if activeTab === 'captions'}
						<TextPoolEditor title="Captions" poolKey="captions" idPrefix="caption" />
					{:else if activeTab === 'prompts'}
						<div class="flex flex-col gap-3">
							<TextPoolEditor title="Prompts" poolKey="prompts" idPrefix="prompt" />
							<label class="flex flex-col gap-[5px]"
								><span class="text-text text-xs font-semibold">Submit button label</span><input
									bind:value={store.behaviour!.content.prompt_settings.submit_label}
									oninput={scheduleBehaviourSave}
									placeholder="Submit"
									class="border-border bg-surface text-text w-48 rounded-sm border px-2.5 py-2 text-sm transition-colors hover:border-[var(--ui-border-strong)]"
								/></label
							>
						</div>
					{:else if activeTab === 'notifications'}
						<TextPoolEditor title="Notifications" poolKey="notifications" idPrefix="notification" />
					{:else if activeTab === 'subliminals'}
						<TextPoolEditor title="Subliminals" poolKey="subliminals" idPrefix="subliminal" />
					{:else if activeTab === 'web_links'}
						<WebLinksEditor />
					{:else if activeTab === 'wallpaper'}
						<div class="flex flex-col gap-6">
							<section class="flex flex-col gap-2">
								<div>
									<h3 class="text-text text-sm font-semibold">Wallpaper</h3>
									<p class="text-muted text-xs">
										Tags identifying wallpaper media. Leave empty to disable engine-managed
										wallpaper.
									</p>
								</div>
								<TagPicker
									tags={store.behaviour!.content.wallpaper_tags}
									id="wallpaper-tags"
									onchange={(tags) => (store.behaviour!.content.wallpaper_tags = tags)}
								/>{#if store.behaviour!.content.wallpaper_tags.length === 0}<p
										class="text-muted text-xs italic"
									>
										No wallpaper tags selected. Lewdware will not change the wallpaper.
									</p>{/if}
							</section>
							<section class="flex flex-col gap-2">
								<div>
									<h3 class="text-text text-sm font-semibold">Splash</h3>
									<p class="text-muted text-xs">
										Tags identifying a startup splash image. Leave empty to disable it.
									</p>
								</div>
								<TagPicker
									tags={store.behaviour!.content.splash_tags}
									id="splash-tags"
									onchange={(tags) => (store.behaviour!.content.splash_tags = tags)}
								/>{#if store.behaviour!.content.splash_tags.length === 0}<p
										class="text-muted text-xs italic"
									>
										No splash tags selected. No startup image will be shown.
									</p>{/if}
							</section>
						</div>
					{:else if activeTab === 'appearance'}
						<div class="flex max-w-md flex-col gap-2">
							<Select
								label="Window style"
								value={store.behaviour!.content.theme ?? ''}
								options={themes}
								description="How popup frames, buttons and text fields look."
								onchange={(value) => {
									// The engine omits `theme` entirely when unset, so a pack written before
									// this existed has no key at all — normalize to `null` rather than
									// leaving `undefined` in the document.
									store.behaviour!.content.theme = value === '' ? null : value;
									scheduleBehaviourSave();
								}}
							/>
							{#if (store.behaviour!.content.theme ?? null) === null}
								<p class="text-muted text-xs italic">
									No preference. Whoever runs this pack sees their own choice of style, which
									defaults to matching their system.
								</p>
							{/if}
						</div>
					{/if}
				</div>
			</div>
		</div>
	{/if}
</div>
