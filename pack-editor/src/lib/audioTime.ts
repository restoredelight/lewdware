export function audioDuration(value: number): number {
	return Number.isFinite(value) && value > 0 ? value : 0;
}

export function clampAudioPosition(value: number, duration: number): number {
	if (!Number.isFinite(value)) return 0;
	return Math.min(audioDuration(duration), Math.max(0, value));
}
