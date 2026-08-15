// Human-readable durations and sizes. One copy, because the same file's duration and size are
// shown in more than one place at once -- a row in the audio list and the inspector beside it --
// and two formatters that disagree on precision make the same number look like two numbers.

/** `m:ss`, or `h:mm:ss` past an hour. Non-finite and negative inputs read as `0:00`. */
export function formatDuration(value: number): string {
	const seconds = Math.floor(Math.max(0, Number.isFinite(value) ? value : 0));
	const hours = Math.floor(seconds / 3600);
	const minutes = Math.floor((seconds % 3600) / 60);
	const remainder = seconds % 60;
	return hours > 0
		? `${hours}:${String(minutes).padStart(2, '0')}:${String(remainder).padStart(2, '0')}`
		: `${minutes}:${String(remainder).padStart(2, '0')}`;
}

export function formatFileSize(bytes: number): string {
	if (bytes < 1024) return `${bytes} B`;
	const units = ['KB', 'MB', 'GB'];
	let value = bytes;
	let unit = -1;
	do {
		value /= 1024;
		unit++;
	} while (value >= 1024 && unit < units.length - 1);
	return `${value.toFixed(value < 10 ? 2 : 1)} ${units[unit]}`;
}
