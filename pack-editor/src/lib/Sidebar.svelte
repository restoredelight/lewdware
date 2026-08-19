<script lang="ts">
	import { ClipboardDocument, Eye, Icon, MusicalNote, Plus, Trash, XMark } from 'svelte-hero-icons';
	import { store } from './store.svelte.js';
	import type { FileInfo } from './types.js';
	import TagInput from '$ui/TagInput.svelte';
	import { NON_POPUP_TAG, SUBLIMINAL_TAG } from './tags.js';
	import Button from '$ui/Button.svelte';
	import { api } from './api.js';
	import { onMount } from 'svelte';
	import { history } from './history.svelte.js';
	import EmptyState from '$ui/EmptyState.svelte';
	import IconButton from '$ui/IconButton.svelte';
	import { copyFileName } from './clipboard.js';
	import { openMediaPreview, openSelectionEditor } from './mediaPreview.js';
	import RadioGroup from '$ui/RadioGroup.svelte';
	import { audioRole, setAudioRole, type AudioRole } from './audioRoles.js';
	import { summarizeLabels, type LabelSummary } from './labelSummary.js';
	import { taskFeedback } from './taskFeedback.svelte.js';
	import { formatDuration, formatFileSize } from './format.js';
	import { clampScroll } from '$ui/scroll';

	function infoRows(info: FileInfo, size: number): { label: string; value: string }[] {
		const rows =
			info.type === 'image'
				? [
						{ label: 'Type', value: info.transparent ? 'Image (transparent)' : 'Image' },
						{ label: 'Dimensions', value: `${info.width} × ${info.height}` }
					]
				: info.type === 'video'
					? [
							{ label: 'Type', value: info.transparent ? 'Video (transparent)' : 'Video' },
							{ label: 'Dimensions', value: `${info.width} × ${info.height}` },
							{ label: 'Duration', value: formatDuration(info.duration) },
							{ label: 'Audio', value: info.audio ? 'Yes' : 'No' }
						]
					: [
							{ label: 'Type', value: 'Audio' },
							{ label: 'Duration', value: formatDuration(info.duration) }
						];
		rows.push({ label: 'File size', value: formatFileSize(size) });
		return rows;
	}

	const selCount = $derived(store.mediaTab.selectedIds.size);
	const primary = $derived(store.primaryFile);
	const selected = $derived(store.selectedFiles);
	// Only in the tab that owns the role. All media is an inventory surface -- it lists every file
	// the pack has, for a custom mode's sake as much as anyone's, and a control that only the
	// built-in modes read has no business being offered there. What it does say about the role is
	// "Used as", which reports rather than sets, and links to the tab that does the setting.
	const selectedAudio = $derived(
		store.activeView === 'audio' &&
			selected.length > 0 &&
			selected.every((file) => file.file_info.type === 'audio')
	);
	// Popup attributes are *not* here. They live in the overlay, next to the picture that is the
	// only thing able to say whether a size or a placement is right -- see `MediaViewer`. What the
	// inspector keeps is the way in, since the overlay is a one-file surface and this is the
	// surface that knows about selections.
	const selectedPopups = $derived(
		store.activeView === 'popups' &&
			selected.length > 0 &&
			selected.every((file) => file.file_info.type !== 'audio')
	);
	const selectedAudioRole = $derived.by((): AudioRole | null => {
		if (!selectedAudio) return null;
		const roles = new Set(selected.map((file) => audioRole(file.tags)));
		return roles.size === 1 ? ([...roles][0] ?? null) : null;
	});
	const showPreview = $derived(
		!(store.activeView === 'audio' && primary?.file_info.type === 'audio')
	);
	const usedAs = $derived.by(() => {
		if (!primary || selCount !== 1) return [];
		const uses: {
			label: string;
			target:
				| { kind: 'media'; view: 'popups' | 'audio' }
				| { kind: 'content'; tab: 'subliminals'; fileId: number }
				| { kind: 'content'; tab: 'wallpaper'; slot: 'wallpaper' | 'splash' }
				| { kind: 'experience'; stageId: string };
		}[] = [];
		if (primary.file_info.type === 'audio')
			uses.push({
				label: `${audioRole(primary.tags) === 'popup' ? 'Popup' : 'Background'} audio`,
				target: { kind: 'media', view: 'audio' }
			});
		else if (!primary.tags.includes(NON_POPUP_TAG))
			uses.push({ label: 'Popup', target: { kind: 'media', view: 'popups' } });
		if (primary.tags.includes(SUBLIMINAL_TAG))
			uses.push({
				label: 'Subliminal',
				target: { kind: 'content', tab: 'subliminals', fileId: primary.id }
			});
		if (store.behaviour?.content.wallpaper === primary.id)
			uses.push({
				label: 'Wallpaper',
				target: { kind: 'content', tab: 'wallpaper', slot: 'wallpaper' }
			});
		if (store.behaviour?.content.splash === primary.id)
			uses.push({
				label: 'Splash',
				target: { kind: 'content', tab: 'wallpaper', slot: 'splash' }
			});
		for (const stage of store.behaviour?.experience?.timeline.stages ?? [])
			if (stage.content.wallpaper === primary.id)
				uses.push({
					label: `Wallpaper for “${stage.label}”`,
					target: { kind: 'experience', stageId: stage.id }
				});
		return uses;
	});
	const showUsedAs = $derived(store.activeView === 'all-media' && usedAs.length > 0);
	const tagSummary = $derived(summarizeLabels(selected, 'tags'));
	const artistSummary = $derived(summarizeLabels(selected, 'artists'));
	let titleValue = $state('');
	let titleError = $state<string | null>(null);
	let sourceValue = $state('');
	let inspectorBody = $state<HTMLDivElement>();
	let inspectorWidth = $state(256);
	let resizing = $state(false);
	let audioRoleBusy = $state(false);

	const MIN_WIDTH = 220;
	const MAX_WIDTH = 420;
	const DEFAULT_WIDTH = 256;

	onMount(() => {
		const saved = Number(localStorage.getItem('pack-editor:inspector-width'));
		if (Number.isFinite(saved)) inspectorWidth = Math.min(MAX_WIDTH, Math.max(MIN_WIDTH, saved));
	});

	function setInspectorWidth(width: number) {
		inspectorWidth = Math.min(MAX_WIDTH, Math.max(MIN_WIDTH, Math.round(width)));
		localStorage.setItem('pack-editor:inspector-width', String(inspectorWidth));
	}

	function startResize(event: PointerEvent) {
		resizing = true;
		(event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
	}

	function resize(event: PointerEvent) {
		if (resizing) setInspectorWidth(window.innerWidth - event.clientX);
	}

	function stopResize(event: PointerEvent) {
		resizing = false;
		const handle = event.currentTarget as HTMLElement;
		if (handle.hasPointerCapture(event.pointerId)) handle.releasePointerCapture(event.pointerId);
	}

	function resizeWithKeyboard(event: KeyboardEvent) {
		if (event.key === 'ArrowLeft' || event.key === 'ArrowRight') {
			event.preventDefault();
			const amount = event.shiftKey ? 32 : 8;
			setInspectorWidth(inspectorWidth + (event.key === 'ArrowLeft' ? amount : -amount));
		} else if (event.key === 'Home') {
			event.preventDefault();
			setInspectorWidth(MIN_WIDTH);
		} else if (event.key === 'End') {
			event.preventDefault();
			setInspectorWidth(MAX_WIDTH);
		}
	}

	$effect(() => {
		titleValue = primary?.file_name ?? '';
		titleError = null;
	});
	$effect(() => {
		sourceValue = primary?.source_url ?? '';
	});
	$effect(() => {
		// The browser otherwise tries to preserve an anchor when the single-item fields are
		// replaced by the multi-selection summary, which can leave the new Tags section scrolled
		// underneath the fixed preview.
		selected.map((file) => file.id).join(',');
		queueMicrotask(() => {
			if (inspectorBody) inspectorBody.scrollTop = 0;
		});
	});

	/**
	 * Puts a tag or an artist on the whole selection, or takes it off.
	 *
	 * The undo entry counts only the files the edit actually changed, not the files it was sent
	 * for: "Add tag “spiral” to 3 items" over a selection of twenty already carrying it would
	 * describe work that did not happen.
	 */
	async function editLabel(field: 'tags' | 'artists', value: string, add: boolean) {
		const ids = selected.map((file) => file.id);
		const affected = selected.filter((file) => file[field].includes(value) !== add).length;
		if (!affected) return;

		if (field === 'tags') {
			if (add) {
				await api.addTagToFiles(ids, value);
				store.addTagToFiles(ids, value, true);
			} else {
				await api.removeTagFromFiles(ids, value);
				store.removeTagFromFiles(ids, value, true);
			}
		} else if (add) {
			await api.addArtistToFiles(ids, value);
			store.addArtistToFiles(ids, value, true);
		} else {
			await api.removeArtistFromFiles(ids, value);
			store.removeArtistFromFiles(ids, value, true);
		}

		const verb = add ? 'Add' : 'Remove';
		const noun = field === 'tags' ? 'tag' : 'artist';
		history.record({
			label:
				affected === 1
					? `${verb} ${noun} “${value}”`
					: `${verb} ${noun} “${value}” ${add ? 'to' : 'from'} ${affected} items`
		});
	}

	async function saveSource() {
		if (!primary || selCount !== 1) return;
		const id = primary.id;
		const before = primary.source_url;
		const after = sourceValue.trim() || null;
		if (after === before) return;
		await api.setFileSourceUrl(id, after);
		store.updateFileSourceUrl(id, after, true);
		history.record({
			label: `Set source for “${primary.file_name}”`
		});
	}
	async function rename() {
		if (!primary || selCount !== 1 || !titleValue.trim() || titleValue === primary.file_name)
			return;
		titleError = null;
		const id = primary.id;
		const before = primary.file_name;
		const after = titleValue.trim();
		try {
			// The behaviour document is untouched: its media slots hold ids, so a rename cannot
			// move or break one.
			await api.setFileTitle(id, after);
			store.updateFileName(id, after, true);
			history.record({ label: `Rename “${before}”` });
		} catch (error) {
			titleError = String(error);
			titleValue = primary.file_name;
		}
	}
	function removeSelected() {
		const ids = selected.map((file) => file.id);
		if (ids.length) store.requestMediaRemoval(ids);
	}

	async function changeAudioRole(role: AudioRole) {
		audioRoleBusy = true;
		try {
			await setAudioRole(
				selected.map((file) => file.id),
				role
			);
		} catch (error) {
			taskFeedback.error('audio-role', `Could not change audio type: ${String(error)}`);
		} finally {
			audioRoleBusy = false;
		}
	}

	function navigateToUse(target: (typeof usedAs)[number]['target']) {
		if (target.kind === 'media') {
			if (primary) store.revealMedia(target.view, primary.id);
		} else if (target.kind === 'content') store.revealContent(target);
		else store.revealExperienceStage(target.stageId);
	}
</script>

<!-- Tags and artists are the same control over two different lists: the names the whole selection
     shares, editable as a set, and the names only part of it carries, offered as chips that add to
     all or remove from the selection. -->
{#snippet labelSection(
	title: string,
	noun: string,
	field: 'tags' | 'artists',
	summary: LabelSummary,
	suggestions: string[]
)}
	<section>
		<div class="section-heading">
			<h2>{title}</h2>
			<span>{summary.common.length} shared</span>
		</div>
		<TagInput
			tags={summary.common}
			{suggestions}
			label={selCount === 1 ? title : `${title} on all selected items`}
			placeholder={selCount === 1 ? `Add ${noun}…` : 'Add to all…'}
			onadd={(value) => editLabel(field, value, true)}
			onremove={(value) => editLabel(field, value, false)}
		/>
		{#if summary.mixed.length > 0}
			<p class="mixed-label">On some selected items</p>
			<div class="mixed-tags">
				{#each summary.mixed as item (item.name)}
					<span class="mixed-tag"
						><span>{item.name} <small>{item.count}/{selCount}</small></span><button
							onclick={() => editLabel(field, item.name, true)}
							aria-label={`Add ${item.name} to all selected items`}
							title="Add to all"><Icon src={Plus} mini size="13px" /></button
						><button
							onclick={() => editLabel(field, item.name, false)}
							aria-label={`Remove ${item.name} from selected items`}
							title="Remove from selection"><Icon src={XMark} mini size="13px" /></button
						></span
					>
				{/each}
			</div>
		{/if}
	</section>
{/snippet}

<aside
	class:resizing
	class="inspector bg-surface border-border flex shrink-0 flex-col border-l"
	style={`width: ${inspectorWidth}px`}
	aria-label="Media inspector"
>
	<!-- A focusable ARIA separator is the prescribed resize-handle pattern. -->
	<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
	<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
	<div
		class="resize-handle"
		role="separator"
		aria-label="Resize media inspector"
		aria-orientation="vertical"
		aria-valuemin={MIN_WIDTH}
		aria-valuemax={MAX_WIDTH}
		aria-valuenow={inspectorWidth}
		tabindex="0"
		onpointerdown={startResize}
		onpointermove={resize}
		onpointerup={stopResize}
		onpointercancel={stopResize}
		ondblclick={() => setInspectorWidth(DEFAULT_WIDTH)}
		onkeydown={resizeWithKeyboard}
	></div>
	{#if primary}
		{#if showPreview}
			<!-- Audio already has an inline player in its dedicated list. All media keeps this
			     preview because its grid tile has no playback control. -->
			<button
				class="preview bg-bg flex shrink-0 items-center justify-center"
				onclick={() => openMediaPreview(primary.id)}
				aria-label={`Preview ${primary.file_name}`}
			>
				{#if primary.file_info.type === 'audio'}
					<span class="text-muted h-12 w-12"><Icon src={MusicalNote} /></span>
				{:else}
					<img
						src={store.mediaUrl(
							`/${store.saveBlocksPreviews ? 'thumbnail' : 'preview'}/${primary.id}`,
							primary.hash
						)}
						alt={primary.file_name}
						draggable="false"
						class="h-full w-full object-contain"
					/>
				{/if}
				<span class="preview-hint"><Icon src={Eye} mini /> Preview</span>
			</button>
		{/if}

		<!-- Info -->
		<div class="inspector-body" bind:this={inspectorBody} use:clampScroll>
			{#if showUsedAs}
				<section>
					<div class="section-heading"><h2>Used as</h2></div>
					<div class="used-as">
						{#each usedAs as use, index (`${use.label}-${index}`)}
							<button type="button" onclick={() => navigateToUse(use.target)}>{use.label}</button>
						{/each}
					</div>
				</section>
			{/if}

			{#if selectedPopups}
				<!-- The way into the popup editor, not a copy of it. Everything a file does as a popup
				     is edited in the overlay, where the picture is; what this surface contributes is the
				     selection, which the overlay cannot see on its own. -->
				<section>
					<div class="section-heading">
						<h2>Popup</h2>
					</div>
					<Button size="compact" onclick={() => openSelectionEditor()}>
						{selCount === 1 ? 'Edit this popup…' : `Edit ${selCount} popups…`}
					</Button>
				</section>
			{/if}

			{#if selectedAudio}
				<section>
					<div class="section-heading">
						<h2>Audio type</h2>
						{#if selectedAudioRole === null}<span>Mixed</span>{/if}
					</div>
					<RadioGroup
						ariaLabel="Audio type"
						value={selectedAudioRole}
						disabled={audioRoleBusy}
						options={[
							{
								value: 'background',
								label: 'Background',
								description: 'Plays continuously during the session.'
							},
							{ value: 'popup', label: 'Popup', description: 'Plays when a popup appears.' }
						]}
						onchange={(value) => changeAudioRole(value as AudioRole)}
					/>
				</section>
			{/if}

			<section>
				<div class="section-heading">
					<h2>{selCount === 1 ? 'Media' : `${selCount} items selected`}</h2>
				</div>
				{#if selCount === 1}
					<div class="title-field">
						<label for={`media-title-${primary.id}`}>File name</label>
						<div class="title-control pb-1">
							<input
								id={`media-title-${primary.id}`}
								bind:value={titleValue}
								onblur={rename}
								onkeydown={(event) => {
									if (event.key === 'Enter') event.currentTarget.blur();
									if (event.key === 'Escape') {
										titleValue = primary.file_name;
										event.currentTarget.blur();
									}
								}}
							/>
							<IconButton
								label={`Copy file name “${primary.file_name}”`}
								onclick={() => copyFileName(primary.file_name)}
								><Icon src={ClipboardDocument} mini size="15px" /></IconButton
							>
						</div>
					</div>
					{#if titleError}<p class="field-error" role="alert">{titleError}</p>{/if}
					<div class="title-field">
						<label for={`media-source-${primary.id}`}>Source URL</label>
						<div class="title-control">
							<input
								id={`media-source-${primary.id}`}
								type="url"
								placeholder="https://…"
								bind:value={sourceValue}
								onblur={saveSource}
								onkeydown={(event) => {
									if (event.key === 'Enter') event.currentTarget.blur();
									if (event.key === 'Escape') {
										sourceValue = primary.source_url ?? '';
										event.currentTarget.blur();
									}
								}}
							/>
							{#if primary.source_url}
								<IconButton
									label={`Open source “${primary.source_url}”`}
									onclick={() => window.open(primary.source_url!, '_blank', 'noopener')}
									><Icon src={Eye} mini size="15px" /></IconButton
								>
							{/if}
						</div>
					</div>
				{/if}
			</section>

			{@render labelSection('Tags', 'tag', 'tags', tagSummary, store.allTags)}
			{@render labelSection('Artists', 'artist', 'artists', artistSummary, store.allArtists)}

			{#if selCount === 1}
				<details open>
					<summary>Details</summary>
					<table>
						<tbody
							>{#each infoRows(primary.file_info, primary.size) as row}<tr
									><th>{row.label}</th><td>{row.value}</td></tr
								>{/each}</tbody
						>
					</table>
				</details>
			{:else}
				<div class="selection-summary">
					<span>{formatFileSize(selected.reduce((total, file) => total + file.size, 0))}</span><span
						>{new Set(selected.map((file) => file.file_info.type)).size === 1
							? `${primary.file_info.type} files`
							: 'Mixed media types'}</span
					>
				</div>
			{/if}

			<section class="actions">
				<Button size="compact" variant="destructive" onclick={removeSelected}
					><Icon src={Trash} mini size="15px" /> Remove {selCount === 1
						? 'item'
						: `${selCount} items`}</Button
				>
			</section>
		</div>
	{:else if selCount > 1}
		<div class="text-muted flex h-full flex-col items-center justify-center gap-1">
			<span class="text-2xl font-semibold">{selCount}</span>
			<span class="text-xs">items selected</span>
		</div>
	{:else}
		<div class="flex h-full items-center justify-center p-3">
			<EmptyState
				title="Nothing selected"
				description="Select an item to inspect it. Use Shift-click for a range or Ctrl/⌘-click to select several items and edit their tags together."
			/>
		</div>
	{/if}
</aside>

<style>
	.inspector {
		position: relative;
		min-width: 0;
		overflow: hidden;
	}
	.resize-handle {
		position: absolute;
		z-index: 20;
		top: 0;
		bottom: 0;
		left: -3px;
		width: 7px;
		padding: 0;
		border: 0;
		background: transparent;
		touch-action: none;
		cursor: col-resize;
	}
	.resize-handle::after {
		content: '';
		position: absolute;
		top: 0;
		bottom: 0;
		left: 3px;
		width: 1px;
		background: transparent;
		transition:
			width 100ms,
			left 100ms,
			background 100ms;
	}
	.resize-handle:hover::after,
	.resize-handle:focus-visible::after,
	.resizing .resize-handle::after {
		left: 2px;
		width: 2px;
		background: var(--ui-accent);
	}
	.resize-handle:focus-visible {
		outline-offset: -3px;
	}
	.resizing {
		user-select: none;
	}
	.preview {
		position: relative;
		width: 100%;
		/* Grows with the inspector: widening it is the way to ask for a bigger picture. Capped
		   against the window so a short one still leaves room for the fields below. */
		aspect-ratio: 16 / 10;
		max-height: 38vh;
		padding: 0;
		border: 0;
		color: var(--ui-muted);
		cursor: pointer;
	}
	.preview:focus-visible {
		outline-offset: -2px;
	}
	.preview-hint {
		position: absolute;
		right: 7px;
		bottom: 7px;
		display: flex;
		padding: 4px 6px;
		align-items: center;
		gap: 4px;
		border-radius: 4px;
		background: rgb(0 0 0 / 0.68);
		color: white;
		font-size: 10px;
		opacity: 0;
		transition: opacity 120ms;
	}
	.preview-hint :global(svg) {
		width: 13px;
		height: 13px;
	}
	.preview:hover .preview-hint,
	.preview:focus-visible .preview-hint {
		opacity: 1;
	}
	.inspector-body {
		display: flex;
		min-height: 0;
		padding: 12px;
		overflow-y: auto;
		overflow-anchor: none;
		flex-direction: column;
		gap: 16px;
	}
	section {
		min-width: 0;
	}
	.section-heading {
		display: flex;
		margin-bottom: 7px;
		align-items: baseline;
		justify-content: space-between;
		gap: 8px;
	}
	h2 {
		margin: 0;
		color: var(--ui-text);
		font-size: 12px;
		font-weight: 700;
	}
	.section-heading > span,
	.mixed-label {
		color: var(--ui-muted);
		font-size: 10px;
	}
	.title-field {
		display: flex;
		flex-direction: column;
		gap: 5px;
		color: var(--ui-muted);
		font-size: 11px;
	}
	.title-control {
		display: flex;
		min-width: 0;
		align-items: center;
		gap: 4px;
	}
	.title-field input {
		width: 100%;
		min-width: 0;
		height: 32px;
		padding: 0 8px;
		border: 1px solid var(--ui-border);
		border-radius: var(--ui-radius-sm);
		background: var(--ui-bg);
		color: var(--ui-text);
		font: inherit;
		font-size: 12px;
	}
	.field-error {
		margin: 5px 0 0;
		color: var(--ui-danger);
		font-size: 10px;
		line-height: 1.35;
	}
	.mixed-label {
		margin: 10px 0 5px;
	}
	.mixed-tags {
		display: flex;
		flex-wrap: wrap;
		gap: 5px;
	}
	.mixed-tag {
		display: inline-flex;
		min-height: 26px;
		align-items: center;
		padding-left: 8px;
		border: 1px dashed var(--ui-border-strong);
		border-radius: 999px;
		background: var(--ui-bg);
		color: var(--ui-text);
		font-size: 11px;
	}
	.mixed-tag small {
		color: var(--ui-muted);
	}
	.mixed-tag button {
		display: grid;
		width: 23px;
		height: 23px;
		padding: 0;
		place-items: center;
		border: 0;
		border-radius: 50%;
		background: transparent;
		color: var(--ui-muted);
		cursor: pointer;
	}
	.mixed-tag button:hover {
		background: var(--ui-surface-raised);
		color: var(--ui-text);
	}
	.mixed-tag button:focus-visible {
		outline-offset: -2px;
	}
	details {
		border-top: 1px solid var(--ui-border);
		padding-top: 10px;
	}
	summary {
		color: var(--ui-text);
		font-size: 12px;
		font-weight: 700;
		cursor: pointer;
	}
	table {
		width: 100%;
		margin-top: 7px;
		border-collapse: collapse;
		font-size: 11px;
	}
	th {
		padding: 2px 8px 2px 0;
		color: var(--ui-muted);
		font-weight: 400;
		text-align: left;
		white-space: nowrap;
	}
	td {
		color: var(--ui-text);
		text-align: right;
	}
	.selection-summary {
		display: flex;
		padding-top: 10px;
		justify-content: space-between;
		border-top: 1px solid var(--ui-border);
		color: var(--ui-muted);
		font-size: 11px;
	}
	.used-as {
		display: flex;
		flex-wrap: wrap;
		gap: 5px;
	}
	.used-as button {
		margin: 0;
		padding: 3px 6px;
		border: 1px solid var(--ui-border);
		border-radius: var(--ui-radius-sm);
		background: var(--ui-bg);
		color: var(--ui-text);
		font-size: 11px;
		line-height: 1.45;
		cursor: pointer;
	}
	.used-as button:hover {
		border-color: var(--ui-border-strong);
	}
	.used-as button:focus-visible {
		outline-offset: -2px;
	}
	.actions {
		padding-top: 12px;
		border-top: 1px solid var(--ui-border);
	}
	@media (max-width: 1000px) and (min-width: 521px) {
		.inspector {
			width: 220px !important;
		}
	}
	@media (max-width: 520px) {
		.inspector {
			width: 100% !important;
			height: min(42%, 300px);
			border-top: 1px solid var(--ui-border);
			border-left: 0;
		}
		.resize-handle {
			display: none;
		}
		.preview {
			display: none;
		}
	}
</style>
