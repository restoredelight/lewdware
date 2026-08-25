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
 * behaviour that makes sense for a control there is only one of. The pack's own per-file level is
 * the opposite -- it belongs to the media, not to the transport -- so the list pushes it in with
 * `setLevel` as the active row changes; see `level`.
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
	/**
	 * The active media's own level from the pack, as the element is currently playing it.
	 *
	 * Held here rather than read from the attributes each time the element is touched, because the
	 * list is the only thing that knows it: only one file's attributes are fetched at a time, so
	 * the level for a row arrives after the row is already loaded. Reset to full on a change of
	 * media for exactly that reason -- a new row is played at the level a file with no opinion is
	 * played at until its own arrives, rather than inheriting the level of the row before it.
	 */
	level = $state(1);
	/** The one media that failed, so a broken file marks itself rather than the whole list. */
	failedId = $state<number | null>(null);

	#element: HTMLAudioElement | null = null;
	#frame: number | null = null;

	// Built on first use, never at module scope: this module is imported during prerendering, where
	// there is no `Audio`.
	#audio(): HTMLAudioElement {
		if (this.#element) return this.#element;
		const element = new Audio();
		element.preload = 'none';
		element.addEventListener('play', () => {
			this.playing = true;
			this.#tick();
		});
		element.addEventListener('pause', () => {
			this.playing = false;
			this.#untick();
			this.position = element.currentTime;
		});
		element.addEventListener('ended', () => {
			this.playing = false;
			this.#untick();
			this.position = 0;
		});
		// `timeupdate` only fires a few times a second, which is enough to keep a paused or stalled
		// player honest but far too coarse for a seekbar; while playing, `#tick` takes over.
		element.addEventListener('timeupdate', () => {
			if (this.#frame === null) this.position = element.currentTime;
		});
		element.addEventListener('seeked', () => (this.position = element.currentTime));
		element.addEventListener('durationchange', () => {
			this.measured = audioDuration(element.duration);
		});
		element.addEventListener('error', () => {
			this.failedId = this.activeId;
			this.playing = false;
			this.#untick();
		});
		this.#element = element;
		return element;
	}

	/** Follows `currentTime` per frame, so the seekbar moves with the audio rather than in the
	 * quarter-second jumps `timeupdate` reports. Runs only while something is playing. */
	#tick() {
		if (this.#frame !== null) return;
		const step = () => {
			const element = this.#element;
			if (!element || element.paused) {
				this.#frame = null;
				return;
			}
			this.position = element.currentTime;
			this.#frame = requestAnimationFrame(step);
		};
		this.#frame = requestAnimationFrame(step);
	}

	#untick() {
		if (this.#frame === null) return;
		cancelAnimationFrame(this.#frame);
		this.#frame = null;
	}

	#load(id: number, src: string): HTMLAudioElement {
		const element = this.#audio();
		if (this.activeId !== id) {
			element.src = src;
			element.volume = 1;
			this.level = 1;
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

	/**
	 * Plays the active media at `value`, the level its pack author gave it.
	 *
	 * What the volume slider in the row's detail panel is *for*: levelling a pack is done by ear,
	 * and an author cannot hear a level the preview does not apply. Fed the level being dragged
	 * rather than the stored one, so the slider is audible while it moves.
	 */
	setLevel(value: number) {
		this.level = value;
		if (this.#element) this.#element.volume = value;
	}

	setMuted(value: boolean) {
		this.#audio().muted = value;
		this.muted = value;
	}

	/** Leaves the list. The element is kept, muted and all, for the next visit. */
	stop() {
		this.#element?.pause();
		this.#untick();
		this.activeId = null;
		this.playing = false;
		this.position = 0;
		this.measured = 0;
		this.failedId = null;
		this.level = 1;
		if (this.#element) this.#element.volume = 1;
	}
}

export const playback = new AudioPlayback();
