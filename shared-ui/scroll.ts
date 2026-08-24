/** Keep a scroll container's offset inside its own content.
 *
 * WebKitGTK — what Tauri renders in on Linux — does not reliably re-clamp `scrollTop` when the
 * content below it shrinks. Collapsing a section (closing the monitor-area editor with Escape, say)
 * leaves the view parked past the end of the new content, showing blank space until something else
 * provokes a scroll. The browser is meant to do this itself, and other engines do, so this is a
 * no-op everywhere it isn't needed rather than a behaviour of our own.
 *
 * Attach it to the element that scrolls, not to the content inside it:
 *
 * ```svelte
 * <div class="flex-1 overflow-y-auto" use:clampScroll>
 * ```
 */
export function clampScroll(node: HTMLElement) {
	const clamp = () => {
		const max = Math.max(0, node.scrollHeight - node.clientHeight);
		if (node.scrollTop > max) node.scrollTop = max;
	};

	// The container's own box is fixed by the layout, so a shrink shows up as its *children*
	// resizing. Both are watched: the container's size changes when the window does, which can
	// strand the offset the same way.
	const sizes = new ResizeObserver(clamp);
	const observeContent = () => {
		sizes.disconnect();
		sizes.observe(node);
		for (const child of node.children) sizes.observe(child);
	};

	// Swapping content wholesale (a Svelte `{#if}` at the top level of a page) replaces the very
	// children being watched, so the set has to be rebuilt when it changes.
	const children = new MutationObserver(observeContent);

	observeContent();
	children.observe(node, { childList: true });

	return {
		destroy() {
			sizes.disconnect();
			children.disconnect();
		}
	};
}

/**
 * Brings `target` into view by scrolling *only* the container it lives in.
 *
 * `Element.scrollIntoView` is specified to scroll every scrollable ancestor, up to and including
 * the document. In a Tauri window that is a trap: the app fills the viewport, so the document is
 * not meant to scroll at all — but WebKitGTK will still scroll it, which drags the whole layout
 * (navigation included) off the top of the window and leaves the rest of it blank. There is no
 * scrollbar to get back with, because the document was never supposed to have one.
 *
 * So the scrolling is done by hand, against one element: the nearest ancestor that actually
 * scrolls. Nothing outside it moves.
 */
export function scrollIntoContainer(
	target: Element,
	{ block = 'center' }: { block?: 'center' | 'nearest' } = {}
): void {
	const container = nearestScrollable(target);
	if (!container) return;

	const targetBox = target.getBoundingClientRect();
	const containerBox = container.getBoundingClientRect();
	// Where the target sits relative to the container's visible box, which is what `scrollTop` is
	// measured against once the container's own offset is added back.
	const offset = targetBox.top - containerBox.top + container.scrollTop;

	let desired: number;
	if (block === 'center') {
		desired = offset - (container.clientHeight - targetBox.height) / 2;
	} else {
		const bottom = offset + targetBox.height - container.clientHeight;
		desired = Math.min(Math.max(container.scrollTop, bottom), offset);
	}

	const max = Math.max(0, container.scrollHeight - container.clientHeight);
	container.scrollTop = Math.min(Math.max(desired, 0), max);
}

/** The nearest ancestor that both can scroll and has something to scroll. */
function nearestScrollable(target: Element): HTMLElement | null {
	let node = target.parentElement;
	while (node) {
		const overflow = getComputedStyle(node).overflowY;
		if (
			(overflow === 'auto' || overflow === 'scroll' || overflow === 'overlay') &&
			node.scrollHeight > node.clientHeight
		) {
			return node;
		}
		node = node.parentElement;
	}
	return null;
}
