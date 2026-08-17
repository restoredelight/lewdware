<script lang="ts">
	import Button from '$ui/Button.svelte';
	import InlineAudioPlayer from './InlineAudioPlayer.svelte';
	import { openStandalonePreview } from './mediaPreview.js';
	import { store } from './store.svelte.js';
	import type { MediaFile } from './types.js';

	type Kind = 'image' | 'visual' | 'audio';
	type Props = {
		kind: Kind;
		mediaId?: number;
		emptyNote: string;
		busy?: boolean;
		onselect: (id: number) => void | Promise<void>;
		onimport: () => void | Promise<void>;
		onclear?: () => void | Promise<void>;
	};

	let { kind, mediaId, emptyNote, busy = false, onselect, onimport, onclear }: Props = $props();
	let open = $state(false);
	let query = $state('');

	const file = $derived(
		mediaId == null ? null : (store.files.find((item) => item.id === mediaId) ?? null)
	);
	const candidates = $derived.by(() => {
		const needle = query.trim().toLocaleLowerCase();
		return store.files
			.filter((item) => {
				if (kind === 'audio') return item.file_info.type === 'audio';
				if (kind === 'image') return item.file_info.type === 'image';
				return item.file_info.type === 'image' || item.file_info.type === 'video';
			})
			.filter((item) => !needle || item.file_name.toLocaleLowerCase().includes(needle));
	});

	async function choose(id: number) {
		await onselect(id);
		open = false;
		query = '';
	}

	async function upload() {
		await onimport();
		open = false;
		query = '';
	}

	function duration(item: MediaFile) {
		return item.file_info.type === 'audio' ? item.file_info.duration : 0;
	}
</script>

<div class="picker">
	<div class:audio={kind === 'audio'} class="selection">
		{#if file}
			{#if file.file_info.type === 'audio'}
				<InlineAudioPlayer
					id={file.id}
					src={store.mediaUrl(`/file/${file.id}`, file.hash)}
					label={file.file_name}
					duration={duration(file)}
				/>
			{:else}
				<button
					class="preview"
					type="button"
					title="Preview"
					onclick={() => openStandalonePreview(file.id)}
				>
					<img
						src={store.mediaUrl(`/thumbnail/${file.id}`, file.hash)}
						alt={file.file_name}
						draggable="false"
					/>
				</button>
			{/if}
			<div class="identity"><span>{file.file_name}</span><small>{file.file_info.type}</small></div>
		{:else}
			{#if kind !== 'audio'}<div class="empty" aria-hidden="true">Empty</div>{/if}
			<p>{mediaId != null ? 'That file isn’t in this pack any more.' : emptyNote}</p>
		{/if}
		<div class="actions">
			<Button size="compact" disabled={busy} onclick={() => (open = !open)}>
				{mediaId == null ? 'Choose from pack…' : 'Replace from pack…'}
			</Button>
			<Button size="compact" disabled={busy} onclick={upload}>Upload…</Button>
			{#if mediaId != null && onclear}
				<Button variant="destructive" size="compact" disabled={busy} onclick={onclear}
					>Remove</Button
				>
			{/if}
		</div>
	</div>

	{#if open}
		<div class="browser">
			<div class="browser-head">
				<input
					bind:value={query}
					aria-label="Search pack media"
					placeholder={kind === 'audio' ? 'Search audio…' : 'Search media…'}
				/>
			</div>
			<div class:audio={kind === 'audio'} class="results">
				{#each candidates as item (item.id)}
					{#if item.file_info.type === 'audio'}
						<div class:selected={item.id === mediaId} class="audio-result">
							<span class="audio-name" title={item.file_name}>{item.file_name}</span>
							<InlineAudioPlayer
								id={item.id}
								src={store.mediaUrl(`/file/${item.id}`, item.hash)}
								label={item.file_name}
								duration={duration(item)}
							/>
							<Button size="compact" disabled={busy} onclick={() => choose(item.id)}>Choose</Button>
						</div>
					{:else}
						<button
							class:selected={item.id === mediaId}
							type="button"
							onclick={() => choose(item.id)}
						>
							<img
								src={store.mediaUrl(`/thumbnail/${item.id}`, item.hash)}
								alt=""
								draggable="false"
							/>
							<span>{item.file_name}</span>
						</button>
					{/if}
				{:else}
					<p class="no-results">No matching media in this pack.</p>
				{/each}
			</div>
		</div>
	{/if}
</div>

<style>
	.picker {
		display: flex;
		min-width: 0;
		flex-direction: column;
		gap: 8px;
	}
	.selection {
		display: flex;
		min-width: 0;
		align-items: center;
		gap: 12px;
		padding: 8px;
		border: 1px solid var(--ui-border);
		border-radius: var(--ui-radius-sm);
		background: var(--ui-surface);
	}
	.selection.audio {
		min-height: 50px;
	}
	.selection.audio > :global(.player) {
		max-width: 430px;
	}
	.preview,
	.empty {
		display: grid;
		width: 64px;
		height: 64px;
		flex: none;
		padding: 0;
		place-items: center;
		overflow: hidden;
		border: 1px solid var(--ui-border);
		border-radius: var(--ui-radius-sm);
		background: var(--ui-bg);
		color: var(--ui-muted);
		font-size: 11px;
	}
	.preview {
		cursor: pointer;
	}
	.preview:hover {
		border-color: var(--ui-border-strong);
	}
	.preview img {
		max-width: 100%;
		max-height: 100%;
		object-fit: contain;
	}
	.identity {
		display: flex;
		min-width: 0;
		flex-direction: column;
		gap: 2px;
	}
	.identity span {
		overflow: hidden;
		color: var(--ui-text);
		font-size: 13px;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.identity small,
	.selection p {
		margin: 0;
		color: var(--ui-muted);
		font-size: 11px;
	}
	.actions {
		display: flex;
		margin-left: auto;
		flex: none;
		gap: 8px;
	}
	.browser {
		border: 1px solid var(--ui-border-strong);
		border-radius: var(--ui-radius-sm);
		background: var(--ui-bg);
	}
	.browser-head {
		display: flex;
		gap: 8px;
		padding: 8px;
		border-bottom: 1px solid var(--ui-border);
	}
	.browser-head input {
		min-width: 0;
		height: 32px;
		flex: 1;
		padding: 0 9px;
		border: 1px solid var(--ui-border);
		border-radius: var(--ui-radius-sm);
		background: var(--ui-surface);
		color: var(--ui-text);
	}
	.results {
		display: grid;
		max-height: 260px;
		padding: 8px;
		grid-template-columns: repeat(auto-fill, minmax(112px, 1fr));
		gap: 7px;
		overflow-y: auto;
	}
	.results.audio {
		display: flex;
		flex-direction: column;
	}
	.results > button,
	.audio-result {
		display: flex;
		min-width: 0;
		padding: 7px;
		align-items: center;
		gap: 8px;
		border: 1px solid var(--ui-border);
		border-radius: var(--ui-radius-sm);
		background: var(--ui-surface);
		color: var(--ui-text);
		text-align: left;
	}
	.results > button {
		cursor: pointer;
	}
	.results:not(.audio) button {
		flex-direction: column;
		align-items: stretch;
	}
	.results > button:hover {
		border-color: var(--ui-border-strong);
	}
	.results > button.selected,
	.audio-result.selected {
		box-shadow: inset 2px 0 0 var(--ui-accent-hover);
	}
	.results img {
		width: 100%;
		height: 72px;
		object-fit: contain;
		background: var(--ui-bg);
	}
	.results span {
		overflow: hidden;
		font-size: 11px;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.audio-name {
		width: 140px;
		flex: none;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.audio-result :global(.player) {
		min-width: 180px;
	}
	.no-results {
		margin: 8px;
		color: var(--ui-muted);
		font-size: 12px;
	}
	@media (max-width: 560px) {
		.selection {
			flex-wrap: wrap;
		}
		.actions {
			width: 100%;
			margin-left: 0;
			justify-content: flex-end;
		}
	}
</style>
