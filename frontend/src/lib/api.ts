import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { DevicesResponse, Meters, MixerState, Role, StatusPayload, Telemetry } from "@/types/mixer";

export interface FastMetersResponse {
  running: boolean;
  meters: Meters;
  telemetry: Telemetry;
  master_vol: number;
  master_mute: boolean;
}

export interface CalibrationResult {
  ok: boolean;
  measured_ms: number;
  confidence: number;
  message: string;
  status: StatusPayload;
}

export interface ProfileIoResult {
  ok: boolean;
  cancelled?: boolean;
  path?: string;
  status?: StatusPayload;
}

export interface ToastEvent {
  text: string;
  tone?: "info" | "success" | "warn";
}

export const IS_TAURI =
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

async function req<T>(url: string, method = "GET", body?: unknown): Promise<T> {
  const res = await fetch(url, {
    method,
    headers: { "Content-Type": "application/json" },
    body: body !== undefined ? JSON.stringify(body) : undefined,
  });
  if (!res.ok) {
    throw new Error(`HTTP ${res.status}`);
  }
  return (await res.json()) as T;
}

export const mixerApi = {
  getDevices: () =>
    IS_TAURI ? invoke<DevicesResponse>("get_devices") : req<DevicesResponse>("/api/devices"),
  setDevices: (payload: { input_id: string; intex_id: string; boat_id: string; laptop_id: string }) =>
    IS_TAURI
      ? invoke<StatusPayload>("set_devices", { selected: payload })
      : req<StatusPayload>("/api/devices", "POST", payload),
  getState: () =>
    IS_TAURI ? invoke<StatusPayload>("get_state") : req<StatusPayload>("/api/state"),
  getMeters: () =>
    IS_TAURI ? invoke<FastMetersResponse>("get_meters") : req<FastMetersResponse>("/api/meters"),
  updateMixer: (patch: Partial<MixerState>) =>
    IS_TAURI
      ? invoke<{ ok: boolean; mixer: MixerState }>("update_mixer", { patch })
      : req<{ ok: boolean; mixer: MixerState }>("/api/mixer", "POST", patch),
  startEngine: () =>
    IS_TAURI ? invoke<StatusPayload>("start_engine") : req<StatusPayload>("/api/start", "POST", {}),
  stopEngine: () =>
    IS_TAURI ? invoke<StatusPayload>("stop_engine") : req<StatusPayload>("/api/stop", "POST", {}),
  boostCapture: () =>
    IS_TAURI ? invoke<StatusPayload>("boost_capture") : req<StatusPayload>("/api/boost_capture", "POST", {}),
  setWinVol: (role: Role, volume_pct: number) =>
    IS_TAURI
      ? invoke<StatusPayload>("set_win_vol", { role, volume_pct })
      : req<StatusPayload>("/api/set_win_vol", "POST", { role, volume_pct }),
  openSoundSettings: () =>
    IS_TAURI ? invoke<{ ok: boolean }>("open_sound_settings") : req<{ ok: boolean }>("/api/open_sound_settings", "POST", {}),
  getAutostart: () =>
    IS_TAURI ? invoke<boolean>("get_autostart") : Promise.resolve(false),
  setAutostart: (enabled: boolean) =>
    IS_TAURI ? invoke<boolean>("set_autostart", { enabled }) : Promise.resolve(false),
  calibrateSyncDelay: () =>
    IS_TAURI
      ? invoke<CalibrationResult>("calibrate_sync_delay")
      : req<CalibrationResult>("/api/calibrate_delay", "POST", {}),
  exportProfile: () =>
    IS_TAURI
      ? invoke<ProfileIoResult>("export_profile")
      : Promise.resolve<ProfileIoResult>({ ok: false, cancelled: true }),
  importProfile: () =>
    IS_TAURI
      ? invoke<ProfileIoResult>("import_profile")
      : Promise.resolve<ProfileIoResult>({ ok: false, cancelled: true }),
  resetFactory: () =>
    IS_TAURI ? invoke<StatusPayload>("reset_factory") : req<StatusPayload>("/api/reset_factory", "POST", {}),
};

export async function subscribeMetersEvent(
  onData: (ev: FastMetersResponse) => void
): Promise<() => void> {
  if (!IS_TAURI) return () => {};
  return await listen<FastMetersResponse>("phasemix:meters", (event) => {
    onData(event.payload);
  });
}

export async function subscribeMixerEvent(
  onMixer: (mixer: MixerState) => void
): Promise<() => void> {
  if (!IS_TAURI) return () => {};
  return await listen<MixerState>("phasemix:mixer", (event) => {
    onMixer(event.payload);
  });
}

export async function subscribeDevicesEvent(
  onDevices: (devices: DevicesResponse) => void
): Promise<() => void> {
  if (!IS_TAURI) return () => {};
  return await listen<DevicesResponse>("phasemix:devices", (event) => {
    onDevices(event.payload);
  });
}

export async function subscribeToastEvent(
  onToast: (toast: ToastEvent) => void
): Promise<() => void> {
  if (!IS_TAURI) return () => {};
  return await listen<ToastEvent>("phasemix:toast", (event) => {
    onToast(event.payload);
  });
}
