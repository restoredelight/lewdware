<script lang="ts">
	// The tag namespace. Everything about *being a searchable table of names* lives in
	// `VocabularyPage`, which the Artists tab shares; what is here is the two things tags do that
	// artists do not — they are also referenced by the behaviour document, and every edit to one
	// comes back from the backend as a new document.
	import { onMount } from 'svelte';
	import { api } from './api.js';
	import { adoptBehaviour, ensureBehaviour } from './behaviourSave.svelte.js';
	import { NO_MEDIA_SCOPE_COUNTS, store } from './store.svelte.js';
	import { behaviourTags, tagUsage } from './tagReferences.js';
	import type { Behaviour, TagSummary } from './types.js';
	import VocabularyPage from './VocabularyPage.svelte';

	let summaries = $state<TagSummary[]>([]);
	let loaded = $state(false);
	let loadError = $state<string | null>(null);

	// A tag can be in the pack three ways at once: on media, in the backend's own summary, or named
	// only by the behaviour document (typed into a caption, naming a content group). All three, or
	// the table would be missing rows the Content tab can see.
	const rows = $derived.by(() => {
		const behaviour = store.behaviour;
		if (!behaviour) return [];
		const counts = store.mediaCountsByTag;
		const names = new Set([
			...store.allTags,
			...summaries.map((item) => item.name),
			...behaviourTags(behaviour)
		]);
		return [...names]
			.map((name) => ({
				name,
				media: counts.get(name) ?? NO_MEDIA_SCOPE_COUNTS,
				...tagUsage(behaviour, name)
			}))
			.sort((a, b) => a.name.localeCompare(b.name));
	});

	// The behaviour can also go away *after* a successful load — an undo or a discard replaces it —
	// and a table of tags with no document to count them against has nothing to show.
	const failure = $derived(
		loadError ?? (store.behaviour ? null : 'The pack behaviour could not be loaded.')
	);

	async function load() {
		loaded = false;
		loadError = null;
		try {
			if (!(await ensureBehaviour())) throw new Error('The pack behaviour could not be loaded.');
			summaries = await api.getTagSummaries();
		} catch (cause) {
			loadError = String(cause);
		} finally {
			loaded = true;
		}
	}

	onMount(() => {
		void load();
	});

	/** All three edits rewrite the document server-side, so all three adopt what comes back. */
	async function edit(request: Promise<Behaviour>, label: string, from: string, to: string | null) {
		adoptBehaviour(await request, { label });
		store.retagEverywhere(from, to, true);
		summaries = await api.getTagSummaries();
	}
</script>

<VocabularyPage
	title="Tags"
	noun="tag"
	description="Manage the vocabulary used across media, Content, and Experience."
	columns={[
		{ label: 'Media', value: (row) => row.media['all-media'], width: '70px', narrowWidth: '48px' },
		{ label: 'Content', value: (row) => row.content, width: '80px', narrowWidth: '58px' },
		{ label: 'Experience', value: (row) => row.experience, width: '90px', narrowWidth: '68px' }
	]}
	{rows}
	{loaded}
	loadError={failure}
	onload={load}
	emptyDescription="Tags are created when you tag media or use them in Content and Experience settings."
	renameNote="Every media and behaviour reference will be updated."
	mergeNote="References will be combined and duplicates removed."
	deleteDescription={(row) =>
		`This removes the tag from ${row.media['all-media']} media item(s) and ${row.total} Content/Experience reference(s). No media files will be deleted.`}
	onrename={(from, to) => edit(api.renameTag(from, to), `Rename tag “${from}”`, from, to)}
	onmerge={(from, to) => edit(api.mergeTag(from, to), `Merge tag “${from}”`, from, to)}
	ondelete={(tag) => edit(api.deleteTag(tag), `Delete tag “${tag}”`, tag, null)}
/>
