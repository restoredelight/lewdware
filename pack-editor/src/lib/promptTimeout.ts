/** The default deadline used by the bundled modes when a prompt has no explicit override. */
export function automaticPromptTimeout(text: string): number {
	const characters = Array.from(text).length;
	return Math.ceil(Math.max(15, 10 + characters / 2.5) / 5) * 5;
}
