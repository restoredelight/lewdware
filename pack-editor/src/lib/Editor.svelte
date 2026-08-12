<script lang="ts">
	import { clampScroll } from '$ui/scroll';
	import Button from '$ui/Button.svelte';
	import Tabs from '$ui/Tabs.svelte';
	import IconButton from '$ui/IconButton.svelte';
	import Popover from '$ui/Popover.svelte';
	import Dialog from '$ui/Dialog.svelte';
	import {
		ArrowUturnLeft,
		ArrowUturnRight,
		ChevronLeft,
		ChevronRight,
		Clock,
		CodeBracketSquare,
		Cog6Tooth,
		DocumentText,
		EllipsisVertical,
		Icon,
		PaintBrush,
		Squares2x2,
		Tag
	} from 'svelte-hero-icons';
	import { onMount, tick } from 'svelte';
	import { getCurrentWebview } from '@tauri-apps/api/webview';
	import { api } from './api.js';
	import { store } from './store.svelte.js';
	import MediaGrid from './MediaGrid.svelte';
	import Sidebar from './Sidebar.svelte';
	import Options from './Options.svelte';
	import Content from './Content.svelte';
	import Experience from './Experience.svelte';
	import Modes from './Modes.svelte';
	import UploadProgress from './UploadProgress.svelte';
	import MediaViewer from './MediaViewer.svelte';
	import ImportWarnings from './ImportWarnings.svelte';
	import MediaToolbar from './MediaToolbar.svelte';
	import Tags from './Tags.svelte';
	import Artists from './Artists.svelte';
	import {
		cancelBehaviourSave,
		flushBehaviourSave,
		initializeBehaviourHistory
	} from './behaviourSave.svelte.js';
	import {
		cancelMetadataSave,
		flushMetadataSave,
		initializeMetadataHistory,
		scheduleMetadataSave
	} from './metadataSave.svelte.js';
	import { history } from './history.svelte.js';
	import TaskStatus from './TaskStatus.svelte';
	import { taskFeedback } from './taskFeedback.svelte.js';
	import type { MediaFile } from './types.js';
	import EmptyState from '$ui/EmptyState.svelte';

	let saveError = $state<string | null>(null);
	let navCollapsed = $state(false);
	let narrowWindow = $state(false);
	let showClosePackDialog = $state(false);
	let closePackAfterSave = $state(false);
	let removingMedia = $state(false);
	let saveDestinationChosen = $state(false);
	let modifierLabel = $state('Ctrl');
	let packTitle = $state(store.packName);
	let packTitleInput = $state<HTMLInputElement>();

	const navigationTabs = [
		{ id: 'media', label: 'Media', icon: Squares2x2 },
		{ id: 'tags', label: 'Tags', icon: Tag },
		{ id: 'artists', label: 'Artists', icon: PaintBrush },
		{ id: 'content', label: 'Content', icon: DocumentText },
		{ id: 'experience', label: 'Timeline', icon: Clock },
		{ id: 'modes', label: 'Modes', icon: CodeBracketSquare },
		{ id: 'options', label: 'Pack Metadata', icon: Cog6Tooth }
	];
	const navigationCollapsed = $derived(navCollapsed || narrowWindow);

	$effect(() => {
		const name = store.packName;
		if (packTitleInput !== document.activeElement) packTitle = name;
	});

	$effect(() => {
		if (!closePackAfterSave || store.saveActive) return;
		closePackAfterSave = false;
		if (store.packSaved) void finishClosePack();
		else showClosePackDialog = true;
	});

	$effect(() => {
		if (!store.saveActive) saveDestinationChosen = false;
	});

	function onSaveDestinationChosen() {
		saveDestinationChosen = true;
		taskFeedback.progress('save', 'Saving pack…');
	}

	onMount(() => {
		modifierLabel = navigator.platform.includes('Mac') ? '⌘' : 'Ctrl';
		navCollapsed = localStorage.getItem('pack-editor:navigation-collapsed') === 'true';
		const narrowQuery = window.matchMedia('(max-width: 760px)');
		const updateNarrowWindow = () => (narrowWindow = narrowQuery.matches);
		updateNarrowWindow();
		narrowQuery.addEventListener('change', updateNarrowWindow);
		const handleShortcut = (event: KeyboardEvent) => {
			if (event.defaultPrevented || !(event.ctrlKey || event.metaKey) || event.altKey) return;
			if (showClosePackDialog || store.pendingMediaRemoval.length > 0 || store.openedId !== null)
				return;
			const key = event.key.toLowerCase();
			if (key === 's') {
				event.preventDefault();
				if (event.shiftKey) void saveAs();
				else if (!store.packSaved) void save();
			} else if (key === 'z') {
				event.preventDefault();
				void (event.shiftKey ? redo() : undo());
			} else if (key === 'y' && !event.shiftKey) {
				event.preventDefault();
				void redo();
			}
		};
		window.addEventListener('keydown', handleShortcut);
		const unlisten = getCurrentWebview().onDragDropEvent((e) => {
			if (e.payload.type === 'enter' || e.payload.type === 'over') {
				store.dragActive = true;
			} else if (e.payload.type === 'leave') {
				store.dragActive = false;
			} else if (e.payload.type === 'drop') {
				store.dragActive = false;
				api.addPaths(e.payload.paths);
			}
		});
		return () => {
			unlisten.then((fn) => fn());
			window.removeEventListener('keydown', handleShortcut);
			narrowQuery.removeEventListener('change', updateNarrowWindow);
			store.dragActive = false;
		};
	});

	onMount(async () => {
		try {
			const metadata = await api.getPackMetadata();
			store.metadata = metadata;
			store.packName = metadata.name;
			packTitle = metadata.name;
			initializeMetadataHistory(metadata);
		} catch (error) {
			saveError = `Could not load pack metadata: ${String(error)}`;
			taskFeedback.error('metadata-load', saveError);
		}
	});

	function editPackTitle(value: string) {
		packTitle = value;
		const name = value.trim();
		if (!name || !store.metadata) return;
		store.packName = name;
		store.metadata = { ...store.metadata, name };
		scheduleMetadataSave(store.metadata);
	}

	function finishPackTitleEdit() {
		const name = packTitle.trim();
		if (!name) {
			packTitle = store.packName;
			return;
		}
		packTitle = name;
		if (name !== store.packName) editPackTitle(name);
	}

	function handlePackTitleKeydown(event: KeyboardEvent) {
		if (event.key === 'Enter') packTitleInput?.blur();
		else if (event.key === 'Escape') {
			packTitle = store.packName;
			packTitleInput?.blur();
		}
	}

	function toggleNavigation() {
		navCollapsed = !navCollapsed;
		localStorage.setItem('pack-editor:navigation-collapsed', String(navCollapsed));
	}

	async function undo() {
		saveError = null;
		try {
			taskFeedback.progress(
				'history',
				history.undoLabel ? `Undoing ${history.undoLabel}…` : 'Undoing change…'
			);
			await flushMetadataSave();
			await flushBehaviourSave();
			await history.undo();
			taskFeedback.success('history', 'Change undone');
		} catch (err) {
			saveError = `Undo failed: ${String(err)}`;
			taskFeedback.error('history', saveError);
		}
	}

	async function redo() {
		saveError = null;
		try {
			taskFeedback.progress(
				'history',
				history.redoLabel ? `Redoing ${history.redoLabel}…` : 'Redoing change…'
			);
			await flushMetadataSave();
			await flushBehaviourSave();
			await history.redo();
			taskFeedback.success('history', 'Change redone');
		} catch (err) {
			saveError = `Redo failed: ${String(err)}`;
			taskFeedback.error('history', saveError);
		}
	}

	async function save() {
		if (!store.beginSave()) return;
		saveError = null;
		if (store.uploading && !store.packHasDestination)
			taskFeedback.warning('save', 'Waiting for the import to finish before the first save…');
		else if (store.uploading)
			taskFeedback.warning('save', 'Saving now — unfinished uploads won’t be included');
		else if (store.packHasDestination) taskFeedback.progress('save', 'Saving pack…');
		try {
			await flushMetadataSave();
			await flushBehaviourSave();
			const info = await api.savePack(onSaveDestinationChosen);
			if (info) {
				store.packName = info.name;
				store.packHasDestination = info.has_destination;
			} else {
				store.endSave();
				taskFeedback.dismiss('save');
			}
		} catch (err) {
			// The backend only emits save:done on success, so a failed save would
			// otherwise leave the "Saving… X/Y" progress bar stuck on screen forever.
			saveError = String(err);
			taskFeedback.error('save', `Save failed: ${saveError}`);
			store.endSave();
		}
	}

	async function saveAs() {
		if (!store.beginSave()) return;
		saveError = null;
		if (store.uploading)
			taskFeedback.warning('save', 'Waiting for the import to finish before Save As…');
		try {
			await flushMetadataSave();
			await flushBehaviourSave();
			const info = await api.savePackAsDialog(onSaveDestinationChosen);
			if (info) {
				store.packName = info.name;
				store.packHasDestination = true;
			} else {
				store.endSave();
				taskFeedback.dismiss('save');
			}
		} catch (err) {
			saveError = String(err);
			taskFeedback.error('save', `Save failed: ${saveError}`);
			store.endSave();
		}
	}

	async function discard() {
		if (store.saveActive || store.uploading) {
			taskFeedback.warning(
				'pack-action',
				store.saveActive
					? 'Wait for the save to finish before discarding'
					: 'Stop the import before discarding changes'
			);
			return;
		}
		cancelMetadataSave();
		cancelBehaviourSave();
		saveError = null;
		try {
			const meta = await api.discardChanges();
			store.metadata = meta;
			store.packName = meta.name;
			packTitle = meta.name;
			store.markPackSaved();
			const [files, tags, artists, behaviour] = await Promise.all([
				api.getFiles(),
				api.getAllTags(),
				api.getAllArtists(),
				api.getBehaviour()
			]);
			store.files = files;
			store.allTags = tags;
			store.allArtists = artists;
			store.behaviour = behaviour;
			store.suspendedExperience = null;
			initializeMetadataHistory(meta);
			initializeBehaviourHistory(behaviour);
			history.reset(true);
			taskFeedback.success('pack-action', 'Changes discarded');
		} catch (err) {
			saveError = `Could not discard changes: ${String(err)}`;
			taskFeedback.error('pack-action', saveError);
		}
	}

	async function finishClosePack() {
		cancelBehaviourSave();
		cancelMetadataSave();
		await api.closePack();
		store.closePack();
	}

	async function requestClosePack() {
		if (store.uploading) {
			taskFeedback.warning('pack-action', 'Stop the import before closing the pack');
			return;
		}
		if (store.saveActive) {
			closePackAfterSave = true;
			taskFeedback.progress(
				'save',
				'Finishing save before closing pack…',
				store.saveDone,
				store.saveTotal || null
			);
		} else {
			try {
				await flushMetadataSave();
				await flushBehaviourSave();
				const saved = await api.isPackSaved();
				store.packSaved = saved;
				if (saved) await finishClosePack();
				else showClosePackDialog = true;
			} catch (error) {
				taskFeedback.error(
					'pack-action',
					`Could not verify whether the pack is saved: ${String(error)}`
				);
			}
		}
	}

	async function saveAndClosePack() {
		showClosePackDialog = false;
		if (!store.beginSave()) {
			closePackAfterSave = true;
			return;
		}
		saveError = null;
		try {
			await flushMetadataSave();
			await flushBehaviourSave();
			const info = await api.savePack(onSaveDestinationChosen);
			if (!info) {
				store.endSave();
				taskFeedback.dismiss('save');
				return;
			}
			if (info.has_unsaved_changes) showClosePackDialog = true;
			else await finishClosePack();
		} catch (err) {
			saveError = String(err);
			taskFeedback.error('save', `Save failed: ${saveError}`);
			store.endSave();
		}
	}

	async function discardAndClosePack() {
		showClosePackDialog = false;
		if (store.saveActive) {
			closePackAfterSave = true;
			return;
		}
		if (store.uploading) {
			taskFeedback.warning('pack-action', 'Stop the import before closing the pack');
			return;
		}
		cancelBehaviourSave();
		cancelMetadataSave();
		saveError = null;
		try {
			await api.discardPack();
			store.closePack();
		} catch (err) {
			saveError = `Could not discard changes: ${String(err)}`;
			taskFeedback.error('pack-action', saveError);
		}
	}

	async function confirmMediaRemoval() {
		if (removingMedia) return;
		const ids = store.pendingMediaRemoval;
		if (!ids.length) return;
		removingMedia = true;
		await tick();
		const idSet = new Set(ids);
		const removed = store.files
			.filter((file) => idSet.has(file.id))
			.map((file) => $state.snapshot(file) as MediaFile);
		const activeIndex =
			store.gridActiveId == null
				? -1
				: store.filteredFiles.findIndex((file) => file.id === store.gridActiveId);
		taskFeedback.progress(
			'media-removal',
			`Removing ${ids.length} media item${ids.length === 1 ? '' : 's'}…`
		);
		try {
			await api.removeFiles(ids);
			store.cancelMediaRemoval();
			store.removeFilesById(ids, true);
			history.record({
				label:
					removed.length === 1
						? `Remove “${removed[0].file_name}”`
						: `Remove ${removed.length} media items`,
				storageBytes: removed.reduce((total, file) => total + file.size, 0)
			});
			const remaining = store.filteredFiles;
			if (remaining.length > 0) {
				const next = remaining[Math.min(Math.max(activeIndex, 0), remaining.length - 1)];
				store.selectSingle(next.id);
			}
			taskFeedback.success('media-removal');
		} catch (error) {
			taskFeedback.error('media-removal', `Could not remove media: ${String(error)}`);
		} finally {
			removingMedia = false;
		}
	}
</script>

<div class="bg-bg text-text flex h-screen flex-col select-none">
	<!-- Toolbar -->
	<header class="bg-surface border-border flex h-11 shrink-0 items-center gap-2 border-b px-3">
		<div class="flex items-center gap-0">
			<input
				bind:this={packTitleInput}
				class="pack-title text-text truncate text-sm font-semibold"
				aria-label="Pack title"
				title="Edit pack title"
				value={packTitle}
				disabled={!store.metadata}
				oninput={(event) => editPackTitle(event.currentTarget.value)}
				onblur={finishPackTitleEdit}
				onkeydown={handlePackTitleKeydown}
			/>
			{#if store.recoveryStatus === 'error'}
				<span
					class="recovery-status flex items-center gap-1.5 font-mono text-[11px] text-[var(--ui-danger)]"
					role="alert"
					title={store.recoveryError ?? 'Changes could not be backed up locally.'}
				>
					<span class="h-1.5 w-1.5 shrink-0 rounded-full bg-[var(--ui-danger)]"></span>
					<span class="recovery-label">Backup failed</span>
				</span>
			{:else if store.recoveryStatus !== 'saved'}
				<span
					class="bg-muted h-1.5 w-1.5 shrink-0 rounded-full {store.recoveryStatus === 'pending'
						? 'animate-pulse'
						: ''}"
					role="status"
					aria-label="Unsaved changes"
					title={store.recoveryStatus === 'pending'
						? 'Backing up changes…'
						: store.packHasDestination
							? 'Unsaved changes — backed up locally'
							: 'Draft — backed up locally; choose a destination on first save'}
				></span>
			{/if}
		</div>
		<div class="flex-1"></div>
		<TaskStatus />
		<div class="flex items-center">
			<IconButton
				label={history.undoLabel ? `Undo ${history.undoLabel}` : 'Undo'}
				disabled={!history.canUndo}
				onclick={undo}
				title={`Undo (${modifierLabel}+Z)`}
			>
				<span class="h-4 w-4"><Icon src={ArrowUturnLeft} mini /></span>
			</IconButton>
			<IconButton
				label={history.redoLabel ? `Redo ${history.redoLabel}` : 'Redo'}
				disabled={!history.canRedo}
				onclick={redo}
				title={`Redo (${modifierLabel}+Shift+Z)`}
			>
				<span class="h-4 w-4"><Icon src={ArrowUturnRight} mini /></span>
			</IconButton>
		</div>
		<Button
			size="compact"
			variant="primary"
			onclick={save}
			disabled={store.packSaved || store.saveActive}
			loading={store.saveActive && (store.packHasDestination || saveDestinationChosen)}
			title={`Save (${modifierLabel}+S)`}>Save</Button
		>
		<Popover align="end" label="Pack actions">
			{#snippet trigger(toggle, open)}
				<button
					onclick={toggle}
					aria-label="More pack actions"
					aria-haspopup="menu"
					aria-expanded={open}
					class="text-muted hover:text-text hover:bg-surface-2 grid h-8 w-8 place-items-center rounded"
					><Icon src={EllipsisVertical} mini size="16px" /></button
				>
			{/snippet}
			{#snippet children(close)}
				<div class="w-48 py-1">
					<button
						role="menuitem"
						disabled={store.saveActive}
						onclick={() => {
							close();
							saveAs();
						}}
						class="hover:bg-bg flex w-full items-center justify-between gap-3 px-3 py-2 text-left text-xs disabled:cursor-not-allowed disabled:opacity-40"
						><span>Save As…</span><kbd class="text-muted text-[10px]">{modifierLabel}+Shift+S</kbd
						></button
					>
					{#if !store.packSaved && store.packHasDestination}<button
							role="menuitem"
							disabled={store.saveActive}
							onclick={() => {
								close();
								discard();
							}}
							class="hover:bg-bg w-full px-3 py-2 text-left text-xs text-[var(--ui-warning)] disabled:cursor-not-allowed disabled:opacity-40"
							>Discard changes</button
						>{/if}
					<div class="border-border my-1 border-t"></div>
					<button
						role="menuitem"
						onclick={() => {
							close();
							requestClosePack();
						}}
						class="w-full px-3 py-2 text-left text-xs text-[var(--ui-danger)] hover:bg-[var(--ui-danger-bg)]"
						>Close pack</button
					>
				</div>
			{/snippet}
		</Popover>
	</header>

	<div class="flex min-h-0 flex-1">
		<aside
			class="bg-surface border-border flex shrink-0 flex-col overflow-hidden border-r transition-[width] duration-150 {navigationCollapsed
				? 'w-12'
				: 'w-44'}"
		>
			<div
				class="border-border flex h-11 items-center border-b px-2 {navigationCollapsed
					? 'justify-center'
					: 'justify-between'}"
			>
				{#if !navigationCollapsed}<span class="text-text px-1 text-sm font-semibold">Sections</span
					>{/if}
				{#if !narrowWindow}<IconButton
						label={navCollapsed ? 'Expand navigation' : 'Collapse navigation'}
						onclick={toggleNavigation}
					>
						<span class="h-4 w-4"
							><Icon src={navCollapsed ? ChevronRight : ChevronLeft} mini /></span
						>
					</IconButton>{/if}
			</div>
			<nav class="p-2 {navigationCollapsed ? '' : 'w-44'}">
				<Tabs
					tabs={navigationTabs}
					active={store.activeView}
					orientation="vertical"
					collapsed={navigationCollapsed}
					onselect={(id) => (store.activeView = id as typeof store.activeView)}
				/>
			</nav>
		</aside>

		<!-- Main content -->
		<div class="flex min-w-0 flex-1 flex-col">
			{#key store.historyRevision}
				{#if store.activeView === 'media'}
					<MediaToolbar />
					<div class="flex min-h-0 flex-1 max-[520px]:flex-col">
						<div class="min-h-0 min-w-0 flex-1">
							{#if store.filteredFiles.length === 0 && store.files.length === 0}
								<div class="flex h-full items-center justify-center p-8">
									<div class="w-full max-w-lg">
										<EmptyState
											title="Add media to this pack"
											description="Import images, videos, or audio files. You can also drag files or folders anywhere onto this window."
											actionLabel="Import files…"
											onclick={() => api.addFilesDialog()}
											secondaryActionLabel="Import folder…"
											onsecondary={() => api.addFolderDialog()}
										/>
									</div>
								</div>
							{:else if store.filteredFiles.length === 0}
								<div class="flex h-full items-center justify-center p-8">
									<div class="w-full max-w-lg">
										<EmptyState
											title="No matching media"
											description="No media matches the current search, type, or tag filters."
											actionLabel="Clear filters"
											onclick={() => {
												store.searchQuery = '';
												store.mediaTypeFilter = 'all';
												store.tagFilter = new Set();
												store.artistFilter = new Set();
											}}
										/>
									</div>
								</div>
							{:else}
								<MediaGrid />
							{/if}
						</div>
						<Sidebar />
					</div>
				{:else if store.activeView === 'content'}
					<div class="flex min-h-0 flex-1 flex-col">
						<Content />
					</div>
				{:else if store.activeView === 'tags'}
					<Tags />
				{:else if store.activeView === 'artists'}
					<Artists />
				{:else if store.activeView === 'experience'}
					<div class="flex min-h-0 flex-1 flex-col">
						<Experience />
					</div>
				{:else if store.activeView === 'modes'}
					<Modes />
				{:else}
					<div class="flex-1 overflow-y-auto" use:clampScroll>
						<Options />
					</div>
				{/if}
			{/key}
		</div>
	</div>

	<!-- Upload progress bar -->
	{#if store.showUploadProgress}
		<UploadProgress />
	{/if}
</div>

{#if showClosePackDialog}
	<Dialog
		title="Unsaved changes"
		description="You have unsaved changes. What would you like to do?"
		buttons={[
			{ label: 'Cancel', onclick: () => (showClosePackDialog = false) },
			{ label: 'Discard', destructive: true, onclick: discardAndClosePack },
			{ label: 'Save', primary: true, onclick: saveAndClosePack }
		]}
		onclose={() => (showClosePackDialog = false)}
	/>
{/if}

{#if store.pendingMediaRemoval.length > 0}
	{@const removalCount = store.pendingMediaRemoval.length}
	{@const removalFile =
		removalCount === 1
			? store.files.find((file) => file.id === store.pendingMediaRemoval[0])
			: null}
	<Dialog
		title={removalCount === 1
			? 'Remove media from pack?'
			: `Remove ${removalCount} items from pack?`}
		description={removalCount === 1
			? `“${removalFile?.file_name ?? 'This item'}” will be removed from this pack. The original file on your computer will not be deleted.`
			: `These ${removalCount} media items will be removed from this pack. The original files on your computer will not be deleted.`}
		buttons={[
			{ label: 'Cancel', disabled: removingMedia, onclick: () => store.cancelMediaRemoval() },
			{
				label: removingMedia
					? 'Removing…'
					: removalCount === 1
						? 'Remove item'
						: `Remove ${removalCount} items`,
				destructive: true,
				loading: removingMedia,
				onclick: confirmMediaRemoval
			}
		]}
		onclose={removingMedia ? undefined : () => store.cancelMediaRemoval()}
	/>
{/if}

<!-- Media viewer overlay -->
{#if store.openedId !== null}
	<MediaViewer />
{/if}

<!-- Edgeware import warnings -->
{#if store.importWarnings.length > 0}
	<ImportWarnings />
{/if}

<!-- Drag and drop overlay -->
{#if store.dragActive}
	<div class="drop-overlay pointer-events-none fixed inset-0 z-[60] grid place-items-center">
		<div class="drop-window">
			<div class="drop-titlebar"><span class="drop-dot"></span><span>Import</span></div>
			<div class="drop-body">Drop files or folders to import them into this pack.</div>
		</div>
	</div>
{/if}

<style>
	.pack-title {
		field-sizing: content;
		min-width: 1ch;
		max-width: min(36vw, 360px);
		padding: 2px 2px 2px 4px;
		border: 1px solid transparent;
		border-radius: var(--ui-radius-sm);
		background: transparent;
		outline: none;
		margin-right: 0.25rem;
	}
	.pack-title:hover:not(:disabled) {
		border-color: var(--ui-border);
		background: var(--ui-bg);
	}
	.pack-title:focus {
		border-color: var(--ui-focus);
		background: var(--ui-bg);
	}
	.pack-title:disabled {
		opacity: 1;
	}
	.drop-overlay {
		background: rgb(0 0 0 / 0.62);
	}
	.drop-window {
		position: relative;
		width: min(380px, calc(100vw - 64px));
		border: 1px solid var(--ui-accent);
		border-radius: var(--ui-radius-md);
		background: var(--ui-surface);
		box-shadow: var(--ui-shadow-pop);
	}
	.drop-window::before,
	.drop-window::after {
		content: '';
		position: absolute;
		inset: 0;
		z-index: -1;
		border: 1px solid var(--ui-border);
		border-radius: var(--ui-radius-md);
		background: rgb(10 8 9 / 0.4);
	}
	.drop-window::before {
		transform: translate(-18px, -16px);
		opacity: 0.5;
	}
	.drop-window::after {
		transform: translate(-9px, -8px);
	}
	.drop-titlebar {
		display: flex;
		height: 32px;
		padding: 0 10px;
		align-items: center;
		gap: 8px;
		border-bottom: 1px solid var(--ui-border);
		border-radius: var(--ui-radius-md) var(--ui-radius-md) 0 0;
		background: var(--ui-surface-raised);
		color: var(--ui-text);
		font-family: var(--ui-font-mono);
		font-size: 11.5px;
		font-weight: 700;
	}
	.drop-dot {
		width: 8px;
		height: 8px;
		flex: none;
		border-radius: 50%;
		background: var(--ui-accent);
	}
	.drop-body {
		padding: 20px 16px;
		color: var(--ui-muted);
		font-size: 13px;
	}
	@media (max-width: 760px) {
		.pack-title {
			max-width: 28vw;
		}
		.recovery-label {
			position: absolute;
			width: 1px;
			height: 1px;
			overflow: hidden;
			clip-path: inset(50%);
			white-space: nowrap;
		}
		.recovery-status {
			flex: none;
		}
	}
	@media (max-width: 520px) {
		.pack-title {
			max-width: 20vw;
		}
	}
</style>
