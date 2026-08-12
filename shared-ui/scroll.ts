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
