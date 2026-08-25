/**
 * Runs `run` when `value()` becomes something it was not.
 *
 * `$effect` re-runs whenever anything it read is *invalidated*, which is not the same as changed. A
 * prop read out of a fetched object — `stageId={stage.id}` — is invalidated every time the fetch
 * behind it repeats, because the list comes back as new objects saying exactly what the old ones
 * said. An effect that merely reads such a prop therefore fires on every refetch of the query it
 * came from, and where the effect does something the author can see, the whole surface twitches on
 * every edit made to it.
 *
 * That is what this is for: the effect still re-runs, but the side effect is gated on the value.
 */
import { untrack } from 'svelte';

export function onChange<T>(value: () => T, run: (next: T) => void): void {
	let seen: T | undefined;
	let ran = false;
	$effect(() => {
		const next = value();
		if (ran && seen === next) return;
		seen = next;
		ran = true;
		// The side effect reads whatever it likes — a bound element, a store — without any of it
		// becoming a reason to run again.
		untrack(() => run(next));
	});
}
