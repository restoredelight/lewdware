/**
 * The reserved tag namespace, mirroring `shared/src/tags.rs`.
 *
 * The backend is what enforces it — every tag-writing command refuses these names, and
 * `get_all_tags` never offers one. This is the display half: a file's own tag list comes from
 * `get_files` unfiltered (the marker is real data, and the media slots read it), so anywhere the
 * author is shown or offered a file's tags has to hide the managed ones itself.
 */
export const MANAGED_TAG_PREFIX = '__lewdware-';

/** Keeps a file out of the ordinary popup pool. */
export const NON_POPUP_TAG = '__lewdware-non-popup';

/** Membership of the subliminal pool. Orthogonal to {@link NON_POPUP_TAG}: a subliminal that is
 * also shown in popups keeps this and loses that. */
export const SUBLIMINAL_TAG = '__lewdware-subliminal';

/** Audio without this marker is background audio; marked audio plays when a popup spawns. */
export const POPUP_AUDIO_TAG = '__lewdware-audio-popup';

export function isManagedTag(tag: string): boolean {
	return tag.startsWith(MANAGED_TAG_PREFIX);
}

export function withoutManagedTags(tags: string[]): string[] {
	return tags.filter((tag) => !isManagedTag(tag));
}
