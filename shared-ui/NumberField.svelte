<script lang="ts">
	/**
	 * A field holding a number, or nothing.
	 *
	 * `Field` can render `type="number"` -- and four of its props (`min`, `max`, `step`, `suffix`)
	 * mean nothing unless it does -- but it reports what was typed as a **string**, which leaves
	 * every caller to turn that back into a number itself. Across the two apps that was being done
	 * three different ways, and one of them is a trap: `Number('')` is `0`, not `NaN`, so a guard
	 * written as `Number.isFinite(Number(raw))` accepts an *empty field* as a real zero. That is how
	 * clearing a speed writes `0` rather than clearing it.
	 *
	 * So the parse lives here, once, and this component reports `number | null` instead of a string.
	 * `null` means the field is empty or holds something that is not a number -- the two cases a
	 * caller has to tell apart from a genuine `0`, and cannot once it has been through `Number()`.
	 *
	 * What `null` *means* is deliberately the caller's: for most fields it clears the value ("no
	 * opinion", the sparse-row rule -- see `mediaAttributes.ts`), but for a few it means "keep what
	 * was there", and that is a decision about the field, not about parsing. Either way it is now
	 * one visible line at the call site rather than an implicit consequence of `Number()`.
	 *
	 * The rendering is `Field`'s, so the two stay identical by construction rather than by care.
	 */
	import Field from './Field.svelte';

	type Props = {
		label: string;
		/** The number to show, or null for an empty field. */
		value: number | null | undefined;
		description?: string;
		error?: string;
		placeholder?: string;
		disabled?: boolean;
		min?: number;
		max?: number;
		step?: number;
		size?: 'compact' | 'normal';
		hideLabel?: boolean;
		/** A unit drawn inside the field's right edge: `%`, `px`, `s`, `×`. */
		suffix?: string;
		class?: string;
		/** Fires per keystroke -- for a value that is debounced on its way to being stored. */
		oninput?: (value: number | null) => void;
		/** Fires once the edit is committed (blur, Enter) -- for a value stored outright. */
		onchange?: (value: number | null) => void;
	};

	let {
		label,
		value,
		description,
		error,
		placeholder,
		disabled = false,
		min,
		max,
		step,
		size = 'normal',
		hideLabel = false,
		suffix,
		class: className,
		oninput,
		onchange
	}: Props = $props();

	/** The one place a typed string becomes a number. Empty and unparseable both answer null. */
	function parse(raw: string): number | null {
		const trimmed = raw.trim();
		if (trimmed === '') return null;
		const parsed = Number(trimmed);
		return Number.isFinite(parsed) ? parsed : null;
	}
</script>

<Field
	{label}
	type="number"
	value={value ?? ''}
	{description}
	{error}
	{placeholder}
	{disabled}
	{min}
	{max}
	{step}
	{size}
	{hideLabel}
	{suffix}
	class={className}
	oninput={oninput && ((raw) => oninput(parse(raw)))}
	onchange={onchange && ((raw) => onchange(parse(raw)))}
/>
