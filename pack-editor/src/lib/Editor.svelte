<script lang="ts">
	import { clampScroll } from '$ui/scroll';
	import Tabs from '$ui/Tabs.svelte';
	import IconButton from '$ui/IconButton.svelte';
	import Dialog from '$ui/Dialog.svelte';
	import {
		ChevronLeft,
		ChevronRight,
		Clock,
		CodeBracketSquare,
		Cog6Tooth,
		DocumentText,
		Icon,
		MusicalNote,
		PaintBrush,
		Photo,
		Squares2x2,
		Tag
	} from 'svelte-hero-icons';
	import { modifierKeyLabel } from './platform.js';
	import { onMount, tick } from 'svelte';
	import { getCurrentWebview } from '@tauri-apps/api/webview';
	import { api } from './api.js';
	import { isMediaView, store } from './store.svelte.js';
	import DropOverlay from './DropOverlay.svelte';
	import EditorToolbar from './EditorToolbar.svelte';
	import MediaGrid from './MediaGrid.svelte';
	import Sidebar from './Sidebar.svelte';
	import Options from './Options.svelte';
	import Content from './Content.svelte';
	import Experience from './Experience.svelte';
	import Modes from './Modes.svelte';
	import UploadProgress from './UploadProgress.svelte';
	import MediaViewer from './MediaViewer.svelte';
	import MediaPreview from './MediaPreview.svelte';
	import AudioList from './AudioList.svelte';
	import ImportWarnings from './ImportWarnings.svelte';
	import { formatList } from './format.js';
	import MediaToolbar from './MediaToolbar.svelte';
	import Tags from './Tags.svelte';
	import Artists from './Artists.svelte';
	import { initializeMetadataHistory, scheduleMetadataSave } from './metadataSave.svelte.js';
	import { cancelPendingWrites, flushPendingWrites, packSave } from './packActions.svelte.js';
	import { history } from './history.svelte.js';
	import { invalidate, keys, query } from './query.svelte.js';
	import { taskFeedback } from '$ui/taskFeedback.svelte.js';
	import type { MediaFile } from './types.js';
	import EmptyState from '$ui/EmptyState.svelte';

	// Which of the three media tabs to open in. One key for the editor, not one per pack: it records
	// a habit of the person using it, the way the nav's collapsed state and the inspector's width
	// beside it do, and a key per pack would accrue one for every pack ever opened.
	const LAST_MEDIA_VIEW_KEY = 'pack-editor:last-media-view';

	let navCollapsed = $state(false);
	let narrowWindow = $state(false);
	let showClosePackDialog = $state(false);
	let closePackAfterSave = $state(false);
	let removingMedia = $state(false);
	let modifierLabel = $state('Ctrl');
	let packTitle = $state(store.packName);
	let toolbar = $state<ReturnType<typeof EditorToolbar>>();
	let mediaViewRestored = $state(false);

	const navigationTabs = [
		{ id: 'popups', label: 'Popups', icon: Photo, group: 'Media' },
		{ id: 'audio', label: 'Audio', icon: MusicalNote, group: 'Media' },
		{ id: 'all-media', label: 'All media', icon: Squares2x2, group: 'Media' },
		{ id: 'tags', label: 'Tags', icon: Tag, group: 'Organization' },
		{ id: 'artists', label: 'Artists', icon: PaintBrush, group: 'Organization' },
		{ id: 'content', label: 'Content', icon: DocumentText, group: 'Behaviour' },
		{ id: 'experience', label: 'Timeline', icon: Clock, group: 'Behaviour' },
		{ id: 'modes', label: 'Modes', icon: CodeBracketSquare, group: 'Pack' },
		{ id: 'options', label: 'Metadata', icon: Cog6Tooth, group: 'Pack' }
	];
	const navigationCollapsed = $derived(navCollapsed || narrowWindow);

	$effect(() => {
		const name = store.packName;
		if (!toolbar?.isEditingTitle()) packTitle = name;
	});

	$effect(() => {
		if (!closePackAfterSave || store.saveActive) return;
		closePackAfterSave = false;
		if (store.packSaved) void finishClosePack();
		else showClosePackDialog = true;
	});

	$effect(() => {
		if (!store.saveActive) packSave.destinationChosen = false;
	});

	// The only writer of the remembered media tab: every route into one goes through
	// `store.lastMediaView`, including the deep links from the inspector's "Used as". Waits for the
	// restore, which would otherwise be overwritten by the default it is about to replace.
	$effect(() => {
		if (mediaViewRestored) localStorage.setItem(LAST_MEDIA_VIEW_KEY, store.lastMediaView);
	});

	onMount(() => {
		modifierLabel = modifierKeyLabel();
		navCollapsed = localStorage.getItem('pack-editor:navigation-collapsed') === 'true';
		const rememberedMediaView = localStorage.getItem(LAST_MEDIA_VIEW_KEY);
		if (isMediaView(rememberedMediaView)) store.setActiveView(rememberedMediaView);
		mediaViewRestored = true;
		// The app supports an 800 px-wide window. Collapse the global navigation before the
		// editor and inspector panes become too narrow to lay out their own controls.
		const narrowQuery = window.matchMedia('(max-width: 1000px)');
		const updateNarrowWindow = () => (narrowWindow = narrowQuery.matches);
		updateNarrowWindow();
		narrowQuery.addEventListener('change', updateNarrowWindow);
		const handleShortcut = (event: KeyboardEvent) => {
			if (event.defaultPrevented || !(event.ctrlKey || event.metaKey) || event.altKey) return;
			if (
				showClosePackDialog ||
				store.pendingMediaRemoval.length > 0 ||
				store.openedId !== null ||
				store.previewId !== null
			)
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

	// Which media slots the files being removed are filling, so the confirmation can name them.
	//
	// Deleting a file clears the slots referencing it, which is right and is a surprising thing to
	// discover afterwards — the pack quietly stops having a wallpaper. Asked of the backend, which
	// can see slots on a switched-off timeline that a read document would hide.
	const removalUsageQuery = query(
		() => `media-usage:${store.pendingMediaRemoval.join(',')}`,
		async () => {
			const perFile = await Promise.all(
				store.pendingMediaRemoval.map((id) => api.getMediaUsage(id))
			);
			return [...new Set(perFile.flat())];
		}
	);

	onMount(async () => {
		// Nothing is preloaded here any more. The document used to be fetched with the pack because
		// the media tabs read it without owning it — the inspector's "Used as" line, and naming the
		// slots a removal would clear. Each of those asks for what it shows now, when it shows it.
		try {
			const metadata = await api.getPackMetadata();
			store.metadata = metadata;
			store.packName = metadata.name;
			packTitle = metadata.name;
			initializeMetadataHistory(metadata);
		} catch (error) {
			taskFeedback.error('metadata-load', `Could not load pack metadata: ${String(error)}`);
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
		if (event.key === 'Enter') toolbar?.blurTitle();
		else if (event.key === 'Escape') {
			packTitle = store.packName;
			toolbar?.blurTitle();
		}
	}

	function toggleNavigation() {
		navCollapsed = !navCollapsed;
		localStorage.setItem('pack-editor:navigation-collapsed', String(navCollapsed));
	}

	/** Undo and redo differ only in direction, and both must flush before the backend rewinds. */
	async function step(direction: 'undo' | 'redo') {
		const undoing = direction === 'undo';
		const label = undoing ? history.undoLabel : history.redoLabel;
		const verb = undoing ? 'Undoing' : 'Redoing';
		try {
			taskFeedback.progress('history', label ? `${verb} “${label}”…` : `${verb} change…`);
			// A pending write that fails must not block getting back to a good state: it never
			// reached the pack, so there is nothing of it to undo, and `mutate` has already said so.
			await flushPendingWrites().catch(() => {});
			await (undoing ? history.undo() : history.redo());
			taskFeedback.success('history', undoing ? 'Change undone' : 'Change redone');
		} catch (error) {
			taskFeedback.error('history', `${undoing ? 'Undo' : 'Redo'} failed: ${String(error)}`);
		}
	}

	const undo = () => step('undo');
	const redo = () => step('redo');

	async function write(mode: 'save' | 'save-as') {
		if (!store.beginSave()) return;
		const { info } = await packSave.run(mode);
		if (!info) return;
		store.packId = info.id;
		store.packName = info.name;
		// A Save As always leaves the pack with the destination it just chose.
		store.packHasDestination = mode === 'save-as' || info.has_destination;
	}

	const save = () => write('save');
	const saveAs = () => write('save-as');

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
		cancelPendingWrites();
		try {
			const meta = await api.discardChanges();
			store.metadata = meta;
			store.packName = meta.name;
			packTitle = meta.name;
			store.markPackSaved();
			const [files, tags, artists] = await Promise.all([
				api.getFiles(),
				api.getAllTags(),
				api.getAllArtists()
			]);
			store.files = files;
			store.allTags = tags;
			store.allArtists = artists;
			// Discarding restores the pack from its saved archive, so nothing the surfaces are
			// showing survives it.
			invalidate(keys.behaviour);
			invalidate(keys.tags);
			invalidate(keys.artists);
			initializeMetadataHistory(meta);
			history.reset(true);
			taskFeedback.success('pack-action', 'Changes discarded');
		} catch (error) {
			taskFeedback.error('pack-action', `Could not discard changes: ${String(error)}`);
		}
	}

	async function finishClosePack() {
		cancelPendingWrites();
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
				await flushPendingWrites();
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
		const { info } = await packSave.run('save');
		if (!info) return;
		// A pack that still reports unsaved changes gained some while it was being written; ask
		// again rather than closing over them.
		if (info.has_unsaved_changes) showClosePackDialog = true;
		else await finishClosePack();
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
		cancelPendingWrites();
		try {
			await api.discardPack();
			store.closePack();
		} catch (error) {
			taskFeedback.error('pack-action', `Could not discard changes: ${String(error)}`);
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
			store.mediaTab.gridActiveId == null
				? -1
				: store.filteredFiles.findIndex((file) => file.id === store.mediaTab.gridActiveId);
		taskFeedback.progress(
			'media-removal',
			`Removing ${ids.length} media item${ids.length === 1 ? '' : 's'}…`
		);
		try {
			// Deleting a file clears any slot that named it -- see `MediaPack::remove_files`, and
			// the dialog above names those slots first so it is a decision rather than a surprise.
			await api.removeFiles(ids);
			store.cancelMediaRemoval();
			store.removeFilesById(ids, true);
			const record = {
				label:
					removed.length === 1
						? `Remove “${removed[0].file_name}”`
						: `Remove ${removed.length} media items`,
				storageBytes: removed.reduce((total, file) => total + file.size, 0)
			};
			invalidate(keys.behaviour);
			history.record(record);
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

<div class="bg-bg text-text flex h-full flex-col select-none">
	<EditorToolbar
		bind:this={toolbar}
		{packTitle}
		{modifierLabel}
		onedittitle={editPackTitle}
		onfinishtitle={finishPackTitleEdit}
		ontitlekeydown={handlePackTitleKeydown}
		onundo={undo}
		onredo={redo}
		onsave={save}
		onsaveas={saveAs}
		ondiscard={discard}
		onclosepack={requestClosePack}
	/>

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
			<nav
				class="min-h-0 flex-1 overflow-y-auto p-2 {navigationCollapsed ? '' : 'w-44'}"
				use:clampScroll
			>
				<Tabs
					tabs={navigationTabs}
					active={store.activeView}
					orientation="vertical"
					collapsed={navigationCollapsed}
					onselect={(id) => store.setActiveView(id as typeof store.activeView)}
				/>
			</nav>
		</aside>

		<!-- Main content -->
		<div class="flex min-w-0 flex-1 flex-col">
			{#if store.activeView === 'popups' || store.activeView === 'all-media'}
				{#if store.activeView === 'all-media'}
					<div class="border-border bg-surface border-b px-3 py-2">
						<p class="text-muted text-xs">
							Everything in the pack, including wallpapers and splashes. Custom modes can draw from
							all of it.
						</p>
					</div>
				{/if}
				<MediaToolbar view={store.activeView} />
				<div class="flex min-h-0 flex-1 max-[520px]:flex-col">
					<div class="min-h-0 min-w-0 flex-1">
						{#if store.filteredFiles.length === 0 && store.mediaScopeFiles.length === 0}
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
										onclick={() => store.clearMediaFilters()}
									/>
								</div>
							</div>
						{:else}
							<MediaGrid />
						{/if}
					</div>
					<Sidebar />
				</div>
			{:else if store.activeView === 'audio'}
				<MediaToolbar view="audio" />
				<div class="flex min-h-0 flex-1 max-[520px]:flex-col">
					<div class="min-h-0 min-w-0 flex-1"><AudioList /></div>
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
	{@const removalUsage = removalUsageQuery.current ?? []}
	<Dialog
		title={removalCount === 1
			? 'Remove media from pack?'
			: `Remove ${removalCount} items from pack?`}
		description={`${
			removalCount === 1
				? `“${removalFile?.file_name ?? 'This item'}” will be removed from this pack.`
				: `These ${removalCount} media items will be removed from this pack.`
		}${
			removalUsage.length > 0 ? ` This also clears ${formatList(removalUsage)}.` : ''
		} The original file${removalCount === 1 ? '' : 's'} on your computer will not be deleted.`}
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

<!-- Standalone preview overlay for previews launched from a role slot or pool. -->
{#if store.previewId !== null}
	<MediaPreview />
{/if}

<!-- Edgeware import warnings -->
{#if store.importWarnings.length > 0}
	<ImportWarnings />
{/if}

<!-- Drag and drop overlay -->
{#if store.dragActive}
	<DropOverlay />
{/if}
