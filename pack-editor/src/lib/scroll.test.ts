import { describe, expect, it } from 'vitest';
import { scrollIntoContainer } from '$ui/scroll';

/**
 * Builds a scroll container with a target inside it, and an outer element that also *looks*
 * scrollable — which is the whole point. `scrollIntoView` scrolls every scrollable ancestor up to
 * the document; this helper must move exactly one.
 *
 * happy-dom does no layout, so the geometry is stated rather than measured. That is fine for what
 * is under test: which element gets scrolled, and to what offset.
 */
function scene(options: {
	targetTop: number;
	targetHeight: number;
	containerHeight: number;
	contentHeight: number;
	scrollTop?: number;
	containerOverflow?: string;
}) {
	const outer = document.createElement('div');
	const container = document.createElement('div');
	const target = document.createElement('div');
	container.appendChild(target);
	outer.appendChild(container);
	document.body.appendChild(outer);

	geometry(outer, {
		overflowY: 'auto',
		clientHeight: 200,
		scrollHeight: 10_000,
		rectTop: 0,
		rectHeight: 200
	});
	geometry(container, {
		overflowY: options.containerOverflow ?? 'auto',
		clientHeight: options.containerHeight,
		scrollHeight: options.contentHeight,
		rectTop: 0,
		rectHeight: options.containerHeight
	});
	geometry(target, {
		overflowY: 'visible',
		clientHeight: options.targetHeight,
		scrollHeight: options.targetHeight,
		rectTop: options.targetTop,
		rectHeight: options.targetHeight
	});
	container.scrollTop = options.scrollTop ?? 0;
	outer.scrollTop = 0;
	return { outer, container, target };
}

function geometry(
	node: HTMLElement,
	values: {
		overflowY: string;
		clientHeight: number;
		scrollHeight: number;
		rectTop: number;
		rectHeight: number;
	}
) {
	node.style.overflowY = values.overflowY;
	Object.defineProperty(node, 'clientHeight', { value: values.clientHeight, configurable: true });
	Object.defineProperty(node, 'scrollHeight', { value: values.scrollHeight, configurable: true });
	node.getBoundingClientRect = () =>
		({ top: values.rectTop, height: values.rectHeight }) as DOMRect;
}

describe('scrolling one container', () => {
	// The bug this exists for: `scrollIntoView` drags the document with it, which in a Tauri window
	// takes the navigation off the top of the screen and leaves no scrollbar to come back with.
	it('moves the nearest scroll container and nothing above it', () => {
		const { outer, container, target } = scene({
			targetTop: 900,
			targetHeight: 100,
			containerHeight: 400,
			contentHeight: 5000
		});

		scrollIntoContainer(target, { block: 'center' });

		expect(container.scrollTop).toBeGreaterThan(0);
		expect(outer.scrollTop, 'nothing outside the container may move').toBe(0);
		expect(document.documentElement.scrollTop).toBe(0);
	});

	it('centres the target in the container', () => {
		const { container, target } = scene({
			targetTop: 900,
			targetHeight: 100,
			containerHeight: 400,
			contentHeight: 5000
		});

		scrollIntoContainer(target, { block: 'center' });

		// 900 from the container's top, minus half the leftover height (400 - 100) / 2.
		expect(container.scrollTop).toBe(750);
	});

	it('never scrolls past the end of the content', () => {
		const { container, target } = scene({
			targetTop: 4900,
			targetHeight: 100,
			containerHeight: 400,
			contentHeight: 5000
		});

		scrollIntoContainer(target, { block: 'center' });

		// The blank-space failure: an offset past `scrollHeight - clientHeight` parks the view
		// below the content, which is exactly what the author saw.
		expect(container.scrollTop).toBe(4600);
	});

	it('never scrolls above the start of the content', () => {
		const { container, target } = scene({
			targetTop: 0,
			targetHeight: 100,
			containerHeight: 400,
			contentHeight: 5000
		});

		scrollIntoContainer(target, { block: 'center' });

		expect(container.scrollTop).toBe(0);
	});

	it('leaves a target already in view alone when asked for the nearest position', () => {
		const { container, target } = scene({
			targetTop: 100,
			targetHeight: 100,
			containerHeight: 400,
			contentHeight: 5000,
			scrollTop: 50
		});

		scrollIntoContainer(target, { block: 'nearest' });

		expect(container.scrollTop).toBe(50);
	});

	// An element with no scrolling ancestor has nothing to be brought into view *of*.
	it('does nothing when nothing around it scrolls', () => {
		const { outer, container, target } = scene({
			targetTop: 900,
			targetHeight: 100,
			containerHeight: 400,
			contentHeight: 5000,
			containerOverflow: 'visible'
		});
		Object.defineProperty(outer, 'scrollHeight', { value: 200, configurable: true });

		scrollIntoContainer(target, { block: 'center' });

		expect(container.scrollTop).toBe(0);
		expect(outer.scrollTop).toBe(0);
	});
});
