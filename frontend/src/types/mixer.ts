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
  calibrated_delay_ms?: number | null;
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
  saved_pct?: number | null;
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
  default_output_name?: string;
  default_output_is_virtual?: boolean;
}

export interface DevicesResponse {
  inputs: DeviceInfo[];
  outputs: DeviceInfo[];
  selected: StatusPayload["selected"];
  selected_names: StatusPayload["selected_names"];
  has_virtual_cable: boolean;
  win_vols: StatusPayload["win_vols"];
  default_output_name?: string;
  default_output_is_virtual?: boolean;
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
    name: "Desk 4.2 Full Range",
    subtitle: "Primary 2.2 full subs & stereo • Built-in near-field vocals & 20 kHz air",
    badge: "BALANCED",
    patch: {
      position_preset: "desk_triangle",
      protection_max_master: 60,
      protection_ceiling_db: -3,
      intex_vol: 105, intex_balance: 0, intex_width: 108,
      intex_bass_db: 1.0, intex_mid_db: -0.5, intex_treble_db: 1.0,
      boat_vol: 95, boat_mode: "stereo", boat_hpf_hz: 120, boat_lpf_hz: 18000,
      boat_bass_db: 0.0, boat_mid_db: 0.5, boat_treble_db: 0.5,
      laptop_vol: 115, laptop_mode: "stereo", laptop_hpf_hz: 210, laptop_lpf_hz: 18000,
      laptop_bass_db: -1.0, laptop_mid_db: 1.5, laptop_treble_db: 2.0,
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
    name: "Cinema 4.2 Dialogue",
    subtitle: "Wide primary L/R + deep rumble subs • Screen-locked dialogue focus",
    badge: "MOVIES",
    patch: {
      position_preset: "cinema_center",
      protection_max_master: 60,
      protection_ceiling_db: -3,
      intex_vol: 105, intex_balance: 0, intex_width: 132,
      intex_bass_db: 3.0, intex_mid_db: -2.0, intex_treble_db: 0.5,
      boat_vol: 95, boat_mode: "vocal_mid", boat_hpf_hz: 160, boat_lpf_hz: 16000,
      boat_bass_db: -1.5, boat_mid_db: 2.5, boat_treble_db: 1.0,
      laptop_vol: 125, laptop_mode: "vocal_mid", laptop_hpf_hz: 210, laptop_lpf_hz: 18000,
      laptop_bass_db: -2.0, laptop_mid_db: 3.0, laptop_treble_db: 1.5,
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
    name: "Wide Room 4.2 Music",
    subtitle: "Punchy 2.2 dual subs & full stereo • Sparkling near-field treble air",
    badge: "MUSIC",
    patch: {
      position_preset: "wide_room",
      protection_max_master: 60,
      protection_ceiling_db: -3,
      intex_vol: 115, intex_balance: 0, intex_width: 122,
      intex_bass_db: 3.5, intex_mid_db: 0.5, intex_treble_db: 2.0,
      boat_vol: 100, boat_mode: "stereo", boat_hpf_hz: 100, boat_lpf_hz: 18000,
      boat_bass_db: 1.0, boat_mid_db: 0.5, boat_treble_db: 1.5,
      laptop_vol: 120, laptop_mode: "stereo", laptop_hpf_hz: 190, laptop_lpf_hz: 18000,
      laptop_bass_db: -0.5, laptop_mid_db: 1.0, laptop_treble_db: 2.5,
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
    subtitle: "-15 dB spike ceiling • 60% max cap • Tamed dual subs & clear voices",
    badge: "PROTECT",
    patch: {
      position_preset: "night_safe",
      protection_enabled: true,
      protection_max_master: 60,
      protection_ceiling_db: -15.0,
      intex_vol: 80, intex_balance: 0, intex_width: 96,
      intex_bass_db: -4.5, intex_mid_db: 0.5, intex_treble_db: -1.5,
      boat_vol: 80, boat_mode: "vocal_mid", boat_hpf_hz: 200, boat_lpf_hz: 12000,
      boat_bass_db: -2.5, boat_mid_db: 1.5, boat_treble_db: -1.5,
      laptop_vol: 85, laptop_mode: "stereo", laptop_hpf_hz: 230, laptop_lpf_hz: 14000,
      laptop_bass_db: -2.5, laptop_mid_db: 1.5, laptop_treble_db: -1.5,
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
    name: "Offset Desk 4.2",
    subtitle: "Balances off-center primary satellites while Built-in anchors center",
    badge: "OFFSET",
    patch: {
      position_preset: "asym_desk",
      protection_max_master: 60,
      protection_ceiling_db: -3,
      intex_vol: 105, intex_balance: 16, intex_width: 108,
      intex_bass_db: 1.5, intex_mid_db: -0.5, intex_treble_db: 1.0,
      boat_vol: 95, boat_mode: "stereo", boat_hpf_hz: 140, boat_lpf_hz: 18000,
      boat_bass_db: 0.0, boat_mid_db: 1.0, boat_treble_db: 1.0,
      laptop_vol: 125, laptop_mode: "stereo", laptop_hpf_hz: 210, laptop_lpf_hz: 18000,
      laptop_bass_db: -1.0, laptop_mid_db: 1.0, laptop_treble_db: 2.0,
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
    name: "Pure Flat 4.2",
    subtitle: "0 dB flat reference across Primary 2.2 & Built-in 2.0 • 180 Hz sub-guard",
    badge: "FLAT",
    patch: {
      position_preset: "pure_mirror",
      protection_max_master: 60,
      protection_ceiling_db: -3,
      intex_vol: 100, intex_balance: 0, intex_width: 100,
      intex_bass_db: 0.0, intex_mid_db: 0.0, intex_treble_db: 0.0,
      boat_vol: 100, boat_mode: "stereo", boat_hpf_hz: 0, boat_lpf_hz: 18000,
      boat_bass_db: 0.0, boat_mid_db: 0.0, boat_treble_db: 0.0,
      laptop_vol: 100, laptop_mode: "stereo", laptop_hpf_hz: 180, laptop_lpf_hz: 18000,
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
