import type { DevicesResponse, Meters, MixerState, Role, StatusPayload, Telemetry } from "@/types/mixer";

export interface FastMetersResponse {
  running: boolean;
  meters: Meters;
  telemetry: Telemetry;
  master_vol: number;
  master_mute: boolean;
}

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
  getDevices: () => req<DevicesResponse>("/api/devices"),
  setDevices: (payload: { input_id: string; intex_id: string; boat_id: string; laptop_id: string }) =>
    req<StatusPayload>("/api/devices", "POST", payload),
  getState: () => req<StatusPayload>("/api/state"),
  getMeters: () => req<FastMetersResponse>("/api/meters"),
  updateMixer: (patch: Partial<MixerState>) => req<{ ok: boolean; mixer: MixerState }>("/api/mixer", "POST", patch),
  startEngine: () => req<StatusPayload>("/api/start", "POST", {}),
  stopEngine: () => req<StatusPayload>("/api/stop", "POST", {}),
  boostCapture: () => req<StatusPayload>("/api/boost_capture", "POST", {}),
  setWinVol: (role: Role, volume_pct: number) => req<StatusPayload>("/api/set_win_vol", "POST", { role, volume_pct }),
  openSoundSettings: () => req<{ ok: boolean }>("/api/open_sound_settings", "POST", {}),
};
