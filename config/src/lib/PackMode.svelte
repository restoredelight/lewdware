<script lang="ts">
	import { store } from './store.svelte';
	import type {
		ModeOptionDto,
		OptionEntryDto,
		OptionGroupEntryDto,
		OptionType,
		Permission,
		ShowWhen
	} from './types';
	import { ArrowUpTray, Check, ChevronRight, FolderOpen, Icon, XMark } from 'svelte-hero-icons';
	import Slider from '$ui/Slider.svelte';
	import Toggle from '$ui/Toggle.svelte';
	import Select from '$ui/Select.svelte';
	import Button from '$ui/Button.svelte';
	import Card from '$ui/Card.svelte';
	import Dialog from '$ui/Dialog.svelte';
	import IconButton from '$ui/IconButton.svelte';

	type Removal = { kind: 'pack' } | { kind: 'mode'; path: string; name: string };
	let pendingRemoval = $state<Removal | null>(null);

	function fileName(path: string): string {
		return path.split(/[\\/]/).filter(Boolean).at(-1) ?? path;
	}

	function confirmRemoval() {
		const removal = pendingRemoval;
		pendingRemoval = null;
		if (!removal) return;
		if (removal.kind === 'pack') void store.removePack();
		else void store.removeUploadedMode(removal.path);
	}

	const modes = $derived(
		store.modeGroups.flatMap((group) =>
			group.entries.map((entry) => ({ entry, source: group.source, sourceLabel: group.label }))
		)
	);

	function sourceName(source: 'pack' | 'uploaded' | 'builtin'): string {
		if (source === 'pack') return 'Pack';
		if (source === 'uploaded') return 'Uploaded';
		return 'Built-in';
	}

	function sourceClass(source: 'pack' | 'uploaded' | 'builtin'): string {
		if (source === 'pack')
			return 'border-[var(--ui-info-border)] bg-[var(--ui-info-bg)] text-[var(--ui-info)]';
		if (source === 'uploaded')
			return 'border-[var(--ui-warning-border)] bg-[var(--ui-warning-bg)] text-[var(--ui-warning)]';
		return 'border-border bg-bg text-muted';
	}

	function optionTypeKey(opt: ModeOptionDto): string {
		return Object.keys(opt.option_type)[0];
	}

	function optionTypeValue(opt: ModeOptionDto): OptionType[keyof OptionType] {
		const key = optionTypeKey(opt) as keyof OptionType;
		return (opt.option_type as Record<string, unknown>)[key] as OptionType[keyof OptionType];
	}

	function isSlider(opt: ModeOptionDto): boolean {
		const tv = optionTypeValue(opt) as Record<string, unknown>;
		return !!tv?.slider;
	}

	function getMin(opt: ModeOptionDto): number | undefined {
		const tv = optionTypeValue(opt) as Record<string, unknown>;
		return tv?.min as number | undefined;
	}

	function getMax(opt: ModeOptionDto): number | undefined {
		const tv = optionTypeValue(opt) as Record<string, unknown>;
		return tv?.max as number | undefined;
	}

	function getStep(opt: ModeOptionDto): number | undefined {
		const tv = optionTypeValue(opt) as Record<string, unknown>;
		return tv?.step as number | undefined;
	}

	function enumValues(opt: ModeOptionDto): Record<string, string> {
		const tv = optionTypeValue(opt) as Record<string, unknown>;
		return (tv?.values ?? {}) as Record<string, string>;
	}

	function roundToStep(value: number, step: number): number {
		if (step <= 0) return value;
		const snapped = Math.round(value / step) * step;
		const decimals = Math.max(0, -Math.floor(Math.log10(step)));
		return parseFloat(snapped.toFixed(decimals));
	}

	function clampValue(value: number, opt: ModeOptionDto): number {
		const tv = optionTypeValue(opt) as Record<string, unknown>;
		if (!tv?.clamp) return value;
		const min = tv.min as number | null;
		const max = tv.max as number | null;
		if (min !== null && min !== undefined && value < min) return min;
		if (max !== null && max !== undefined && value > max) return max;
		return value;
	}

	// When an optional slider is disabled (value=null), fall back to the last known
	// value so thumb and track stay in sync rather than both snapping to 0/midpoint.
	function sliderDisplayValue(opt: ModeOptionDto): number {
		if (opt.value !== null && typeof opt.value === 'number') return opt.value;
		const fallback = lastValues.get(opt.key) ?? getInitialValue(opt);
		return typeof fallback === 'number' ? fallback : 0;
	}

	function handleNumberInput(opt: ModeOptionDto, raw: string) {
		const n = parseFloat(raw);
		if (isNaN(n)) return;
		const step = getStep(opt);
		const stepped = step != null ? roundToStep(n, step) : n;
		const clamped = clampValue(stepped, opt);
		store.setModeOption(opt.key, clamped);
	}

	// Tracks the last non-null value for optional options so we can restore on re-enable.
	const lastValues = new Map<string, number | string | boolean>();

	function getInitialValue(opt: ModeOptionDto): number | string | boolean {
		const typeKey = optionTypeKey(opt);
		const tv = optionTypeValue(opt) as Record<string, unknown>;
		const def = tv?.default;
		if (def !== null && def !== undefined) return def as number | string | boolean;
		// Fallback: should not be reached for well-formed configs
		if (typeKey === 'Integer' || typeKey === 'Number') return (tv?.min as number) ?? 0;
		if (typeKey === 'Boolean') return true;
		if (typeKey === 'Enum')
			return Object.keys((tv?.values as Record<string, string>) ?? {})[0] ?? '';
		return '';
	}

	function handleOptionalToggle(opt: ModeOptionDto, enabled: boolean) {
		if (enabled) {
			const restored = lastValues.get(opt.key) ?? getInitialValue(opt);
			store.setModeOption(opt.key, restored);
		} else {
			if (opt.value !== null) {
				lastValues.set(opt.key, opt.value as number | string | boolean);
			}
			store.setModeOption(opt.key, null);
		}
	}

	// Flat map of option key → current value, used to evaluate show_when conditions. Seeded with the
	// pack-derived `pack_has_*` facts, which a `show_when` can reference but which are not options
	// and so never appear in the entry tree.
	const valueMap = $derived.by(() => {
		const map = new Map<string, unknown>(Object.entries(store.packHas));
		function collect(entries: OptionEntryDto[]) {
			for (const entry of entries) {
				if (entry.kind === 'Option') {
					map.set(entry.key, entry.value);
				} else {
					collect(entry.entries);
				}
			}
		}
		collect(store.modeOptions);
		return map;
	});

	// ─── Declared permissions ──────────────────────────────────────────────────
	// A mode's schema can say which of the user's permissions an option or group actually uses
	// (`needs_permissions` in config.jsonc). It grants nothing -- a denied permission still makes the
	// call no-op engine-side. Rather than let the user set an option that can't take effect, we
	// grey it out and say, quietly, which permission it's waiting on, with a link to go grant it.

	const permissionLabels: Record<Permission, string> = {
		set_wallpaper: 'Change wallpaper',
		open_links: 'Open links',
		send_notifications: 'Show notifications'
	};

	function isGranted(permission: Permission): boolean {
		return store.config?.capabilities[permission] ?? false;
	}

	// Callers only ever render visible entries, so `show_when` visibility is already accounted for.
	// Unlike the value-gated version this replaced, an off toggle still reports its requirement --
	// being unable to turn a feature *on* because its permission is denied is the whole point.
	function unmetForOption(opt: ModeOptionDto): Permission[] {
		return opt.needs_permissions.filter((p) => !isGranted(p));
	}

	function unmetForGroup(group: OptionGroupEntryDto): Permission[] {
		return group.needs_permissions.filter((p) => !isGranted(p));
	}

	const unmetForMode = $derived(store.modeNeedsPermissions.filter((p) => !isGranted(p)));

	function permissionSentence(permissions: Permission[]): string {
		const names = permissions.map((p) => `“${permissionLabels[p]}”`);
		if (names.length === 1) return names[0];
		return `${names.slice(0, -1).join(', ')} and ${names.at(-1)}`;
	}

	function isVisible(showWhen: ShowWhen | null): boolean {
		if (!showWhen) return true;
		for (const [key, expected] of Object.entries(showWhen)) {
			const actual = valueMap.get(key);
			if (actual !== expected) return false;
		}
		return true;
	}

	type EntryChunk =
		| { kind: 'options'; items: (ModeOptionDto & { kind: 'Option' })[] }
		| { kind: 'group'; group: OptionGroupEntryDto };

	// Consecutive options share one card (Permissions-style rows); groups get their own.
	function chunkEntries(entries: OptionEntryDto[]): EntryChunk[] {
		const chunks: EntryChunk[] = [];
		for (const entry of entries) {
			if (entry.kind === 'Option') {
				const last = chunks.at(-1);
				if (last?.kind === 'options') last.items.push(entry);
				else chunks.push({ kind: 'options', items: [entry] });
			} else {
				chunks.push({ kind: 'group', group: entry });
			}
		}
		return chunks;
	}

	// Keys of groups the user has manually collapsed (groups start open).
	const collapsedGroups = new Set<string>();
	let collapsedGroupsVersion = $state(0);

	function toggleGroup(key: string) {
		if (collapsedGroups.has(key)) {
			collapsedGroups.delete(key);
		} else {
			collapsedGroups.add(key);
		}
		collapsedGroupsVersion += 1;
	}

	function isCollapsed(key: string) {
		collapsedGroupsVersion; // reactive dependency
		return collapsedGroups.has(key);
	}
</script>

{#snippet permissionHint(permissions: Permission[])}
	<!-- Deliberately quiet: muted, small, no warning colour. It states a fact and offers a route,
       it doesn't sound an alarm. "Behaviour" jumps to that page (no router; just a tab swap). -->
	<p class="text-muted m-0 text-[11px] leading-snug">
		Requires the {permissionSentence(permissions)}
		{permissions.length > 1 ? 'permissions' : 'permission'} ·
		<button
			type="button"
			class="hover:text-text underline decoration-dotted underline-offset-2 transition-colors"
			onclick={() => (store.activeTab = 'behaviour')}>Behaviour</button
		>
	</p>
{/snippet}

{#snippet optionControl(opt: ModeOptionDto, disabled: boolean = false)}
	{@const typeKey = optionTypeKey(opt)}

	{#if typeKey === 'Boolean'}
		<Toggle
			ariaLabel={opt.label}
			checked={opt.value === true}
			{disabled}
			onchange={(checked) => store.setModeOption(opt.key, checked)}
		/>
	{:else if typeKey === 'String'}
		<input
			type="text"
			value={opt.value as string}
			{disabled}
			oninput={(e) => store.setModeOption(opt.key, e.currentTarget.value)}
			class="border-border bg-bg text-text w-56 rounded-sm border px-2.5 py-1.5 text-sm transition-colors hover:border-[var(--ui-border-strong)] disabled:cursor-not-allowed disabled:opacity-50"
		/>
	{:else if typeKey === 'Enum'}
		<Select
			class="w-56"
			size="compact"
			hideLabel
			label={opt.label}
			value={opt.value as string}
			{disabled}
			options={Object.entries(enumValues(opt)).map(([value, label]) => ({ value, label }))}
			onchange={(value) => store.setModeOption(opt.key, value)}
		/>
	{:else if typeKey === 'Integer' || typeKey === 'Number'}
		{#if isSlider(opt)}
			{@const displayVal = sliderDisplayValue(opt)}
			<Slider
				ariaLabel={opt.label}
				min={getMin(opt) ?? 0}
				max={getMax(opt) ?? 100}
				step={getStep(opt) ?? 1}
				value={displayVal}
				{disabled}
				oninput={(value) => handleNumberInput(opt, String(value))}
				class="w-40 sm:w-52"
			/>
			<input
				type="number"
				value={opt.value as number}
				min={getMin(opt)}
				max={getMax(opt)}
				step={getStep(opt)}
				{disabled}
				oninput={(e) => handleNumberInput(opt, e.currentTarget.value)}
				class="border-border bg-bg text-text w-20 rounded-sm border px-2.5 py-1.5 text-sm transition-colors hover:border-[var(--ui-border-strong)] disabled:cursor-not-allowed disabled:opacity-50"
			/>
		{:else}
			<input
				type="number"
				value={opt.value as number}
				min={getMin(opt)}
				max={getMax(opt)}
				step={getStep(opt)}
				{disabled}
				oninput={(e) => handleNumberInput(opt, e.currentTarget.value)}
				class="border-border bg-bg text-text w-24 rounded-sm border px-2.5 py-1.5 text-sm transition-colors hover:border-[var(--ui-border-strong)] disabled:cursor-not-allowed disabled:opacity-50"
			/>
		{/if}
	{/if}
{/snippet}

<div class="flex-1 overflow-y-auto">
	<div class="mx-auto flex w-full max-w-4xl flex-col gap-6 p-8">
		<header class="max-w-2xl">
			<h1 class="ui-page-title">Pack &amp; mode</h1>
			<p class="text-muted mt-1.5 mb-0 text-sm">
				Choose the pack Lewdware uses, then select and configure how it behaves.
			</p>
		</header>

		<!-- Pack picker -->
		<section class="flex flex-col gap-3">
			<div>
				<h2 class="ui-section-title">Pack</h2>
				<p class="text-muted mt-1 mb-0 text-xs">Contains media, captions, prompts, etc.</p>
			</div>
			{#if store.config?.pack_path}
				<Card class="flex items-center gap-4 p-4">
					<div
						class="bg-accent/10 text-accent-foreground grid h-10 w-10 shrink-0 place-items-center rounded-md"
					>
						<span class="h-5 w-5"><Icon src={FolderOpen} /></span>
					</div>
					<div class="min-w-0 flex-1">
						<p class="text-text m-0 truncate text-sm font-semibold">
							{fileName(store.config.pack_path)}
						</p>
						<p class="text-muted m-0 mt-0.5 truncate text-xs" title={store.config.pack_path}>
							{store.config.pack_path}
						</p>
					</div>
					<div class="flex shrink-0 items-center gap-2">
						<Button
							size="compact"
							variant="destructive"
							disabled={store.isBusy('pack')}
							onclick={() => (pendingRemoval = { kind: 'pack' })}>Remove</Button
						>
						<!-- Not primary: Launch owns the carmine fill in the sidebar, and with a pack already
						     selected it is the live next step, not this. -->
						<Button
							size="compact"
							variant="secondary"
							loading={store.isBusy('pack')}
							onclick={() => store.pickPack()}>Change pack…</Button
						>
					</div>
				</Card>
			{:else}
				<Card
					class="flex flex-col items-center border-dashed !border-[var(--ui-border-strong)] p-7 text-center"
				>
					<h3 class="text-text m-0 text-sm font-semibold">No pack selected</h3>
					<p class="text-muted mt-1 mb-4 max-w-md text-xs leading-relaxed">
						Choose a .lwpack file before launching Lewdware.
					</p>
					<Button
						size="compact"
						variant="primary"
						loading={store.isBusy('pack')}
						onclick={() => store.pickPack()}>Choose pack…</Button
					>
				</Card>
			{/if}
		</section>

		<!-- Mode selector -->
		<section class="border-border flex flex-col gap-3 border-t pt-6">
			<div class="flex items-start justify-between gap-4">
				<div>
					<h2 class="ui-section-title">Mode</h2>
					<p class="text-muted mt-1 mb-0 text-xs">Changes the behaviour of Lewdware.</p>
				</div>
				<Button
					size="compact"
					variant="secondary"
					loading={store.isBusy('mode')}
					onclick={() => store.uploadMode()}
				>
					<span class="h-4 w-4"><Icon src={ArrowUpTray} mini /></span> Upload mode
				</Button>
			</div>

			<div role="radiogroup" aria-label="Mode">
				<Card class="max-h-96 overflow-y-auto p-2">
					<div class="flex flex-col gap-1">
						{#each modes as mode (JSON.stringify(mode.entry.id))}
							{@const selected = store.isModeSelected(mode.entry.id)}
							<div class="flex items-center gap-1">
								<button
									onclick={() => store.setMode(mode.entry.id)}
									disabled={store.isBusy('mode')}
									role="radio"
									aria-checked={selected}
									class="text-text flex min-h-10 flex-1 cursor-pointer items-center gap-3 rounded-md px-3 py-2 text-left
                   text-sm transition-colors disabled:cursor-not-allowed
                   {selected
										? 'bg-surface-2 font-medium shadow-[inset_2px_0_0_var(--ui-accent-hover)]'
										: 'hover:bg-surface-2'}"
								>
									<span
										class="grid h-4 w-4 shrink-0 place-items-center rounded-full border {selected
											? 'border-accent bg-accent'
											: 'border-border-strong'}"
									>
										{#if selected}<span class="h-2 w-2 text-white"><Icon src={Check} mini /></span
											>{/if}
									</span>
									<span class="min-w-0 flex-1 truncate">{mode.entry.name}</span>
									<span
										class="shrink-0 rounded-full border px-2 py-0.5 text-[10px] leading-4 font-semibold {sourceClass(
											mode.source
										)}"
										title={mode.source === 'pack' ? mode.sourceLabel : undefined}
										>{sourceName(mode.source)}</span
									>
								</button>
								{#if mode.entry.id.type === 'File'}
									<IconButton
										label={`Remove ${mode.entry.name}`}
										variant="destructive"
										disabled={store.isBusy('mode')}
										onclick={() =>
											(pendingRemoval = {
												kind: 'mode',
												path: (mode.entry.id as Extract<typeof mode.entry.id, { type: 'File' }>)
													.path,
												name: mode.entry.name
											})}
									>
										<span class="block h-4 w-4"><Icon src={XMark} mini /></span>
									</IconButton>
								{/if}
							</div>
						{:else}
							<p class="text-muted m-0 px-3 py-4 text-center text-xs">No modes are available.</p>
						{/each}
					</div>
				</Card>
			</div>
		</section>

		<!-- Mode options -->
		{#if store.modeOptions.length > 0}
			<section class="border-border flex flex-col gap-3 border-t pt-6">
				<div>
					<h2 class="ui-section-title">Mode options</h2>
					<!-- <p class="mt-1 mb-0 text-xs text-muted">Customize the selected mode.</p> -->
				</div>

				<!-- Permissions this mode uses no matter how it's configured; they belong to no one option,
           so there's no single control to disable -- just the note. -->
				{#if unmetForMode.length > 0}
					{@render permissionHint(unmetForMode)}
				{/if}

				<div class="flex flex-col gap-3">
					{@render optionEntries(store.modeOptions)}
				</div>
			</section>
		{/if}
	</div>
</div>

{#if pendingRemoval}
	<Dialog
		title={pendingRemoval.kind === 'pack' ? 'Remove pack?' : `Remove “${pendingRemoval.name}”?`}
		description={pendingRemoval.kind === 'pack'
			? 'Lewdware cannot launch until another pack is selected. Your pack file will not be deleted.'
			: 'This removes the uploaded mode from Lewdware. If it is selected, Lewdware will switch to a built-in mode.'}
		buttons={[
			{ label: 'Cancel', onclick: () => (pendingRemoval = null) },
			{ label: 'Remove', destructive: true, onclick: confirmRemoval }
		]}
		onclose={() => (pendingRemoval = null)}
	/>
{/if}

{#snippet optionRow(opt: ModeOptionDto, inheritedDisabled: boolean = false)}
	{@const typeKey = optionTypeKey(opt)}
	{@const optionalOff = opt.optional && opt.value === null}
	{@const own = unmetForOption(opt)}
	{@const permDisabled = inheritedDisabled || own.length > 0}
	<div class="flex flex-col gap-1.5 px-4 py-3">
		<!-- Hint above the row, and only for the option's *own* requirement -- when a group is what's
         unmet, the group shows one hint rather than repeating it on every child. -->
		{#if own.length > 0}{@render permissionHint(own)}{/if}
		<div
			class="flex flex-wrap items-center justify-between gap-x-6 gap-y-2 {permDisabled
				? 'opacity-50'
				: ''}"
		>
			<div class="min-w-0 flex-1 basis-52">
				<h3 class="text-text m-0 text-sm font-medium">{opt.label}</h3>
				{#if opt.description}<p class="text-muted m-0 mt-0.5 text-xs">{opt.description}</p>{/if}
			</div>
			{#if typeKey === 'Boolean' && !opt.optional}
				{@render optionControl(opt, permDisabled)}
			{:else}
				<div class="flex shrink-0 items-center gap-3">
					<!-- A disabled optional renders no control at all — the toggle alone says "off". -->
					{#if !optionalOff}{@render optionControl(opt, permDisabled)}{/if}
					{#if opt.optional}
						<Toggle
							ariaLabel={`Enable ${opt.label}`}
							checked={!optionalOff}
							disabled={permDisabled}
							onchange={(checked) => handleOptionalToggle(opt, checked)}
						/>
					{/if}
				</div>
			{/if}
		</div>
	</div>
{/snippet}

{#snippet optionGroup(group: OptionGroupEntryDto, inheritedDisabled: boolean = false)}
	{@const collapsed = isCollapsed(group.key)}
	{@const unmet = unmetForGroup(group)}
	{@const childrenDisabled = inheritedDisabled || unmet.length > 0}
	<Card class="flex flex-col gap-0">
		<button
			onclick={() => toggleGroup(group.key)}
			aria-expanded={!collapsed}
			class="text-text hover:bg-surface-2 flex items-center gap-2 rounded-md px-4 py-3 text-left
             text-sm font-semibold transition-colors"
		>
			<span class="text-xs transition-transform" class:rotate-90={!collapsed}>
				<Icon src={ChevronRight} solid class="h-4"></Icon>
			</span>
			<span class="flex flex-col gap-0.5">
				<span>{group.label}</span>
				{#if group.description}<span class="text-muted text-xs font-normal"
						>{group.description}</span
					>{/if}
			</span>
		</button>

		<!-- One hint at the top of the group; every control inside is greyed out and disabled. Shown
         even while collapsed, so the state is legible without expanding. The header stays live so
         the group can still be collapsed. -->
		{#if unmet.length > 0}
			<div class="px-4 pb-2">{@render permissionHint(unmet)}</div>
		{/if}

		{#if !collapsed}
			<div class="border-border border-t">
				{@render optionEntries(group.entries, true, childrenDisabled)}
			</div>
		{/if}
	</Card>
{/snippet}

{#snippet optionEntries(
	entries: OptionEntryDto[],
	bare: boolean = false,
	inheritedDisabled: boolean = false
)}
	{@const chunks = chunkEntries(entries.filter((entry) => isVisible(entry.show_when)))}
	{#each chunks as chunk, index (chunk.kind === 'group' ? `group:${chunk.group.key}` : `options:${chunk.items[0].key}`)}
		{#if chunk.kind === 'options'}
			<div
				class="divide-border divide-y {bare ? '' : 'border-border bg-surface rounded-md border'}"
			>
				{#each chunk.items as opt (opt.key)}
					{@render optionRow(opt, inheritedDisabled)}
				{/each}
			</div>
		{:else if bare}
			<div class="border-border border-t p-3 {index === 0 ? 'border-t-0' : ''}">
				{@render optionGroup(chunk.group, inheritedDisabled)}
			</div>
		{:else}
			{@render optionGroup(chunk.group, inheritedDisabled)}
		{/if}
	{/each}
{/snippet}
