<script lang="ts">
	// The tag namespace. Everything about *being a searchable table of names* lives in
	// `VocabularyPage`, which the Artists tab shares; what is here is the one thing tags do that
	// artists do not — they are also referenced by the behaviour document.
	//
	// This used to merge three sources in the browser: the media list, a backend summary, and the
	// tags named only by the behaviour document (typed into a caption, naming a content group). All
	// three had to be there or the table would be missing rows the Content tab can see. It is one
	// query now, because that is a question SQL can answer directly.
	import { api } from './api.js';
	import { history } from './history.svelte.js';
	import { invalidate, keys, query } from './query.svelte.js';
	import { NO_MEDIA_SCOPE_COUNTS, store } from './store.svelte.js';
	import VocabularyPage from './VocabularyPage.svelte';

	const tagRows = query(keys.tags, api.getTagRows);

	const rows = $derived.by(() => {
		// The per-scope media counts (Popups / Audio / All media) still come from the file list the
		// grid already holds — the backend counts every live file, which is the "all media" column.
		const counts = store.mediaCountsByTag;
		return (tagRows.current ?? [])
			.map((row) => ({
				name: row.name,
				media: counts.get(row.name) ?? {
					...NO_MEDIA_SCOPE_COUNTS,
					'all-media': row.media_count
				},
				content: row.content_uses,
				experience: row.experience_uses,
				total: row.content_uses + row.experience_uses
			}))
			.sort((a, b) => a.name.localeCompare(b.name));
	});

	async function edit(request: Promise<void>, label: string, from: string, to: string | null) {
		await request;
		store.retagEverywhere(from, to, true);
		// A tag rename reaches captions, groups, links and stage selections alike — every one of
		// them is a join to this row, so every surface showing tags is now out of date.
		invalidate(keys.behaviour);
		invalidate(keys.tags);
		history.record({ label });
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
	loaded={tagRows.current !== undefined}
	loadError={tagRows.error}
	onload={() => tagRows.reload()}
	emptyDescription="Tags are created when you tag media or use them in Content and Experience settings."
	renameNote="Every media and behaviour reference will be updated."
	mergeNote="References will be combined and duplicates removed."
	deleteDescription={(row) =>
		`This removes the tag from ${row.media['all-media']} media item(s) and ${row.total} Content/Experience reference(s). No media files will be deleted.`}
	onrename={(from, to) => edit(api.renameTag(from, to), `Rename tag “${from}”`, from, to)}
	onmerge={(from, to) => edit(api.mergeTag(from, to), `Merge tag “${from}”`, from, to)}
	ondelete={(tag) => edit(api.deleteTag(tag), `Delete tag “${tag}”`, tag, null)}
/>
