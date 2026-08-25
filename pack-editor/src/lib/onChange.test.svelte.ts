import { describe, expect, it } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import OnChangeProbe from './OnChangeProbe.test.svelte';

function probe() {
	const target = document.createElement('div');
	document.body.appendChild(target);
	const runs: string[] = [];
	const bare: string[] = [];
	const props: {
		id: string;
		stages: { id: string }[];
		onrun: (v: string) => void;
		onbare: (v: string) => void;
	} = $state({
		id: 'peak',
		stages: [{ id: 'peak' }],
		onrun: (value: string) => void runs.push(value),
		onbare: (value: string) => void bare.push(value)
	});
	const app = mount(OnChangeProbe, { target, props });
	flushSync();
	return {
		runs,
		bare,
		select: (id: string) => {
			props.id = id;
			flushSync();
		},
		/** What a refetch does: the same answer, in new objects. */
		refetch: () => {
			props.stages = [{ id: props.id }];
			flushSync();
		},
		dispose: () => {
			unmount(app);
			document.body.removeChild(target);
		}
	};
}

describe('onChange', () => {
	it('runs once for the value it starts on', () => {
		const it_ = probe();
		expect(it_.runs).toEqual(['peak']);
		it_.dispose();
	});

	it('runs again when the value actually changes', () => {
		const it_ = probe();
		it_.select('climax');
		expect(it_.runs).toEqual(['peak', 'climax']);
		it_.dispose();
	});

	it('stays put when a refetch replaces the object the value came off', () => {
		const it_ = probe();
		it_.refetch();
		it_.refetch();
		expect(it_.runs).toEqual(['peak']);
		// And the version this replaced really does fire on each one, which is the whole reason the
		// helper exists: every edit on the timeline tab refetches it, so a scroll reset written as a
		// bare effect happens on every keystroke rather than when the author picks another stage.
		expect(it_.bare).toEqual(['peak', 'peak', 'peak']);
		it_.dispose();
	});

	it('still notices a change that arrives in the same refetch', () => {
		const it_ = probe();
		it_.select('climax');
		it_.refetch();
		it_.select('peak');
		expect(it_.runs).toEqual(['peak', 'climax', 'peak']);
		it_.dispose();
	});
});
