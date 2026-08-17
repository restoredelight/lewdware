import { describe, expect, it } from 'vitest';
import { automaticPromptTimeout } from './promptTimeout.js';

describe('automaticPromptTimeout', () => {
	it('gives short prompts a fifteen-second floor', () => {
		expect(automaticPromptTimeout('Type me')).toBe(15);
	});

	it('grows in readable five-second steps with the prompt length', () => {
		expect(automaticPromptTimeout('x'.repeat(50))).toBe(30);
		expect(automaticPromptTimeout('x'.repeat(100))).toBe(50);
	});

	it('counts user-visible characters rather than UTF-16 code units', () => {
		expect(automaticPromptTimeout('😀'.repeat(50))).toBe(30);
	});
});
