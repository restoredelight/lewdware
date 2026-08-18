<script lang="ts">
	// The editor's top bar: the pack's name, what the backup is doing, whatever the app is busy
	// with, undo/redo, Save, and the menu holding the rest of the pack actions.
	//
	// The actions themselves belong to the editor -- they are what the keyboard shortcuts run too,
	// so they cannot live here without being reached for twice. This is the bar; `Editor` is what
	// the bar does.
	import Button from '$ui/Button.svelte';
	import IconButton from '$ui/IconButton.svelte';
	import Popover from '$ui/Popover.svelte';
	import { ArrowUturnLeft, ArrowUturnRight, EllipsisVertical, Icon } from 'svelte-hero-icons';
	import { history } from './history.svelte.js';
	import { packSave } from './packActions.svelte.js';
	import { store } from './store.svelte.js';
	import TaskStatus from './TaskStatus.svelte';

	type Props = {
		/** The name as typed, which is the field's own until it is committed. */
		packTitle: string;
		/** ⌘ or Ctrl, for the shortcut hints. */
		modifierLabel: string;
		onedittitle: (value: string) => void;
		onfinishtitle: () => void;
		ontitlekeydown: (event: KeyboardEvent) => void;
		onundo: () => void;
		onredo: () => void;
		onsave: () => void;
		onsaveas: () => void;
		ondiscard: () => void;
		onclosepack: () => void;
	};

	let {
		packTitle,
		modifierLabel,
		onedittitle,
		onfinishtitle,
		ontitlekeydown,
		onundo,
		onredo,
		onsave,
		onsaveas,
		ondiscard,
		onclosepack
	}: Props = $props();

	let titleInput = $state<HTMLInputElement>();

	/** Whether the field has focus, so the store can overwrite it only when nobody is typing. */
	export function isEditingTitle() {
		return titleInput === document.activeElement;
	}

	export function blurTitle() {
		titleInput?.blur();
	}

	/**
	 * The backup indicator's tooltip.
	 *
	 * Detail goes here rather than on screen: a routine unsaved state is a bare muted dot, never a
	 * warning colour. See `shared-ui/DESIGN.md`, "Routine states are not warnings".
	 */
	const recoveryTitle = $derived(
		store.recoveryStatus === 'pending'
			? 'Backing up changes…'
			: store.packHasDestination
				? 'Unsaved changes — backed up locally'
				: 'Draft — backed up locally; choose a destination on first save'
	);
</script>

<header class="bg-surface border-border flex h-11 shrink-0 items-center gap-2 border-b px-3">
	<div class="flex items-center gap-0">
		<input
			bind:this={titleInput}
			class="pack-title text-text truncate text-sm font-semibold"
			aria-label="Pack title"
			title="Edit pack title"
			value={packTitle}
			disabled={!store.metadata}
			oninput={(event) => onedittitle(event.currentTarget.value)}
			onblur={onfinishtitle}
			onkeydown={ontitlekeydown}
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
				title={recoveryTitle}
			></span>
		{/if}
	</div>
	<div class="flex-1"></div>
	<TaskStatus />
	<div class="flex items-center">
		<IconButton
			label={history.undoLabel ? `Undo ${history.undoLabel}` : 'Undo'}
			disabled={!history.canUndo}
			onclick={onundo}
			title={history.undoLabel
				? `Undo “${history.undoLabel}” (${modifierLabel}+Z)`
				: `Undo (${modifierLabel}+Z)`}
		>
			<span class="h-4 w-4"><Icon src={ArrowUturnLeft} mini /></span>
		</IconButton>
		<IconButton
			label={history.redoLabel ? `Redo ${history.redoLabel}` : 'Redo'}
			disabled={!history.canRedo}
			onclick={onredo}
			title={history.redoLabel
				? `Redo “${history.redoLabel}” (${modifierLabel}+Shift+Z)`
				: `Redo (${modifierLabel}+Shift+Z)`}
		>
			<span class="h-4 w-4"><Icon src={ArrowUturnRight} mini /></span>
		</IconButton>
	</div>
	<Button
		size="compact"
		variant="primary"
		onclick={onsave}
		disabled={store.packSaved || store.saveActive}
		loading={store.saveActive && (store.packHasDestination || packSave.destinationChosen)}
		title={`Save (${modifierLabel}+S)`}>Save</Button
	>
	<Popover align="end" label="Pack actions">
		{#snippet trigger(toggle, open)}
			<button
				onclick={toggle}
				aria-label="More pack actions"
				aria-haspopup="menu"
				aria-expanded={open}
				class="text-muted hover:text-text hover:bg-surface-2 grid h-8 w-8 place-items-center rounded hover:cursor-pointer"
				><Icon src={EllipsisVertical} mini size="18px" /></button
			>
		{/snippet}
		{#snippet children(close)}
			<div class="w-48 py-1">
				<button
					role="menuitem"
					disabled={store.saveActive}
					onclick={() => {
						close();
						onsaveas();
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
							ondiscard();
						}}
						class="w-full px-3 py-2 text-left text-xs text-[var(--ui-danger)] hover:bg-[var(--ui-danger-bg)] disabled:cursor-not-allowed disabled:opacity-40"
						>Discard changes</button
					>{/if}
				<div class="border-border my-1 border-t"></div>
				<button
					role="menuitem"
					onclick={() => {
						close();
						onclosepack();
					}}
					class="w-full px-3 py-2 text-left text-xs {store.packSaved
						? 'hover:bg-bg'
						: 'text-[var(--ui-danger)] hover:bg-[var(--ui-danger-bg)]'}">Close pack</button
				>
			</div>
		{/snippet}
	</Popover>
</header>

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
