export type Role = "intex" | "boat" | "laptop";
export type AuxMode = "stereo" | "mono_center" | "vocal_mid" | "side_ambience";

export interface MixerState {
  master_vol: number;
  master_mute: boolean;
  protection_enabled: boolean;
  protection_max_master: number;
  protection_ceiling_db: number;
  position_preset: string;

  intex_vol: number;
  intex_mute: boolean;
  intex_delay_ms: number;
  intex_balance: number;
  intex_bass_db: number;
  intex_mid_db: number;
  intex_treble_db: number;
  intex_width: number;

  boat_vol: number;
  boat_mute: boolean;
  boat_delay_ms: number;
  boat_mode: AuxMode;
  boat_hpf_hz: number;
  boat_lpf_hz: number;
  boat_bass_db: number;
  boat_mid_db: number;
  boat_treble_db: number;
  boat_invert: boolean;

  laptop_vol: number;
  laptop_mute: boolean;
  laptop_delay_ms: number;
  laptop_mode: AuxMode;
  laptop_hpf_hz: number;
  laptop_lpf_hz: number;
  laptop_bass_db: number;
  laptop_mid_db: number;
  laptop_treble_db: number;
  laptop_invert: boolean;

  solo: Role | null;
  test_mode: "off" | "click" | "chirp" | "lr";
}

export interface DeviceInfo {
  id: string | number;
  name: string;
  rate: number;
  channels: number;
  is_loopback: boolean;
  label: string;
}

export interface WinEndpointVol {
  id: string;
  name: string;
  volume_pct: number;
  db?: number;
  mute: boolean;
}

export interface Telemetry {
  sample_rate: number;
  input_status: string;
  intex_status: string;
  boat_status: string;
  laptop_status: string;
  intex_actual_ms: number;
  boat_actual_ms: number;
  laptop_actual_ms: number;
  intex_rate_ppm: number;
  boat_rate_ppm: number;
  laptop_rate_ppm: number;
  intex_gr_db?: number;
  boat_gr_db?: number;
  laptop_gr_db?: number;
}

export interface Meters {
  in_db: number;
  intex_db: number;
  boat_db: number;
  laptop_db: number;
}

export interface StatusPayload {
  running: boolean;
  mixer: MixerState;
  meters: Meters;
  telemetry: Telemetry;
  selected: {
    input_id: string;
    intex_id: string;
    boat_id: string;
    laptop_id: string;
  };
  selected_names: {
    input_name: string;
    intex_name: string;
    boat_name: string;
    laptop_name: string;
  };
  win_vols: {
    input: WinEndpointVol | null;
    intex: WinEndpointVol | null;
    boat: WinEndpointVol | null;
    laptop: WinEndpointVol | null;
  };
}

export interface DevicesResponse {
  inputs: DeviceInfo[];
  outputs: DeviceInfo[];
  selected: StatusPayload["selected"];
  selected_names: StatusPayload["selected_names"];
  has_virtual_cable: boolean;
  win_vols: StatusPayload["win_vols"];
}

export interface StageLayout {
  intexL: [number, number, number];
  intexR: [number, number, number];
  subL: [number, number, number];
  subR: [number, number, number];
  boat: [number, number, number];
  laptop: [number, number, number];
}

export interface PositionPreset {
  id: string;
  name: string;
  subtitle: string;
  badge: string;
  patch: Partial<MixerState>;
  layout: StageLayout;
}

export const POSITION_PRESETS: PositionPreset[] = [
  {
    id: "desk_triangle",
    name: "Desk Triangle",
    subtitle: "Primary L/R flanking • Wireless focal center (260Hz+) • Built-in front air",
    badge: "BALANCED",
    patch: {
      position_preset: "desk_triangle",
      protection_max_master: 60,
      protection_ceiling_db: -6,
      intex_vol: 100, intex_balance: 0, intex_width: 106, intex_delay_ms: 224,
      intex_bass_db: 0.5, intex_mid_db: -2.0, intex_treble_db: 0.5,
      boat_vol: 60, boat_mode: "vocal_mid", boat_delay_ms: 0, boat_hpf_hz: 260, boat_lpf_hz: 14000,
      boat_bass_db: -2.5, boat_mid_db: 2.0, boat_treble_db: 0.5,
      laptop_vol: 125, laptop_mode: "stereo", laptop_delay_ms: 224, laptop_hpf_hz: 300, laptop_lpf_hz: 18000,
      laptop_bass_db: -2.0, laptop_mid_db: 0.0, laptop_treble_db: 2.0,
    },
    layout: {
      intexL: [-2.15, 0.4, -1.25],
      intexR: [2.15, 0.4, -1.25],
      subL: [-1.45, -0.55, -1.75],
      subR: [1.45, -0.55, -1.75],
      boat: [0, 0.25, -1.55],
      laptop: [0, 0.12, 0.55],
    },
  },
  {
    id: "cinema_center",
    name: "Cinema 3.2 Dialogue",
    subtitle: "Wide L/R stage • Carved vocal lock on center • Built-in side ambience",
    badge: "MOVIES",
    patch: {
      position_preset: "cinema_center",
      protection_max_master: 60,
      protection_ceiling_db: -6,
      intex_vol: 100, intex_balance: 0, intex_width: 135, intex_delay_ms: 224,
      intex_bass_db: 2.5, intex_mid_db: -3.5, intex_treble_db: 0.0,
      boat_vol: 72, boat_mode: "vocal_mid", boat_delay_ms: 0, boat_hpf_hz: 220, boat_lpf_hz: 12000,
      boat_bass_db: -3.0, boat_mid_db: 3.5, boat_treble_db: 1.0,
      laptop_vol: 120, laptop_mode: "side_ambience", laptop_delay_ms: 224, laptop_hpf_hz: 340, laptop_lpf_hz: 15000,
      laptop_bass_db: -3.0, laptop_mid_db: 0.0, laptop_treble_db: 2.0,
    },
    layout: {
      intexL: [-2.75, 0.45, -0.9],
      intexR: [2.75, 0.45, -0.9],
      subL: [-1.5, -0.55, -1.9],
      subR: [1.5, -0.55, -1.9],
      boat: [0, 0.35, -1.75],
      laptop: [0, 0.12, 0.72],
    },
  },
  {
    id: "wide_room",
    name: "Wide Room Music",
    subtitle: "Punchy L/R subs • Full-mid center fill • Smooth built-in tweeters",
    badge: "MUSIC",
    patch: {
      position_preset: "wide_room",
      protection_max_master: 60,
      protection_ceiling_db: -6,
      intex_vol: 110, intex_balance: 0, intex_width: 120, intex_delay_ms: 224,
      intex_bass_db: 4.0, intex_mid_db: 0.0, intex_treble_db: 1.5,
      boat_vol: 75, boat_mode: "mono_center", boat_delay_ms: 0, boat_hpf_hz: 185, boat_lpf_hz: 17500,
      boat_bass_db: 0.0, boat_mid_db: 1.5, boat_treble_db: 1.5,
      laptop_vol: 160, laptop_mode: "stereo", laptop_delay_ms: 224, laptop_hpf_hz: 250, laptop_lpf_hz: 18000,
      laptop_bass_db: 0.0, laptop_mid_db: 1.0, laptop_treble_db: 2.0,
    },
    layout: {
      intexL: [-2.95, 0.55, -1.75],
      intexR: [2.95, 0.55, -1.75],
      subL: [-1.95, -0.55, -2.05],
      subR: [1.95, -0.55, -2.05],
      boat: [0, 0.3, -1.35],
      laptop: [0, 0.12, 0.45],
    },
  },
  {
    id: "night_safe",
    name: "Night Ear-Safe",
    subtitle: "-15 dB spike ceiling • 60% max cap • Tamed subs & clear voice",
    badge: "PROTECT",
    patch: {
      position_preset: "night_safe",
      protection_enabled: true,
      protection_max_master: 60,
      protection_ceiling_db: -15.0,
      intex_vol: 75, intex_balance: 0, intex_width: 95, intex_delay_ms: 224,
      intex_bass_db: -5.0, intex_mid_db: 0.0, intex_treble_db: -2.0,
      boat_vol: 65, boat_mode: "vocal_mid", boat_delay_ms: 0, boat_hpf_hz: 240, boat_lpf_hz: 10000,
      boat_bass_db: -2.5, boat_mid_db: 2.0, boat_treble_db: -1.5,
      laptop_vol: 80, laptop_mode: "stereo", laptop_delay_ms: 224, laptop_hpf_hz: 280, laptop_lpf_hz: 12000,
      laptop_bass_db: -3.0, laptop_mid_db: 1.0, laptop_treble_db: -2.0,
    },
    layout: {
      intexL: [-1.5, 0.3, -0.85],
      intexR: [1.5, 0.3, -0.85],
      subL: [-1.05, -0.55, -1.4],
      subR: [1.05, -0.55, -1.4],
      boat: [0, 0.25, -0.75],
      laptop: [0, 0.12, 0.5],
    },
  },
  {
    id: "asym_desk",
    name: "Offset Desk",
    subtitle: "Compensates when left/right satellites sit closer to one side",
    badge: "OFFSET",
    patch: {
      position_preset: "asym_desk",
      protection_max_master: 60,
      protection_ceiling_db: -6,
      intex_vol: 100, intex_balance: 18, intex_width: 105, intex_delay_ms: 224,
      intex_bass_db: 1.0, intex_mid_db: -1.5, intex_treble_db: 0.5,
      boat_vol: 65, boat_mode: "vocal_mid", boat_delay_ms: 0, boat_hpf_hz: 280, boat_lpf_hz: 16000,
      boat_bass_db: -2.0, boat_mid_db: 2.0, boat_treble_db: 1.0,
      laptop_vol: 165, laptop_mode: "stereo", laptop_delay_ms: 224, laptop_hpf_hz: 260, laptop_lpf_hz: 18000,
      laptop_bass_db: -1.0, laptop_mid_db: 0.5, laptop_treble_db: 1.5,
    },
    layout: {
      intexL: [-2.55, 0.4, -0.6],
      intexR: [1.45, 0.4, -1.55],
      subL: [-1.95, -0.55, -1.3],
      subR: [1.05, -0.55, -1.85],
      boat: [0.12, 0.28, -1.45],
      laptop: [0.25, 0.12, 0.55],
    },
  },
  {
    id: "pure_mirror",
    name: "Pure Flat",
    subtitle: "0 dB flat EQ across all 9 bands • Direct stereo reference",
    badge: "FLAT",
    patch: {
      position_preset: "pure_mirror",
      protection_max_master: 60,
      protection_ceiling_db: -6,
      intex_vol: 100, intex_balance: 0, intex_width: 100, intex_delay_ms: 224,
      intex_bass_db: 0.0, intex_mid_db: 0.0, intex_treble_db: 0.0,
      boat_vol: 65, boat_mode: "stereo", boat_delay_ms: 0, boat_hpf_hz: 0, boat_lpf_hz: 18000,
      boat_bass_db: 0.0, boat_mid_db: 0.0, boat_treble_db: 0.0,
      laptop_vol: 130, laptop_mode: "stereo", laptop_delay_ms: 224, laptop_hpf_hz: 220, laptop_lpf_hz: 18000,
      laptop_bass_db: 0.0, laptop_mid_db: 0.0, laptop_treble_db: 0.0,
    },
    layout: {
      intexL: [-2.0, 0.4, -1.2],
      intexR: [2.0, 0.4, -1.2],
      subL: [-1.4, -0.55, -1.65],
      subR: [1.4, -0.55, -1.65],
      boat: [0, 0.25, -1.3],
      laptop: [0, 0.12, 0.55],
    },
  },
];
