import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type {
	AudioDeviceInfo,
	ConfigDto,
	DiagnosticsDto,
	EngineStatusDto,
	Key,
	ModeGroupDto,
	ModeId,
	ModeOptionsDto,
	MonitorDto,
	PackMetadataDto,
	PickPackResult,
	ScheduleStatusDto,
	StoredValue,
	TestAudioResult,
	ThemeCatalogueDto,
	UploadModeResult,
	WallpaperSupportDto
} from './types';

async function invokeAfterSelection<T>(
	command: string,
	event: string,
	onSelected?: () => void
): Promise<T> {
	const unlisten = await listen(event, () => onSelected?.());
	try {
		return await invoke<T>(command);
	} finally {
		unlisten();
	}
}

export const api = {
	getConfig: () => invoke<ConfigDto>('get_config'),

	saveConfig: (config: ConfigDto) => invoke<void>('save_config', { config }),

	getThemeCatalogue: () => invoke<ThemeCatalogueDto>('get_theme_catalogue'),

	getMonitors: () => invoke<MonitorDto[]>('get_monitors'),

	getAudioDevices: () => invoke<AudioDeviceInfo[]>('get_audio_devices'),

	// Resolves only once the chime has finished playing, so the button can stay busy for its
	// duration. `device` is `null` for the system default.
	testAudioDevice: (device: string | null) =>
		invoke<TestAudioResult>('test_audio_device', { device }),

	getModeGroups: () => invoke<ModeGroupDto[]>('get_mode_groups'),

	getPackMetadata: () => invoke<PackMetadataDto | null>('get_pack_metadata'),

	getModeOptions: () => invoke<ModeOptionsDto>('get_mode_options'),

	// Sends the value as the control produced it; the backend reads it back against the
	// mode's schema, so there is nothing to convert here.
	setModeOption: (key: string, value: StoredValue) =>
		invoke<void>('set_mode_option', { key, value }),

	pickPack: (onSelected?: () => void) =>
		invokeAfterSelection<PickPackResult | null>('pick_pack', 'picker:pack-selected', onSelected),

	removePack: () => invoke<void>('remove_pack'),

	uploadMode: (onSelected?: () => void) =>
		invokeAfterSelection<UploadModeResult | null>(
			'upload_mode',
			'picker:mode-selected',
			onSelected
		),

	removeUploadedMode: (path: string) => invoke<ModeGroupDto[]>('remove_uploaded_mode', { path }),

	launchLewdware: () => invoke<void>('launch_lewdware'),

	stopLewdware: () => invoke<void>('stop_lewdware'),

	lewdwareRunning: () => invoke<EngineStatusDto>('lewdware_running'),

	getScheduleStatus: () => invoke<ScheduleStatusDto>('get_schedule_status'),

	setScheduleEnabled: (enabled: boolean) => invoke<void>('set_schedule_enabled', { enabled }),

	reloadSupervisorSchedule: () => invoke<void>('reload_supervisor_schedule'),

	openLogs: () => invoke<void>('open_logs'),

	getDiagnostics: (limit = 2000) => invoke<DiagnosticsDto>('get_diagnostics', { limit }),

	inputMonitoringGranted: () => invoke<boolean>('input_monitoring_granted'),

	requestInputMonitoring: () => invoke<boolean>('request_input_monitoring'),

	openInputMonitoringSettings: () => invoke<void>('open_input_monitoring_settings'),

	wallpaperSupport: () => invoke<WallpaperSupportDto>('wallpaper_support'),

	wallpaperRestorePreview: (path: string) =>
		invoke<string | null>('wallpaper_restore_preview', { path }),

	pickRestoreImage: (onSelected?: () => void) =>
		invokeAfterSelection<string | null>(
			'pick_restore_image',
			'picker:restore-image-selected',
			onSelected
		),

	defaultRestoreImage: () => invoke<string>('default_restore_image')
};
