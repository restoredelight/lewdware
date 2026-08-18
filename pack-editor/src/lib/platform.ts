/**
 * The modifier key this platform names in a shortcut hint: ⌘ on a Mac, Ctrl everywhere else.
 *
 * Read at call time rather than at module scope, because both callers set it inside `onMount`:
 * `navigator` does not exist while the app is prerendered, and a module-level constant would be
 * evaluated then.
 */
export function modifierKeyLabel(): string {
	return navigator.platform.includes('Mac') ? '⌘' : 'Ctrl';
}
