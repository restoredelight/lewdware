<script lang="ts">
	// The artist namespace. The table, its search and its three edits are `VocabularyPage`, shared
	// with the Tags tab; an artist is only ever on media, so what is left here is the media half.
	import { onMount } from 'svelte';
	import { api } from './api.js';
	import { history } from './history.svelte.js';
	import { NO_MEDIA_SCOPE_COUNTS, store } from './store.svelte.js';
	import type { ArtistSummary } from './types.js';
	import VocabularyPage from './VocabularyPage.svelte';

	let summaries = $state<ArtistSummary[]>([]);
	let loaded = $state(false);
	let loadError = $state<string | null>(null);

	// The counts come from `store.files` rather than from the summaries beside them, so that
	// attribution added in the inspector shows up here without a round trip.
	const rows = $derived(
		summaries
			.map((row) => ({
				name: row.name,
				media: store.mediaCountsByArtist.get(row.name) ?? NO_MEDIA_SCOPE_COUNTS
			}))
			.sort((a, b) => a.name.localeCompare(b.name))
	);

	async function load() {
		loaded = false;
		loadError = null;
		try {
			summaries = await api.getArtistSummaries();
		} catch (cause) {
			loadError = String(cause);
		} finally {
			loaded = true;
		}
	}

	onMount(() => {
		void load();
	});

	/**
	 * Follows a rename or delete through the copies held in the store.
	 *
	 * Unlike tags, the backend returns nothing here: artists live only on media, so the local copy
	 * is rewritten in place and `allArtists` rebuilt from what is left.
	 */
	function updateLocal(from: string, to: string | null) {
		store.files = store.files.map((file) => ({
			...file,
			artists: [
				...new Set(
					file.artists.flatMap((artist) => (artist === from ? (to ? [to] : []) : [artist]))
				)
			]
		}));
		store.allArtists = [...new Set(store.files.flatMap((file) => file.artists))];
	}

	async function edit(request: Promise<void>, label: string, from: string, to: string | null) {
		await request;
		updateLocal(from, to);
		history.record({ label });
		summaries = await api.getArtistSummaries();
	}
</script>

<VocabularyPage
	title="Artists"
	noun="artist"
	description="Manage attribution recorded across media."
	columns={[
		{ label: 'Media', value: (row) => row.media['all-media'], width: '70px', narrowWidth: '48px' }
	]}
	{rows}
	{loaded}
	{loadError}
	onload={load}
	emptyDescription="Artists are created when you tag media with attribution in the inspector."
	renameNote="Every media item will be updated."
	mergeNote="References will be combined and duplicates removed."
	deleteDescription={(row) =>
		`This removes the artist from ${row.media['all-media']} media item(s). No media files will be deleted.`}
	onrename={(from, to) => edit(api.renameArtist(from, to), `Rename artist “${from}”`, from, to)}
	onmerge={(from, to) => edit(api.mergeArtist(from, to), `Merge artist “${from}”`, from, to)}
	ondelete={(artist) => edit(api.deleteArtist(artist), `Delete artist “${artist}”`, artist, null)}
/>
