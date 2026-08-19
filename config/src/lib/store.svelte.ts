import { api } from './api';
import type {
	AudioDeviceChoice,
	AudioDeviceInfo,
	Capabilities,
	ConditionValue,
	ConfigDto,
	EngineStatusDto,
	Key,
	ModeGroupDto,
	ModeId,
	ModeOptionsDto,
	OptionEntryDto,
	Permission,
	OptionValue,
	MonitorDto,
	MonitorRegion,
	QuietHoursDto,
	ScheduleStatusDto,
	SupervisorStatusDto,
	ThemeCatalogueDto,
	Volume,
	WallpaperRestore,
	WindowDto
} from './types';
import { taskFeedback } from '$ui/taskFeedback.svelte.js';

function defaultWindow(): WindowDto {
	return {
		days: [false, false, false, false, false, false, false],
		start_hour: 9,
		start_minute: 0,
		duration_minutes: 60,
		jitter_minutes: 0
	};
}

function defaultQuietHours(): QuietHoursDto {
	return {
		days: [true, true, true, true, true, true, true],
		start_hour: 22,
		start_minute: 0,
		end_hour: 7,
		end_minute: 0
	};
}

function updateOptionValue(
	entries: OptionEntryDto[],
	key: string,
	value: OptionValue
): OptionEntryDto[] {
	return entries.map((entry) => {
		if (entry.kind === 'Option') {
			return entry.key === key ? { ...entry, value } : entry;
		} else {
			return { ...entry, entries: updateOptionValue(entry.entries, key, value) };
		}
	});
}

function modeIdEqual(a: ModeId, b: ModeId): boolean {
	if (a.type !== b.type) return false;
	if (a.type === 'Sandbox' && b.type === 'Sandbox') return true;
	if (a.type === 'Experience' && b.type === 'Experience') return true;
	if (a.type === 'Pack' && b.type === 'Pack') return a.id === b.id;
	if (a.type === 'File' && b.type === 'File') return a.path === b.path;
	return false;
}

class AppStore {
	config = $state<ConfigDto | null>(null);
	// Listing monitors means spawning the engine to probe them, which is far slower than the rest of
	// the load -- so it gets its own state and only the Monitors section waits on it.
	monitors = $state<MonitorDto[]>([]);
	monitorsLoading = $state(false);
	monitorsError = $state<string | null>(null);
	// Same deal for audio outputs -- another engine probe. Unlike monitors this is *not* loaded up
	// front: it is only ever needed by the Audio page, and devices come and go (headphones,
	// bluetooth), so a list fetched at app start would be stale by the time anyone looked at it.
	// `loadAudioDevices` is called when that page mounts instead.
	audioDevices = $state<AudioDeviceInfo[]>([]);
	audioDevicesLoading = $state(false);
	audioDevicesError = $state<string | null>(null);
	modeGroups = $state<ModeGroupDto[]>([]);
	/** The window looks on offer, read once at load. Static for the life of the app -- it is the
	 * engine's own catalogue (`shared::theme`), not anything the user can change. */
	themeCatalogue = $state<ThemeCatalogueDto>({
		themes: [],
		appearances: [],
		system_appearance: null
	});
	modeOptions = $state<OptionEntryDto[]>([]);
	/** Permissions the selected mode uses regardless of how it is configured -- they hang off no
	 * single option, so they are surfaced once above the option list rather than on a row. */
	modeNeedsPermissions = $state<Permission[]>([]);
	/** Pack-derived `pack_has_*` facts for the selected mode, seeded into `show_when` evaluation
	 * alongside the live option values (see `PackMode.svelte`). */
	packHas = $state<Record<string, ConditionValue>>({});
	activeTab = $state<
		'pack_mode' | 'safety' | 'monitors' | 'audio' | 'window_style' | 'scheduling' | 'diagnostics'
	>('pack_mode');
	loading = $state(false);
	loadError = $state<string | null>(null);
	busyActions = $state<string[]>([]);
	workingActions = $state<string[]>([]);
	// Kept fresh by the `supervisor:status` push event (see +page.svelte); the initial values
	// come from one fetch at startup.
	engineStatus = $state<EngineStatusDto>({ running: false, error: null, warning: null });
	scheduleStatus = $state<ScheduleStatusDto>({ enabled: false, next_session: null });
	private saveQueue: Promise<void> = Promise.resolve();
	private pendingSaves = 0;

	get ready() {
		return this.config !== null;
	}

	/** Single place the two halves of `get_mode_options` land, so a mode's declared permissions can
	 * never drift out of step with the options they belong to. */
	private applyModeOptions(dto: ModeOptionsDto) {
		this.modeOptions = dto.entries;
		this.modeNeedsPermissions = dto.needs_permissions;
		this.packHas = dto.pack_has;
	}

	applySupervisorStatus(status: SupervisorStatusDto) {
		this.engineStatus = status.engine;
		this.scheduleStatus = status.schedule;
	}

	async refreshSupervisorStatus() {
		try {
			const [engine, schedule] = await Promise.all([
				api.lewdwareRunning(),
				api.getScheduleStatus()
			]);
			this.engineStatus = engine;
			this.scheduleStatus = schedule;
		} catch (err) {
			taskFeedback.warning('supervisor-status', `Couldn’t read Lewdware status: ${String(err)}`);
		}
	}

	async loadMonitors() {
		this.monitorsLoading = true;
		this.monitorsError = null;
		try {
			this.monitors = await api.getMonitors();
		} catch (err) {
			this.monitorsError = String(err);
		} finally {
			this.monitorsLoading = false;
		}
	}

	async loadAudioDevices() {
		this.audioDevicesLoading = true;
		this.audioDevicesError = null;
		try {
			this.audioDevices = await api.getAudioDevices();
		} catch (err) {
			this.audioDevicesError = String(err);
		} finally {
			this.audioDevicesLoading = false;
		}
	}

	async load() {
		this.loading = true;
		this.loadError = null;
		// Deliberately not awaited: the rest of the settings shouldn't be held up by the monitor probe,
		// and the Monitors section renders its own loading/error state.
		void this.loadMonitors();
		try {
			const [config, modeGroups, modeOptions, themeCatalogue] = await Promise.all([
				api.getConfig(),
				api.getModeGroups(),
				api.getModeOptions(),
				api.getThemeCatalogue()
			]);
			this.config = config;
			this.modeGroups = modeGroups;
			this.applyModeOptions(modeOptions);
			this.themeCatalogue = themeCatalogue;
			taskFeedback.dismiss('load');
		} catch (err) {
			this.loadError = String(err);
			taskFeedback.error('load', 'Settings couldn’t be loaded');
		} finally {
			this.loading = false;
		}
	}

	isBusy(action: string) {
		return this.busyActions.includes(action);
	}

	isWorking(action: string) {
		return this.workingActions.includes(action);
	}

	private setBusy(action: string, busy: boolean) {
		this.busyActions = busy
			? [...new Set([...this.busyActions, action])]
			: this.busyActions.filter((item) => item !== action);
	}

	private setWorking(action: string, working: boolean) {
		this.workingActions = working
			? [...new Set([...this.workingActions, action])]
			: this.workingActions.filter((item) => item !== action);
	}

	async saveConfig(): Promise<boolean> {
		if (!this.config) return false;
		const snapshot = $state.snapshot(this.config);
		this.pendingSaves += 1;
		taskFeedback.progress('save', 'Saving settings…');
		const operation = this.saveQueue.then(() => api.saveConfig(snapshot));
		this.saveQueue = operation.catch(() => {});
		try {
			await operation;
			return true;
		} catch (err) {
			taskFeedback.error('save', `Couldn’t save settings: ${String(err)}`);
			return false;
		} finally {
			this.pendingSaves -= 1;
			if (
				this.pendingSaves === 0 &&
				taskFeedback.entries.find((entry) => entry.id === 'save')?.tone === 'progress'
			) {
				taskFeedback.success('save', 'Settings saved');
			}
		}
	}

	setPanicButton(key: Key) {
		if (!this.config) return;
		this.config = { ...this.config, panic_button: key };
		this.saveConfig();
	}

	setMonitorEnabled(id: string, enabled: boolean) {
		if (!this.config) return;
		let disabled = [...this.config.disabled_monitors];
		if (enabled) {
			disabled = disabled.filter((m) => m !== id);
		} else if (!disabled.includes(id)) {
			disabled = [...disabled, id];
		}
		this.config = { ...this.config, disabled_monitors: disabled };
		this.monitors = this.monitors.map((m) => (m.id === id ? { ...m, disabled: !enabled } : m));
		this.saveConfig();
	}

	/** Narrow (or restore) the part of a monitor popups may use.
	 *
	 * `null` deletes the entry rather than storing a full-screen rectangle, so a monitor the user
	 * never restricted stays absent from `config.json` — and a region is never mistaken for a
	 * deliberate choice when the monitor is later resized. `MonitorDto.region` is kept in step so
	 * the picker doesn't have to re-run the (slow) monitor probe to see its own edit. */
	setMonitorRegion(id: string, region: MonitorRegion | null) {
		if (!this.config) return;

		const regions = { ...this.config.monitor_regions };
		if (region === null) {
			delete regions[id];
		} else {
			regions[id] = region;
		}

		this.config = { ...this.config, monitor_regions: regions };
		this.monitors = this.monitors.map((m) =>
			m.id === id ? { ...m, region: region ?? { x: 0, y: 0, width: 1, height: 1 } } : m
		);
		this.saveConfig();
	}

	// The user's window look. Both axes are plain `ConfigDto` fields the engine reads at spawn
	// time (see `AppConfig::theme`), so saving is all there is to do -- nothing to tell a running
	// session, which reads its chrome per window as it opens them.
	setTheme(theme: string) {
		if (!this.config) return;
		this.config = { ...this.config, theme };
		this.saveConfig();
	}

	setAppearance(appearance: string) {
		if (!this.config) return;
		this.config = { ...this.config, appearance };
		this.saveConfig();
	}

	setCapability(key: keyof Capabilities, enabled: boolean) {
		if (!this.config) return;
		this.config = {
			...this.config,
			capabilities: { ...this.config.capabilities, [key]: enabled }
		};
		this.saveConfig();
	}

	// What the wallpaper is put back to when a pack finishes. On a desktop that can't report its
	// own wallpaper this is what makes the "Change wallpaper" permission usable at all -- until an
	// image is chosen the engine declines, rather than making a change it could never undo.
	setWallpaperRestore(restore: WallpaperRestore) {
		if (!this.config) return;
		this.config = { ...this.config, wallpaper: { ...this.config.wallpaper, restore } };
		this.saveConfig();
	}

	// Updates local state only, without saving -- meant for a slider's continuous `oninput`, so
	// dragging doesn't fire an IPC round trip per tick. Pair with `saveConfig()` on `onchange`.
	previewVolume(key: keyof Volume, value: number) {
		if (!this.config) return;
		this.config = { ...this.config, volume: { ...this.config.volume, [key]: value } };
	}

	// Which output sounds play on; `null` is the system default, which the engine re-resolves each
	// time it opens a sink. Takes effect for sinks opened from here on, which in practice means the
	// next session -- the engine reads this once at startup, exactly like `volume`.
	//
	// The name is stored alongside the id only so the picker can name a device that is no longer
	// connected; see `AudioDeviceChoice`.
	setAudioDevice(device: AudioDeviceChoice | null) {
		if (!this.config) return;
		this.config = { ...this.config, audio_device: device };
		this.saveConfig();
	}

	// `saveConfig()` (schedule content is a normal ConfigDto field, so this alone persists it) plus
	// a best-effort ping to a resident supervisor so an already-running one picks up the change
	// without waiting for its next boundary wake. Every schedule *content* editing method below ends
	// by calling this -- except `setScheduleEnabled`, which is the one field that also drives OS
	// autostart registration and needs its own error handling, so it's never routed through here.
	async saveSchedule() {
		if (await this.saveConfig()) await api.reloadSupervisorSchedule().catch(() => {});
	}

	// Deliberately not routed through `saveSchedule()`: enabling/disabling also registers/
	// deregisters OS autostart, which can fail (e.g. no installed binary found) -- a failure must
	// not silently flip the toggle. Throws on failure so the caller (`Scheduling.svelte`) can show
	// an error and leave the toggle in its previous state.
	async setScheduleEnabled(enabled: boolean) {
		await api.setScheduleEnabled(enabled);
		if (!this.config) return;
		this.config = { ...this.config, schedule: { ...this.config.schedule, enabled } };
	}

	setGraceNotification(enabled: boolean) {
		if (!this.config) return;
		this.config = {
			...this.config,
			schedule: { ...this.config.schedule, grace_notification: enabled }
		};
		this.saveSchedule();
	}

	addWindow() {
		if (!this.config) return;
		this.config = {
			...this.config,
			schedule: {
				...this.config.schedule,
				windows: [...this.config.schedule.windows, defaultWindow()]
			}
		};
		this.saveSchedule();
	}

	removeWindow(index: number) {
		if (!this.config) return;
		this.config = {
			...this.config,
			schedule: {
				...this.config.schedule,
				windows: this.config.schedule.windows.filter((_, i) => i !== index)
			}
		};
		this.saveSchedule();
	}

	updateWindow(index: number, patch: Partial<WindowDto>) {
		if (!this.config) return;
		this.config = {
			...this.config,
			schedule: {
				...this.config.schedule,
				windows: this.config.schedule.windows.map((w, i) => (i === index ? { ...w, ...patch } : w))
			}
		};
		this.saveSchedule();
	}

	addQuietHours() {
		if (!this.config) return;
		this.config = {
			...this.config,
			schedule: {
				...this.config.schedule,
				quiet_hours: [...this.config.schedule.quiet_hours, defaultQuietHours()]
			}
		};
		this.saveSchedule();
	}

	removeQuietHours(index: number) {
		if (!this.config) return;
		this.config = {
			...this.config,
			schedule: {
				...this.config.schedule,
				quiet_hours: this.config.schedule.quiet_hours.filter((_, i) => i !== index)
			}
		};
		this.saveSchedule();
	}

	updateQuietHours(index: number, patch: Partial<QuietHoursDto>) {
		if (!this.config) return;
		this.config = {
			...this.config,
			schedule: {
				...this.config.schedule,
				quiet_hours: this.config.schedule.quiet_hours.map((q, i) =>
					i === index ? { ...q, ...patch } : q
				)
			}
		};
		this.saveSchedule();
	}

	async pickPack() {
		this.setBusy('pack', true);
		try {
			const result = await api.pickPack(() => this.setWorking('pick-pack', true));
			if (!result || !this.config) return;
			taskFeedback.progress('pack', 'Opening pack…');
			this.config = { ...this.config, pack_path: result.pack_path };
			if (result.first_mode) await this.setMode(result.first_mode, result.mode_groups);
			else this.modeGroups = result.mode_groups;
			taskFeedback.success('pack', 'Pack selected');
		} catch (err) {
			taskFeedback.error('pack', `Couldn’t select pack: ${String(err)}`);
		} finally {
			this.setWorking('pick-pack', false);
			this.setBusy('pack', false);
		}
	}

	async removePack() {
		this.setBusy('pack', true);
		taskFeedback.progress('pack', 'Removing pack…');
		try {
			await api.removePack();
			if (!this.config) return;
			this.config = { ...this.config, pack_path: null };
			const [groups, options] = await Promise.all([api.getModeGroups(), api.getModeOptions()]);
			this.modeGroups = groups;
			this.applyModeOptions(options);
			taskFeedback.success('pack', 'Pack removed');
		} catch (err) {
			taskFeedback.error('pack', `Couldn’t remove pack: ${String(err)}`);
		} finally {
			this.setBusy('pack', false);
		}
	}

	async setMode(modeId: ModeId, groups?: ModeGroupDto[]) {
		if (!this.config) return;
		this.setBusy('mode', true);
		taskFeedback.progress('mode', 'Changing mode…');
		const previousMode = this.config.mode;
		let modeSaved = false;
		try {
			this.config = { ...this.config, mode: modeId };
			if (groups) this.modeGroups = groups;
			// The backend resolves options from the persisted config, so the selected mode must be
			// saved before asking for its options. Fetching first returns the previously selected
			// mode's schema (most visibly swapping Sandbox and Experience options).
			if (!(await this.saveConfig())) {
				this.config = { ...this.config, mode: previousMode };
				throw new Error('the new mode could not be saved');
			}
			modeSaved = true;
			this.modeOptions = [];
			this.applyModeOptions(await api.getModeOptions());
			taskFeedback.success('mode', 'Mode changed');
		} catch (err) {
			taskFeedback.error(
				'mode',
				modeSaved
					? `Mode changed, but its options couldn’t be loaded: ${String(err)}`
					: `Couldn’t change mode: ${String(err)}`
			);
		} finally {
			this.setBusy('mode', false);
		}
	}

	async uploadMode() {
		this.setBusy('mode', true);
		try {
			const result = await api.uploadMode(() => this.setWorking('upload-mode', true));
			if (!result) return;
			this.modeGroups = result.mode_groups;
			taskFeedback.success('mode', 'Mode uploaded');
		} catch (err) {
			taskFeedback.error('mode', `Couldn’t upload mode: ${String(err)}`);
		} finally {
			this.setWorking('upload-mode', false);
			this.setBusy('mode', false);
		}
	}

	async removeUploadedMode(path: string) {
		this.setBusy('mode', true);
		taskFeedback.progress('mode', 'Removing uploaded mode…');
		try {
			const groups = await api.removeUploadedMode(path);
			this.modeGroups = groups;
			if (this.config && this.config.mode.type === 'File' && this.config.mode.path === path) {
				const first = groups.find((g) => g.source === 'builtin')?.entries[0];
				if (first) {
					this.config = { ...this.config, mode: first.id };
					this.applyModeOptions(await api.getModeOptions());
					await this.saveConfig();
				}
			}
			taskFeedback.success('mode', 'Uploaded mode removed');
		} catch (err) {
			taskFeedback.error('mode', `Couldn’t remove mode: ${String(err)}`);
		} finally {
			this.setBusy('mode', false);
		}
	}

	async setModeOption(key: string, value: unknown) {
		try {
			await api.setModeOption(key, value as never);
			this.modeOptions = updateOptionValue(this.modeOptions, key, value as OptionValue);
		} catch (err) {
			taskFeedback.error('mode-option', `Couldn’t update option: ${String(err)}`);
		}
	}

	isModeSelected(modeId: ModeId): boolean {
		return !!this.config && modeIdEqual(this.config.mode, modeId);
	}
}

export const store = new AppStore();
