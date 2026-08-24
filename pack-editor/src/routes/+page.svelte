<script lang="ts">
	import { onMount } from 'svelte';
	import { listen } from '@tauri-apps/api/event';
	import { store } from '$lib/store.svelte.js';
	import { api } from '$lib/api.js';
	import { invalidate, keys } from '$lib/query.svelte.js';
	import { cancelPendingWrites, flushPendingWrites, packSave } from '$lib/packActions.svelte.js';
	import type { FilledSlot, MediaFile, UploadError, SaveDone, SaveProgress } from '$lib/types.js';
	import Start from '$lib/Start.svelte';
	import Editor from '$lib/Editor.svelte';
	import Dialog from '$ui/Dialog.svelte';
	import { history } from '$lib/history.svelte.js';
	import { taskFeedback } from '$ui/taskFeedback.svelte.js';

	let showCloseDialog = $state(false);
	let closeError = $state<string | null>(null);
	let pendingClose = $state(false);
	let checkingClose = false;
	let pendingImportToken: number | null = null;
	let pendingImportFiles: MediaFile[] = [];

	$effect(() => {
		if (!pendingClose || store.saveActive) return;
		pendingClose = false;
		void resolveCloseRequest();
	});

	async function resolveCloseRequest() {
		if (checkingClose) return;
		if (store.uploading) {
			taskFeedback.warning('pack-action', 'Stop the import before closing Lewdware Pack Editor');
			return;
		}
		if (store.saveActive) {
			pendingClose = true;
			taskFeedback.progress(
				'save',
				'Finishing save before closing…',
				store.saveDone,
				store.saveTotal || null
			);
			return;
		}
		if (!store.packOpen) {
			await api.confirmClose();
			return;
		}
		checkingClose = true;
		try {
			await flushPendingWrites();
			const saved = await api.isPackSaved();
			store.packSaved = saved;
			if (saved) await api.confirmClose();
			else showCloseDialog = true;
		} catch (error) {
			showCloseDialog = true;
			taskFeedback.error(
				'pack-action',
				`Could not verify whether the pack is saved: ${String(error)}`
			);
		} finally {
			checkingClose = false;
		}
	}

	function finalizeImportHistory() {
		if (pendingImportToken === null) return;
		const token = pendingImportToken;
		const files = pendingImportFiles.map((file) => structuredClone(file));
		pendingImportToken = null;
		pendingImportFiles = [];
		if (!files.length) {
			history.finalize(token, null);
			return;
		}
		history.finalize(token, {
			label:
				files.length === 1
					? `Import “${files[0].file_name}”`
					: `Import ${files.length} media items`,
			storageBytes: files.reduce((total, file) => total + file.size, 0)
		});
	}

	onMount(() => {
		api
			.getMediaServer()
			.then((server) => {
				store.mediaPort = server.port;
				store.mediaToken = server.token;
			})
			.catch((error) =>
				taskFeedback.error('media-server', `Previews unavailable: ${String(error)}`)
			);

		const unsubs = [
			// Import feedback (progress, errors, completion) is owned by the UploadProgress window.
			listen<{ total: number }>('upload:start', (e) => {
				if (store.uploadBatches === 0) {
					pendingImportToken = history.reserve('Import still in progress');
					pendingImportFiles = [];
				}
				store.onUploadStart(e.payload.total);
			}),
			listen<MediaFile>('upload:added', (e) => {
				store.addFile(e.payload, true);
				pendingImportFiles.push(structuredClone(e.payload));
				if (pendingImportToken !== null) history.touchPending(pendingImportToken);
			}),
			listen<UploadError>('upload:error', (e) => {
				store.addUploadError(e.payload);
			}),
			listen('upload:skipped', () => {
				store.onUploadSkipped();
			}),
			listen('upload:file-done', () => {
				store.onUploadFileDone();
			}),
			listen('upload:done', () => {
				store.onUploadDone();
				if (store.uploadBatches === 0) finalizeImportHistory();
			}),
			// Emitted by an Edgeware import as each file a media slot names arrives, and only when
			// it really filled a slot -- the one part of an imported behaviour written after the
			// front end already has the document.
			listen<FilledSlot[]>('import:slots-filled', (e) => {
				// The slots an Edgeware import fills as its media lands, well after the command that
				// started it returned. Nothing holds a document to patch, so this is just "the
				// answer moved": whatever is showing a slot asks again.
				invalidate(keys.behaviour);
			}),
			listen<SaveProgress>('save:progress', (e) => {
				store.saveActive = true;
				store.saveDone = e.payload.saved;
				store.saveTotal = e.payload.total;
				taskFeedback.progress('save', 'Saving pack…', e.payload.saved, e.payload.total);
			}),
			listen('save:in-place', () => {
				store.saveBlocksPreviews = true;
			}),
			listen<SaveDone>('save:done', (event) => {
				store.endSave();
				taskFeedback.dismiss('preview');
				if (event.payload.has_unsaved_changes) {
					taskFeedback.warning('save', 'Pack saved — newer changes remain unsaved');
				} else {
					history.markSaved();
					taskFeedback.success('save', 'Pack saved');
				}
			}),
			listen('close-requested', () => {
				void resolveCloseRequest();
			})
		];

		return () => {
			unsubs.forEach((p) => p.then((fn) => fn()));
		};
	});

	async function onCloseSave() {
		showCloseDialog = false;
		pendingClose = true;
		if (store.saveActive) return;
		store.beginSave();
		const { info, error } = await packSave.run('save');
		// Either way the pack is still open; only a failure is worth a dialog of its own, since a
		// dismissed destination picker is the user having already said no.
		if (!info) {
			pendingClose = false;
			if (error) closeError = `Save failed: ${String(error)} The pack was not closed.`;
			return;
		}
		store.packHasDestination = info.has_destination;
	}

	async function onCloseDiscard() {
		showCloseDialog = false;
		cancelPendingWrites();
		try {
			await api.discardPack();
			await api.confirmClose();
		} catch (err) {
			closeError = `Could not discard changes: ${String(err)} The pack was not closed.`;
			taskFeedback.error('pack-action', `Could not discard changes: ${String(err)}`);
		}
	}

	function onCloseCancel() {
		showCloseDialog = false;
	}
</script>

{#if store.packOpen}
	<Editor />
{:else}
	<Start />
{/if}

{#if closeError}
	<Dialog
		title="Could not close the editor"
		description={closeError}
		buttons={[{ label: 'OK', primary: true, onclick: () => (closeError = null) }]}
		onclose={() => (closeError = null)}
	/>
{/if}

{#if showCloseDialog}
	<Dialog
		title="Unsaved changes"
		description="You have unsaved changes. What would you like to do?"
		buttons={[
			{ label: 'Cancel', onclick: onCloseCancel },
			{ label: 'Discard', destructive: true, onclick: onCloseDiscard },
			{ label: 'Save', primary: true, onclick: onCloseSave }
		]}
		onclose={onCloseCancel}
	/>
{/if}
