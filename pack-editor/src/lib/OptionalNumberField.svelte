<script lang="ts">
	import Checkbox from '$ui/Checkbox.svelte';
	type Props = {
		label: string;
		value: number | null;
		onchange: (v: number | null) => void;
		unit?: string;
		min?: number;
		max?: number;
		step?: number;
		default?: number;
	};

	let { label, value, onchange, unit, min, max, step, default: defaultValue = 1 }: Props = $props();

	const enabled = $derived(value !== null);

	function toggle(checked: boolean) {
		onchange(checked ? (value ?? defaultValue) : null);
	}

	function setValue(raw: string) {
		const n = parseFloat(raw);
		onchange(Number.isFinite(n) ? n : null);
	}
</script>

<label class="optional-number flex min-h-8 items-center gap-2">
	<Checkbox checked={enabled} ariaLabel={label} onchange={toggle} />
	<span class="field-label text-text w-40 shrink-0 text-xs">{label}</span>
	{#if enabled}
		<input
			type="number"
			{value}
			oninput={(e) => setValue(e.currentTarget.value)}
			{min}
			{max}
			{step}
			class="border-border bg-surface text-text h-8 w-24 rounded border px-2 py-0 text-xs
"
		/>
		{#if unit}
			<span class="text-muted text-xs">{unit}</span>
		{/if}
	{/if}
</label>

<style>
	@media (max-width: 560px) {
		.optional-number {
			flex-wrap: wrap;
		}
		.field-label {
			width: calc(100% - 32px);
		}
		.optional-number :global(input) {
			margin-left: 32px;
		}
	}
</style>
