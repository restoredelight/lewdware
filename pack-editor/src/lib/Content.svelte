<script lang="ts">
	import { clampScroll } from '$ui/scroll';
	import { onMount } from 'svelte';
	import { store } from './store.svelte.js';
	import type { ContentTab } from './store.svelte.js';
	import BehaviourGate from './BehaviourGate.svelte';
	import TextPoolEditor from './TextPoolEditor.svelte';
	import ContentGroupsEditor from './ContentGroupsEditor.svelte';
	import MediaSlot from './MediaSlot.svelte';
	import SubliminalPool from './SubliminalPool.svelte';
	import { SUBLIMINAL_TAG } from './tags.js';
	import WebLinksEditor from './WebLinksEditor.svelte';
	import Tabs from '$ui/Tabs.svelte';

	// The subliminal pool is media, not a behaviour field: membership is the managed tag every
	// file already carries in `store.files`, so the count is a filter rather than a query.
	const subliminalCount = $derived(
		store.files.filter((file) => file.tags.includes(SUBLIMINAL_TAG)).length
	);

	const tabs = $derived<{ id: ContentTab; label: string; group: string; badge?: number }[]>([
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
			badge: subliminalCount
		},
		{
			id: 'web_links',
			label: 'Web Links',
			group: 'Other',
			badge: store.behaviour?.content.web_links.length
		},
		{ id: 'wallpaper', label: 'Wallpaper & Splash', group: 'Other' }
	]);

	const sectionInfo: Record<ContentTab, { title: string; description: string }> = {
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
			description:
				'Messages displayed as desktop notifications. A notification can carry a title above its message; leave it blank to show the message on its own.'
		},
		subliminals: {
			title: 'Subliminals',
			description:
				'Videos layered over popups at low opacity — hypno spirals and anything else meant to sit on top.'
		},
		web_links: {
			title: 'Web Links',
			description: 'Links the experience may open in the user’s browser.'
		},
		wallpaper: {
			title: 'Wallpaper & Splash',
			description:
				'Pick the image Lewdware uses as the desktop wallpaper, and the one it shows when a session starts.'
		}
	};

	$effect(() => {
		if (store.contentTarget === null) return;
		store.contentTab = store.contentTarget.tab;
	});
	let narrowWindow = $state(false);
	let panel = $state<HTMLDivElement>();

	// WebKitGTK doesn't reliably clamp scrollTop when the panel's content shrinks,
	// leaving a shorter tab blank and unscrollable.
	$effect(() => {
		store.contentTab;
		panel?.scrollTo(0, 0);
	});

	onMount(() => {
		const query = window.matchMedia('(max-width: 900px)');
		const update = () => (narrowWindow = query.matches);
		update();
		query.addEventListener('change', update);
		return () => query.removeEventListener('change', update);
	});
</script>

<div class="flex min-h-0 w-full flex-1 flex-col">
	<BehaviourGate title="Content">
		<div class="flex min-h-0 flex-1 max-[900px]:flex-col">
			<aside
				class="border-border bg-surface w-48 shrink-0 border-r p-3 max-[900px]:w-full max-[900px]:border-r-0 max-[900px]:border-b max-[900px]:py-0"
			>
				<Tabs
					{tabs}
					active={store.contentTab}
					orientation={narrowWindow ? 'horizontal' : 'vertical'}
					onselect={(id) => {
						store.contentTarget = null;
						store.contentTab = id as ContentTab;
					}}
				/>
			</aside>

			<div class="min-w-0 flex-1 overflow-y-auto" bind:this={panel} use:clampScroll>
				<div
					class="mx-auto w-full max-w-[848px] px-6 pt-6 pb-6 max-[900px]:max-w-[832px] max-[900px]:px-4 max-[900px]:pt-4 max-[900px]:pb-4"
				>
					<p class="text-muted mb-4 text-xs">
						Read by the built-in modes (Sandbox and Sequence). A custom mode reads none of this.
					</p>
					<div class="mb-5 max-w-2xl">
						<h2 class="ui-page-title">{sectionInfo[store.contentTab].title}</h2>
						<p class="text-muted mt-1 text-sm">{sectionInfo[store.contentTab].description}</p>
					</div>
					{#if store.contentTab === 'groups'}
						<ContentGroupsEditor />
					{:else if store.contentTab === 'captions'}
						<TextPoolEditor title="Captions" poolKey="captions" idPrefix="caption" />
					{:else if store.contentTab === 'prompts'}
						<TextPoolEditor title="Prompts" poolKey="prompts" idPrefix="prompt" />
					{:else if store.contentTab === 'notifications'}
						<TextPoolEditor title="Notifications" poolKey="notifications" idPrefix="notification" />
					{:else if store.contentTab === 'subliminals'}
						<SubliminalPool
							revealId={store.contentTarget?.tab === 'subliminals'
								? store.contentTarget.fileId
								: null}
							onrevealed={() => (store.contentTarget = null)}
						/>
					{:else if store.contentTab === 'web_links'}
						<WebLinksEditor />
					{:else if store.contentTab === 'wallpaper'}
						<div class="flex flex-col gap-6">
							<MediaSlot
								slot={{ kind: 'wallpaper' }}
								mediaId={store.behaviour!.content.wallpaper}
								title="Wallpaper"
								description="The image Lewdware sets as the desktop wallpaper."
								emptyNote="Lewdware will not change the wallpaper."
								reveal={store.contentTarget?.tab === 'wallpaper' &&
									store.contentTarget.slot === 'wallpaper'}
								onrevealed={() => (store.contentTarget = null)}
							/>
							<MediaSlot
								slot={{ kind: 'splash' }}
								mediaId={store.behaviour!.content.splash}
								title="Splash"
								description="Shown once when a session starts. May be a video — an animated GIF is one."
								emptyNote="No startup image will be shown."
								reveal={store.contentTarget?.tab === 'wallpaper' &&
									store.contentTarget.slot === 'splash'}
								onrevealed={() => (store.contentTarget = null)}
							/>
						</div>
					{/if}
				</div>
			</div>
		</div>
	</BehaviourGate>
</div>
