/**
 * Auto-repeat that follows the key rather than the event queue.
 *
 * A held arrow key arrives as a stream of `keydown`s at the desktop's own repeat rate -- around
 * thirty a second. Stepping once per event is fine where a step is cheap, and wrong in the two
 * places here that it isn't: one loads a full-size image or video, the other scrolls a virtualized
 * grid onto rows of thumbnails that have to be fetched and decoded. The repeats queue up behind
 * that work, and the queue is still draining when the key comes up: the viewer carries on flicking
 * through files, the grid onto a few more tiles, after you have let go.
 *
 * Rate-limiting the handler thins that out without ending it -- a limiter sees a backlogged repeat
 * and a live one as the same event, so a long enough hold still overshoots. So the repeats are not
 * what drives the stepping here. The first press steps, the first repeat starts a timer, and from
 * then on the timer steps at a pace the surface can keep up with. Every repeat behind it is a
 * no-op, which costs nothing and moves nothing however deep the backlog goes. `keyup` stops the
 * timer, and being the last thing the desktop sends it arrives after them: the last step is the one
 * the clock had reached when the key came up.
 */

/**
 * How long the timer keeps stepping with no word from the desktop that the key is still down.
 *
 * The repeats are not what paces the stepping, but their arrival is still the proof that the key is
 * held, and it is the only proof left when a `keyup` never comes -- another window taking focus
 * mid-hold, a native menu opening over the app. Well clear of the slowest repeat rate a desktop
 * offers (2/s), so a real hold is never cut short by it.
 */
export const HELD_TIMEOUT_MS = 1000;

/** As much of a `KeyboardEvent` as a hold is made of, so a test can hand over the two fields. */
export type HeldKey = { key: string; repeat: boolean };

export class KeyRepeater {
	/** The key being repeated, or null when nothing is held. */
	#key: string | null = null;
	#timer: ReturnType<typeof setInterval> | undefined;
	/** The step to take, re-read from each repeat so a modifier let go mid-hold is noticed. */
	#run: (() => void) | undefined;
	/** When the desktop last said the key was down, on our clock rather than the event's. */
	#seen = 0;

	/** @param intervalMs the gap between steps taken while the key is held. */
	constructor(private readonly intervalMs: number) {}

	/**
	 * Offers a `keydown` to the repeater, which either steps now or leaves it to the timer.
	 *
	 * `run` is called for the step rather than the event being handed back, so what a step means --
	 * and which modifiers were held for it -- stays with the caller.
	 */
	press(event: HeldKey, run: () => void) {
		if (event.key !== this.#key) this.stop();
		this.#key = event.key;
		this.#run = run;
		this.#seen = performance.now();

		// A press of your own always steps, at once: only repeats are paced.
		if (!event.repeat) {
			run();
			return;
		}
		// The first repeat is a step too -- the desktop's repeat delay has passed, and that delay is
		// the user's setting for when a hold starts moving. From here the clock has it.
		if (this.#timer === undefined) {
			run();
			this.#timer = setInterval(() => this.#tick(), this.intervalMs);
		}
	}

	/** Offers a `keyup`, which ends the hold if it is the held key coming up. */
	release(event: { key: string }) {
		if (event.key === this.#key) this.stop();
	}

	/** Ends any hold. For losing focus, closing the surface, or unmounting it. */
	stop() {
		if (this.#timer !== undefined) clearInterval(this.#timer);
		this.#timer = undefined;
		this.#key = null;
		this.#run = undefined;
	}

	#tick() {
		if (performance.now() - this.#seen > HELD_TIMEOUT_MS) {
			this.stop();
			return;
		}
		this.#run?.();
	}
}
