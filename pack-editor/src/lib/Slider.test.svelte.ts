import { describe, expect, it } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import Slider from '$ui/Slider.svelte';

/**
 * The slider holds the thumb where the pointer put it for the duration of a gesture; when the
 * gesture ends, `value` governs again.
 *
 * That contract is why a parent whose write travels over IPC has to hold the value itself until the
 * answer lands — `AudioList` and its `liveVolume`. The slider cannot do it for them: it has no way
 * to know whether a value it kept showing would ever come back, and a stuck thumb is worse than a
 * correction.
 */
/** The painted position, as a fraction. `--ui-slider-fill` is what draws the accent track. */
function fillOf(target: HTMLElement): number {
	const input = target.querySelector('input') as HTMLInputElement;
	return Number.parseFloat(input.style.getPropertyValue('--ui-slider-fill')) / 100;
}

function slider(initial: number) {
	const target = document.createElement('div');
	document.body.appendChild(target);
	const seen: number[] = [];
	const props = $state({
		value: initial,
		min: 0,
		max: 1,
		step: 0.05,
		ariaLabel: 'Volume',
		onchange: (value: number) => {
			seen.push(value);
			props.value = value;
		}
	});
	const app = mount(Slider, { target, props });
	flushSync();
	const input = target.querySelector('input') as HTMLInputElement;
	return {
		seen,
		input,
		target,
		release(to: number) {
			input.valueAsNumber = to;
			input.dispatchEvent(new Event('input', { bubbles: true }));
			flushSync();
			input.dispatchEvent(new Event('change', { bubbles: true }));
			flushSync();
		},
		dispose() {
			unmount(app);
			document.body.removeChild(target);
		}
	};
}

describe('Slider', () => {
	it('leaves the thumb at the value the parent adopted', () => {
		const it_ = slider(0.2);
		it_.release(0.75);
		expect(fillOf(it_.target)).toBe(0.75);
		it_.dispose();
	});

	it('falls back to the parent’s value when the parent does not adopt the change', () => {
		const target = document.createElement('div');
		document.body.appendChild(target);
		// A parent that rejects or clamps must win: the gesture is over, and the slider has no way
		// to know whether a value it is still holding will ever arrive. Holding on would be a stuck
		// thumb, which is worse than a correction.
		const app = mount(Slider, {
			target,
			props: { value: 0.2, min: 0, max: 1, step: 0.05, ariaLabel: 'Volume', onchange: () => {} }
		});
		flushSync();
		const input = target.querySelector('input') as HTMLInputElement;
		input.valueAsNumber = 0.75;
		input.dispatchEvent(new Event('input', { bubbles: true }));
		flushSync();
		input.dispatchEvent(new Event('change', { bubbles: true }));
		flushSync();
		expect(fillOf(target)).toBe(0.2);
		unmount(app);
		document.body.removeChild(target);
	});
});
