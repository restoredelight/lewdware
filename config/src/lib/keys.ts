import type { Key } from './types';

/** Keys that only ever qualify another key, so recording one on its own is meaningless. */
export const MODIFIER_KEYS = new Set(['Control', 'Alt', 'Shift', 'Meta', 'Super', 'Hyper']);

/** `separator` tightens to `'+'` for the sidebar readout, where 168px of mono has to hold a
 * three-modifier combo without ellipsizing. Anywhere with room uses the spaced default. */
export function formatKey(key: Key, separator = ' + '): string {
	const parts: string[] = [];
	if (key.modifiers.ctrl) parts.push('Ctrl');
	if (key.modifiers.alt) parts.push('Alt');
	if (key.modifiers.shift) parts.push('Shift');
	if (key.modifiers.meta) parts.push('Meta');
	parts.push(key.name);
	return parts.join(separator);
}
