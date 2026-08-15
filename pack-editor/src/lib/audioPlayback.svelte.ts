import { audioDuration } from './audioTime.js';

/**
 * One `<audio>` element behind the whole audio list.
 *
 * The rows are virtualized, so a row scrolled out of view is unmounted -- an element owned by the
 * row would take the playback with it mid-track, and a pack with three hundred sounds would build
 * three hundred media elements to play at most one. The element lives here instead and the rows
 * are transports over it, which also *is* the list's "starting a row stops the one before it" rule
 * rather than something that has to be arranged: loading a new source ends the previous playback.
 *
 * Mute is a property of the element, so it is the list's rather than any one row's -- which is the
 * behaviour that makes sense for a control there is only one of.
 */
class AudioPlayback {
	/** The media the element is loaded with, playing or paused. */
	activeId = $state<number | null>(null);
	playing = $state(false);
	position = $state(0);
	/** What the element itself reports, once it knows; 0 until then, so rows fall back to the
	 * duration the pack recorded rather than showing an empty player. */
	measured = $state(0);
	muted = $state(false);
	/** The one media that failed, so a broken file marks itself rather than the whole list. */
	failedId = $state<number | null>(null);

	#element: HTMLAudioElement | null = null;

	// Built on first use, never at module scope: this module is imported during prerendering, where
	// there is no `Audio`.
	#audio(): HTMLAudioElement {
		if (this.#element) return this.#element;
		const element = new Audio();
		element.preload = 'none';
		element.addEventListener('play', () => (this.playing = true));
		element.addEventListener('pause', () => (this.playing = false));
		element.addEventListener('ended', () => {
			this.playing = false;
			this.position = 0;
		});
		element.addEventListener('timeupdate', () => (this.position = element.currentTime));
		element.addEventListener('durationchange', () => {
			this.measured = audioDuration(element.duration);
		});
		element.addEventListener('error', () => {
			this.failedId = this.activeId;
			this.playing = false;
		});
		this.#element = element;
		return element;
	}

	#load(id: number, src: string): HTMLAudioElement {
		const element = this.#audio();
		if (this.activeId !== id) {
			element.src = src;
			this.activeId = id;
			this.position = 0;
			this.measured = 0;
			this.failedId = null;
		}
		return element;
	}

	async toggle(id: number, src: string) {
		const element = this.#load(id, src);
		if (!element.paused) {
			element.pause();
			return;
		}
		this.failedId = null;
		try {
			await element.play();
		} catch {
			this.failedId = id;
		}
	}

	/**
	 * Moves to `seconds` in `id`, loading it first if another row was playing.
	 *
	 * Seeking a row that isn't playing is how you start one part-way through: the element takes the
	 * position now and honours it when play begins, which is what `preload="none"` is for.
	 */
	seek(id: number, src: string, seconds: number) {
		const element = this.#load(id, src);
		element.currentTime = seconds;
		this.position = element.currentTime;
	}

	setMuted(value: boolean) {
		this.#audio().muted = value;
		this.muted = value;
	}

	/** Leaves the list. The element is kept, muted and all, for the next visit. */
	stop() {
		this.#element?.pause();
		this.activeId = null;
		this.playing = false;
		this.position = 0;
		this.measured = 0;
		this.failedId = null;
	}
}

export const playback = new AudioPlayback();
