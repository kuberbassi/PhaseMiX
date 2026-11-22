import { useSyncExternalStore } from "react";
import type { Meters, Telemetry } from "@/types/mixer";

export interface TelemetrySnapshot {
  running: boolean;
  meters: Meters;
  telemetry: Telemetry;
  master_vol: number;
  master_mute: boolean;
}

const initialSnapshot: TelemetrySnapshot = {
  running: false,
  meters: { in_db: -60, intex_db: -60, boat_db: -60, laptop_db: -60 },
  telemetry: {
    sample_rate: 48000,
    input_status: "Stopped",
    intex_status: "Stopped",
    boat_status: "Stopped",
    laptop_status: "Stopped",
    intex_actual_ms: 0,
    boat_actual_ms: 0,
    laptop_actual_ms: 0,
    intex_rate_ppm: 0,
    boat_rate_ppm: 0,
    laptop_rate_ppm: 0,
  },
  master_vol: 23,
  master_mute: false,
};

let snapshot: TelemetrySnapshot = initialSnapshot;
const listeners = new Set<() => void>();

export const telemetryStore = {
  getSnapshot: () => snapshot,
  subscribe: (fn: () => void) => {
    listeners.add(fn);
    return () => listeners.delete(fn);
  },
  publish: (next: Partial<TelemetrySnapshot>) => {
    snapshot = { ...snapshot, ...next };
    listeners.forEach((fn) => fn());
  },
};

export function useTelemetrySlice<T>(selector: (s: TelemetrySnapshot) => T): T {
  return useSyncExternalStore(
    telemetryStore.subscribe,
    () => selector(telemetryStore.getSnapshot()),
    () => selector(initialSnapshot)
  );
}
