import { Suspense, lazy, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { AnimatePresence, motion } from "motion/react";
import {
  ArrowsClockwise,
  Bluetooth,
  Compass,
  Cube,
  Equalizer,
  FilmSlate,
  Gauge,
  Laptop,
  MoonStars,
  MusicNotes,
  Play,
  Power,
  Record,
  ShieldCheck,
  ShieldWarning,
  Sliders,
  SlidersHorizontal,
  SpeakerHigh,
  Stop,
  X,
} from "@phosphor-icons/react";
import GlideSelect, { type GlideSelectOption } from "@/components/GlideSelect";
import {
  LiveLedMeter,
  LiveOrbBadge,
  LiveProtectionSummary,
  LiveTelemetryFooter,
} from "@/components/LiveWidgets";
import { PillVolumeSlider } from "@/components/PillVolumeSlider";
import { SmoothCollapse } from "@/components/SmoothCollapse";
import { SpeakerRow } from "@/components/SpeakerRow";
import SplitFlapText from "@/components/SplitFlapText";
import SquishSwitch from "@/components/SquishSwitch";
import WarmTooltip, { WarmTooltipGroup } from "@/components/WarmTooltip";
import { mixerApi } from "@/lib/api";
import { telemetryStore } from "@/lib/telemetryStore";
import {
  POSITION_PRESETS,
  type DevicesResponse,
  type MixerState,
  type PositionPreset,
  type Role,
  type StatusPayload,
} from "@/types/mixer";

const Aurora = lazy(() => import("@/components/Aurora"));
const SpeakerStage3D = lazy(() => import("@/components/SpeakerStage3D"));
const SystemProtectionDrawer = lazy(() => import("@/components/SystemProtectionDrawer"));

type PerfMode = "eco" | "balanced" | "studio";

const INITIAL_MIXER: MixerState = {
  master_vol: 23,
  master_mute: false,
  protection_enabled: true,
  protection_max_master: 50,
  protection_ceiling_db: -9,
  position_preset: "custom",
  intex_vol: 100,
  intex_mute: false,
  intex_delay_ms: 222,
  intex_balance: 0,
  intex_bass_db: 0,
  intex_mid_db: 0,
  intex_treble_db: 0,
  intex_width: 100,
  boat_vol: 50,
  boat_mute: false,
  boat_delay_ms: 0,
  boat_mode: "vocal_mid",
  boat_hpf_hz: 350,
  boat_lpf_hz: 18000,
  boat_bass_db: 0,
  boat_mid_db: 0,
  boat_treble_db: 0,
  boat_invert: false,
  laptop_vol: 200,
  laptop_mute: false,
  laptop_delay_ms: 222,
  laptop_mode: "stereo",
  laptop_hpf_hz: 250,
  laptop_lpf_hz: 18000,
  laptop_bass_db: 0,
  laptop_mid_db: 0,
  laptop_treble_db: 0,
  laptop_invert: false,
  solo: null,
  test_mode: "off",
};

function presetIcon(id: string) {
  switch (id) {
    case "desk_triangle":
      return <Compass size={15} />;
    case "cinema_center":
      return <FilmSlate size={15} />;
    case "wide_room":
      return <MusicNotes size={15} />;
    case "night_safe":
      return <MoonStars size={15} />;
    case "asym_desk":
      return <Sliders size={15} />;
    default:
      return <Equalizer size={15} />;
  }
}

export function App() {
  const [mixer, setMixer] = useState<MixerState>(INITIAL_MIXER);
  const [running, setRunning] = useState(false);
  const [devices, setDevices] = useState<DevicesResponse | null>(null);
  const [selected, setSelected] = useState<StatusPayload["selected"]>({
    input_id: "",
    intex_id: "",
    boat_id: "",
    laptop_id: "",
  });
  const [winVols, setWinVols] = useState<StatusPayload["win_vols"]>({
    input: null,
    intex: null,
    boat: null,
    laptop: null,
  });

  const [expandedRole, setExpandedRole] = useState<Role | null>("boat");
  const [systemDrawerOpen, setSystemDrawerOpen] = useState(false);
  const [stageOpen, setStageOpen] = useState(true);
  const [offline, setOffline] = useState(false);
  const [perfMode, setPerfMode] = useState<PerfMode>(() => {
    const saved = localStorage.getItem("sonix_perf_mode");
    return saved === "eco" || saved === "studio" ? saved : "balanced";
  });

  // Leading-edge + trailing flush throttle refs for 0 ms perceived slider latency
  const pendingPatchRef = useRef<Partial<MixerState>>({});
  const patchTimerRef = useRef<number | null>(null);
  const lastPatchSentRef = useRef<number>(0);

  const pendingWinRef = useRef<{ role: Role; pct: number } | null>(null);
  const winTimerRef = useRef<number | null>(null);
  const lastWinSentRef = useRef<number>(0);

  const lastLocalEditRef = useRef<number>(0);
  const failCountRef = useRef<number>(0);

  const applyFullPayload = useCallback((res: StatusPayload, forceMixer = false) => {
    failCountRef.current = 0;
    setOffline((prev) => (prev ? false : prev));
    setRunning(Boolean(res.running));
    if (res.selected) setSelected(res.selected);
    if (res.win_vols) setWinVols(res.win_vols);
    telemetryStore.publish({
      running: Boolean(res.running),
      meters: res.meters,
      telemetry: res.telemetry,
      master_vol: res.mixer?.master_vol ?? 23,
      master_mute: Boolean(res.mixer?.master_mute),
    });
    if (res.mixer && (forceMixer || Date.now() - lastLocalEditRef.current > 450)) {
      setMixer((prev) => ({ ...prev, ...res.mixer }));
    }
  }, []);

  const refreshDevices = useCallback(async () => {
    try {
      const data = await mixerApi.getDevices();
      failCountRef.current = 0;
      setOffline((prev) => (prev ? false : prev));
      setDevices(data);
      setSelected(data.selected);
      if (data.win_vols) setWinVols(data.win_vols);
    } catch {
      // Backend starting up
    }
  }, []);

  useEffect(() => {
    void (async () => {
      await refreshDevices();
      try {
        const st = await mixerApi.getState();
        applyFullPayload(st, true);
      } catch {
        // Ignore initial miss
      }
    })();

    const preloadDrawers = () => {
      void import("@/components/SpeakerEqDrawer");
      void import("@/components/SystemProtectionDrawer");
    };
    const win = window as Window & {
      requestIdleCallback?: (cb: () => void, opts?: { timeout: number }) => number;
      cancelIdleCallback?: (id: number) => void;
    };
    if (typeof win.requestIdleCallback === "function") {
      const idleId = win.requestIdleCallback(preloadDrawers, { timeout: 1200 });
      return () => win.cancelIdleCallback?.(idleId);
    }
    const timer = globalThis.setTimeout(preloadDrawers, 700);
    return () => globalThis.clearTimeout(timer);
  }, [refreshDevices, applyFullPayload]);

  useEffect(() => {
    const flushOnHide = () => {
      if (patchTimerRef.current !== null) {
        window.clearTimeout(patchTimerRef.current);
        patchTimerRef.current = null;
      }
      const batch = pendingPatchRef.current;
      if (Object.keys(batch).length > 0) {
        pendingPatchRef.current = {};
        void fetch("/api/mixer", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify(batch),
          keepalive: true,
        }).catch(() => {});
      }
    };
    window.addEventListener("pagehide", flushOnHide);
    return () => {
      window.removeEventListener("pagehide", flushOnHide);
      if (patchTimerRef.current !== null) window.clearTimeout(patchTimerRef.current);
      if (winTimerRef.current !== null) window.clearTimeout(winTimerRef.current);
    };
  }, []);

  useEffect(() => {
    let alive = true;
    const pollMs = perfMode === "eco" ? 200 : 110;

    const tick = async () => {
      if (document.hidden || !alive) return;
      try {
        const res = await mixerApi.getMeters();
        if (!alive) return;
        failCountRef.current = 0;
        setOffline((prev) => (prev ? false : prev));
        telemetryStore.publish({
          running: res.running,
          meters: res.meters,
          telemetry: res.telemetry,
          master_vol: res.master_vol,
          master_mute: res.master_mute,
        });
        setRunning((prev) => (prev === res.running ? prev : res.running));
        if (Date.now() - lastLocalEditRef.current > 500) {
          setMixer((prev) => {
            if (prev.master_vol === res.master_vol && prev.master_mute === res.master_mute) {
              return prev;
            }
            return { ...prev, master_vol: res.master_vol, master_mute: res.master_mute };
          });
        }
      } catch {
        if (!alive) return;
        failCountRef.current += 1;
        if (failCountRef.current >= 6) {
          setOffline(true);
        }
      }
    };

    const onVisChange = () => {
      if (!document.hidden && alive) void tick();
    };
    document.addEventListener("visibilitychange", onVisChange);
    const id = window.setInterval(() => void tick(), pollMs);
    return () => {
      alive = false;
      document.removeEventListener("visibilitychange", onVisChange);
      window.clearInterval(id);
    };
  }, [perfMode]);

  const flushPatch = useCallback(() => {
    const batch = pendingPatchRef.current;
    if (Object.keys(batch).length === 0) return;
    pendingPatchRef.current = {};
    lastPatchSentRef.current = performance.now();
    void mixerApi.updateMixer(batch);
  }, []);

  const patchMixer = useCallback(
    (patch: Partial<MixerState>) => {
      lastLocalEditRef.current = Date.now();
      const toSend = { ...patch };
      setMixer((prev) => {
        const next = { ...prev, ...toSend };
        if (next.protection_enabled && next.master_vol > next.protection_max_master) {
          next.master_vol = next.protection_max_master;
          toSend.master_vol = next.protection_max_master;
        }
        return next;
      });
      pendingPatchRef.current = { ...pendingPatchRef.current, ...toSend };

      const elapsed = performance.now() - lastPatchSentRef.current;
      if (elapsed >= 18) {
        if (patchTimerRef.current !== null) {
          window.clearTimeout(patchTimerRef.current);
          patchTimerRef.current = null;
        }
        flushPatch();
      } else if (patchTimerRef.current === null) {
        patchTimerRef.current = window.setTimeout(() => {
          patchTimerRef.current = null;
          flushPatch();
        }, 18 - elapsed);
      }
    },
    [flushPatch]
  );

  const flushWinVol = useCallback(() => {
    const item = pendingWinRef.current;
    if (!item) return;
    pendingWinRef.current = null;
    lastWinSentRef.current = performance.now();
    void mixerApi.setWinVol(item.role, item.pct).then((res) => applyFullPayload(res, false));
  }, [applyFullPayload]);

  const handleSetWinVol = useCallback(
    (role: Role, pct: number) => {
      setWinVols((prev) => {
        const cur = prev[role];
        return cur ? { ...prev, [role]: { ...cur, volume_pct: pct, mute: false } } : prev;
      });
      pendingWinRef.current = { role, pct };
      const elapsed = performance.now() - lastWinSentRef.current;
      if (elapsed >= 30) {
        if (winTimerRef.current !== null) {
          window.clearTimeout(winTimerRef.current);
          winTimerRef.current = null;
        }
        flushWinVol();
      } else if (winTimerRef.current === null) {
        winTimerRef.current = window.setTimeout(() => {
          winTimerRef.current = null;
          flushWinVol();
        }, 30 - elapsed);
      }
    },
    [flushWinVol]
  );

  const handleDeviceChange = useCallback(
    async (key: keyof StatusPayload["selected"], val: string) => {
      const next = { ...selected, [key]: val };
      setSelected(next);
      try {
        const res = await mixerApi.setDevices(next);
        applyFullPayload(res, false);
      } catch {
        // Ignored on offline
      }
    },
    [selected, applyFullPayload]
  );

  const onIntexDev = useCallback((v: string) => void handleDeviceChange("intex_id", v), [handleDeviceChange]);
  const onBoatDev = useCallback((v: string) => void handleDeviceChange("boat_id", v), [handleDeviceChange]);
  const onLaptopDev = useCallback((v: string) => void handleDeviceChange("laptop_id", v), [handleDeviceChange]);

  const toggleIntexExpand = useCallback(() => setExpandedRole((c) => (c === "intex" ? null : "intex")), []);
  const toggleBoatExpand = useCallback(() => setExpandedRole((c) => (c === "boat" ? null : "boat")), []);
  const toggleLaptopExpand = useCallback(() => setExpandedRole((c) => (c === "laptop" ? null : "laptop")), []);

  const cyclePerfMode = () => {
    const next: PerfMode = perfMode === "balanced" ? "studio" : perfMode === "studio" ? "eco" : "balanced";
    setPerfMode(next);
    localStorage.setItem("sonix_perf_mode", next);
  };

  const applyPreset = (preset: PositionPreset) => {
    patchMixer(preset.patch);
  };

  const toggleEngine = async () => {
    try {
      const res = running ? await mixerApi.stopEngine() : await mixerApi.startEngine();
      applyFullPayload(res, false);
    } catch {
      setOffline(true);
    }
  };

  const unmuteAll = () => {
    patchMixer({ master_mute: false, intex_mute: false, boat_mute: false, laptop_mute: false, solo: null });
  };

  const inputOptions = useMemo<GlideSelectOption[]>(() => {
    const base: GlideSelectOption[] = [{ value: "", label: "Test Signal Only", tag: "GEN" }];
    for (const d of devices?.inputs ?? []) {
      const nl = d.name.toLowerCase();
      const isVirtual =
        nl.includes("steam streaming speakers") || nl.includes("cable") || nl.includes("voicemeeter");
      if (!isVirtual && String(d.id) !== selected.input_id) continue;

      let shortLabel = d.name.replace(" [Loopback]", "");
      if (nl.includes("steam streaming speakers")) shortLabel = "Steam Virtual Audio";
      else if (nl.includes("cable")) shortLabel = "VB-Audio Cable";
      else if (nl.includes("voicemeeter")) shortLabel = "Voicemeeter VAIO";

      base.push({
        value: String(d.id),
        label: shortLabel,
        tag: d.is_loopback ? "LOOP" : "IN",
      });
    }
    return base;
  }, [devices, selected.input_id]);

  const outputOptions = useMemo<GlideSelectOption[]>(() => {
    const base: GlideSelectOption[] = [{ value: "", label: "Disabled", tag: "OFF" }];
    for (const d of devices?.outputs ?? []) {
      const nl = d.name.toLowerCase();
      const isVirtualSink = nl.includes("steam streaming") || nl.includes("cable input");
      const isSelected = [selected.intex_id, selected.boat_id, selected.laptop_id].includes(String(d.id));
      if (isVirtualSink && !isSelected) continue;

      let shortLabel = d.name;
      let tag = "OUT";
      if (nl.includes("headphones (realtek")) {
        shortLabel = "Realtek AUX Jack";
        tag = "AUX";
      } else if (nl.includes("speakers (realtek")) {
        shortLabel = "Laptop Speakers";
        tag = "PC";
      } else if (nl.includes("stone 358")) {
        shortLabel = "boAt Stone 358 Pro";
        tag = "BT";
      } else if (nl.includes("3.0 bt")) {
        shortLabel = "3.0 BT Receiver";
        tag = "BT";
      } else if (nl.includes("bt") || nl.includes("bluetooth") || nl.includes("anc")) {
        tag = "BT";
      }
      base.push({ value: String(d.id), label: shortLabel, tag });
    }
    return base;
  }, [devices, selected.intex_id, selected.boat_id, selected.laptop_id]);

  const effectiveMaster = mixer.protection_enabled
    ? Math.min(mixer.master_vol, mixer.protection_max_master)
    : mixer.master_vol;
  const masterMax = mixer.protection_enabled ? mixer.protection_max_master : 200;
  const anyMuted = mixer.master_mute || mixer.intex_mute || mixer.boat_mute || mixer.laptop_mute;

  const flapText = useMemo(() => {
    if (anyMuted) return "MUTED";
    if (!running) return "PAUSED";
    const badge = POSITION_PRESETS.find((p) => p.id === mixer.position_preset)?.badge ?? "CUSTOM";
    return `${badge} +${mixer.intex_delay_ms}MS`;
  }, [anyMuted, running, mixer.position_preset, mixer.intex_delay_ms]);

  return (
    <WarmTooltipGroup delay={260} warmWindow={450}>
      <div className="relative min-h-screen bg-[#0f1014] text-zinc-100">
        {/* Full-Width Studio Container */}
        <motion.div
          initial={{ opacity: 0, y: 8 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ duration: 0.28, ease: [0.22, 1, 0.36, 1] }}
          className="relative z-10 mx-auto w-full max-w-[1720px] px-4 py-4 sm:px-6 lg:px-8"
        >
          <div className="relative overflow-visible rounded-3xl border border-white/[0.07] bg-[#16171b]/95 p-5 shadow-[0_28px_80px_rgba(0,0,0,0.72)] sm:p-6">
            {/* Top Smoked Aurora Wave */}
            {perfMode !== "eco" && (
              <div className="pointer-events-none absolute inset-x-0 top-0 h-32 overflow-hidden rounded-t-3xl opacity-45">
                <Suspense fallback={null}>
                  <Aurora
                    colorStops={["#252a38", "#434c63", "#1e222d"]}
                    amplitude={0.62}
                    blend={0.72}
                    height={1.0}
                    speed={perfMode === "studio" ? 0.42 : 0.24}
                    dpr={perfMode === "studio" ? 0.75 : 0.45}
                    layers={2}
                    targetFps={perfMode === "studio" ? 45 : 24}
                    mouseInteraction={false}
                  />
                </Suspense>
              </div>
            )}

            {/* Header Bar */}
            <header className="relative z-10 mb-5 flex flex-wrap items-center justify-between gap-3">
              <div className="flex flex-wrap items-center gap-3">
                <h1 className="text-lg font-semibold tracking-tight text-zinc-100">PhaseMiX</h1>
                <span className="inline-flex items-center gap-1.5 rounded-full border border-white/[0.07] bg-[#202228] px-2.5 py-0.5 text-[11px] font-medium text-zinc-300">
                  <span
                    className={`h-1.5 w-1.5 rounded-full ${
                      running ? "bg-[#22c55e] shadow-[0_0_6px_#22c55e]" : "bg-zinc-500"
                    }`}
                  />
                  {running ? "Active" : "Paused"}
                </span>
                <div className="hidden opacity-90 sm:block">
                  <SplitFlapText
                    text={flapText}
                    padTo={15}
                    fontSize={13}
                    tileColor="#1e2026"
                    textColor="#e4e4e7"
                    tileRadius={4}
                    gap={2}
                    charset="ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789+-:. "
                    flipsPerChar={2}
                  />
                </div>
              </div>

              <div className="flex flex-wrap items-center gap-2">
                <WarmTooltip
                  content="Master cap & spike limiter protection"
                  surfaceColor="#242730"
                  inkColor="#f4f5f7"
                  size="sm"
                >
                  <div className="flex items-center gap-2 rounded-lg border border-white/[0.06] bg-[#202228] px-2.5 py-1">
                    <SquishSwitch
                      checked={mixer.protection_enabled}
                      width={36}
                      height={19}
                      trackColor="#2e3139"
                      trackOnColor="#22c55e"
                      onChange={(c) => patchMixer({ protection_enabled: c })}
                    />
                    <span className="text-[11px] font-medium text-zinc-300">Ear-Safe</span>
                  </div>
                </WarmTooltip>

                <WarmTooltip
                  content="GPU visuals: Balanced, Studio FX, or Eco"
                  surfaceColor="#242730"
                  inkColor="#f4f5f7"
                  size="sm"
                >
                  <button
                    type="button"
                    onClick={cyclePerfMode}
                    className="inline-flex items-center gap-1.5 rounded-lg border border-white/[0.07] bg-[#202228] px-2.5 py-1.5 text-[11px] font-medium text-zinc-300 transition hover:bg-[#2a2d35] hover:text-white"
                  >
                    <Gauge size={13} />
                    {perfMode === "eco" ? "Eco" : perfMode === "studio" ? "Studio FX" : "Balanced"}
                  </button>
                </WarmTooltip>

                <WarmTooltip
                  content="Rescan Windows audio devices & levels"
                  surfaceColor="#242730"
                  inkColor="#f4f5f7"
                  size="sm"
                >
                  <button
                    type="button"
                    onClick={() => void refreshDevices()}
                    className="inline-flex items-center gap-1.5 rounded-lg px-2.5 py-1.5 text-xs font-medium text-zinc-400 transition hover:bg-white/[0.06] hover:text-zinc-100"
                  >
                    <ArrowsClockwise size={14} />
                    Refresh
                  </button>
                </WarmTooltip>

                <WarmTooltip
                  content="Open Windows Sound settings"
                  surfaceColor="#242730"
                  inkColor="#f4f5f7"
                  size="sm"
                >
                  <button
                    type="button"
                    onClick={() => void mixerApi.openSoundSettings()}
                    className="inline-flex items-center gap-1.5 rounded-lg px-2.5 py-1.5 text-xs font-medium text-zinc-400 transition hover:bg-white/[0.06] hover:text-zinc-100"
                  >
                    Sound settings
                  </button>
                </WarmTooltip>

                <WarmTooltip
                  content={running ? "Pause multi-speaker routing" : "Start multi-speaker routing"}
                  surfaceColor="#242730"
                  inkColor="#f4f5f7"
                  size="sm"
                >
                  <button
                    type="button"
                    onClick={toggleEngine}
                    className={`inline-flex items-center gap-1.5 rounded-lg border px-3 py-1.5 text-xs font-medium transition ${
                      running
                        ? "border-white/[0.08] bg-[#24262d] text-zinc-200 hover:bg-[#2d3039]"
                        : "border-emerald-500/40 bg-emerald-500/20 text-emerald-200 hover:bg-emerald-500/30"
                    }`}
                  >
                    {running ? <Stop size={13} weight="fill" /> : <Play size={13} weight="fill" />}
                    {running ? "Stop" : "Start"}
                  </button>
                </WarmTooltip>
              </div>
            </header>

            <AnimatePresence initial={false}>
              {offline && (
                <motion.div
                  key="offline-banner"
                  initial={{ opacity: 0, height: 0, marginBottom: 0 }}
                  animate={{ opacity: 1, height: "auto", marginBottom: 16 }}
                  exit={{ opacity: 0, height: 0, marginBottom: 0 }}
                  transition={{ duration: 0.22, ease: [0.22, 1, 0.36, 1] }}
                  className="overflow-hidden"
                >
                  <div className="flex items-center justify-between gap-3 rounded-xl border border-amber-500/30 bg-amber-500/10 px-3.5 py-2 text-xs text-amber-200">
                    <span className="flex items-center gap-2">
                      <ShieldWarning size={15} className="text-amber-400" />
                      Backend offline — run start_mixer.bat to reconnect.
                    </span>
                    <button
                      type="button"
                      onClick={() => void refreshDevices()}
                      className="rounded-md bg-white/15 px-2.5 py-1 text-[11px] font-medium text-white hover:bg-white/25"
                    >
                      Retry
                    </button>
                  </div>
                </motion.div>
              )}

              {anyMuted && (
                <motion.div
                  key="muted-banner"
                  initial={{ opacity: 0, height: 0, marginBottom: 0 }}
                  animate={{ opacity: 1, height: "auto", marginBottom: 16 }}
                  exit={{ opacity: 0, height: 0, marginBottom: 0 }}
                  transition={{ duration: 0.22, ease: [0.22, 1, 0.36, 1] }}
                  className="overflow-hidden"
                >
                  <div className="flex items-center justify-between gap-3 rounded-xl border border-red-500/30 bg-red-500/10 px-3.5 py-2 text-xs text-red-200">
                    <span className="flex items-center gap-2">
                      <Power size={14} className="text-red-400" />
                      {mixer.master_mute ? "Master audio is muted." : "One or more speaker channels are muted."}
                    </span>
                    <button
                      type="button"
                      onClick={unmuteAll}
                      className="rounded-md bg-white/15 px-2.5 py-1 text-[11px] font-medium text-white hover:bg-white/25"
                    >
                      Unmute
                    </button>
                  </div>
                </motion.div>
              )}
            </AnimatePresence>

            {/* FULL-WIDTH 12-COLUMN STUDIO GRID */}
            <div className="grid grid-cols-1 gap-6 xl:grid-cols-12">
              {/* LEFT 7 COLS: SYSTEM + ACTIVE SPEAKERS */}
              <div className="flex flex-col gap-6 xl:col-span-7">
                {/* SYSTEM SECTION */}
                <section>
                  <div className="mb-2 flex items-center justify-between px-1">
                    <h2 className="text-xs font-medium text-zinc-400">System</h2>
                    <LiveProtectionSummary
                      enabled={mixer.protection_enabled}
                      maxMaster={mixer.protection_max_master}
                      ceilingDb={mixer.protection_ceiling_db}
                    />
                  </div>

                  <div className="flex flex-col gap-2">
                    {/* Output (Master Bus) */}
                    <div className="relative z-30 rounded-2xl border border-white/[0.06] bg-[#1c1e24]">
                      <div className="flex flex-wrap items-center gap-3 px-4 py-2.5 sm:flex-nowrap">
                        <div className="flex min-w-[145px] items-center gap-2.5">
                          <LiveOrbBadge gpuEnabled={perfMode !== "eco"} />
                          <div>
                            <div className="text-[13.5px] font-medium text-zinc-100">Output</div>
                            <div className="text-[11px] text-zinc-500">Master Bus</div>
                          </div>
                        </div>

                        <div className="min-w-[160px] flex-1">
                          <PillVolumeSlider
                            value={effectiveMaster}
                            min={0}
                            max={masterMax}
                            muted={mixer.master_mute}
                            ariaLabel="Master Output Volume"
                            onChange={(v) => patchMixer({ master_vol: v })}
                          />
                        </div>

                        <div className="flex items-center gap-2.5">
                          <span className="w-10 text-right font-mono text-xs text-zinc-400 tabular-nums">
                            {effectiveMaster}%
                          </span>
                          <LiveLedMeter channel="in" muted={mixer.master_mute} />
                        </div>

                        <div className="w-full sm:w-[215px]">
                          <WarmTooltip
                            content="Volume protection & sync calibration"
                            surfaceColor="#242730"
                            inkColor="#f4f5f7"
                            size="sm"
                          >
                            <button
                              type="button"
                              onClick={() => setSystemDrawerOpen((v) => !v)}
                              className="flex h-8 w-full items-center justify-between rounded-lg bg-[#272930] px-2.5 text-xs font-medium text-zinc-200 transition hover:bg-[#31343d]"
                            >
                              <span className="flex items-center gap-1.5 truncate">
                                {mixer.protection_enabled ? (
                                  <ShieldCheck size={14} className="text-[#22c55e]" />
                                ) : (
                                  <ShieldWarning size={14} className="text-amber-400" />
                                )}
                                {mixer.protection_enabled
                                  ? `Guard ${mixer.protection_max_master}% • ${mixer.protection_ceiling_db}dB`
                                  : "Guard Off (200% Max)"}
                              </span>
                            </button>
                          </WarmTooltip>
                        </div>

                        <WarmTooltip
                          content="Volume protection & sync calibration"
                          surfaceColor="#242730"
                          inkColor="#f4f5f7"
                          size="sm"
                        >
                          <button
                            type="button"
                            onClick={() => setSystemDrawerOpen((v) => !v)}
                            className={`flex h-8 w-8 shrink-0 items-center justify-center rounded-lg transition ${
                              systemDrawerOpen
                                ? "bg-white/10 text-white"
                                : "text-zinc-500 hover:bg-white/[0.06] hover:text-zinc-200"
                            }`}
                          >
                            {systemDrawerOpen ? <X size={15} weight="bold" /> : <SlidersHorizontal size={16} />}
                          </button>
                        </WarmTooltip>
                      </div>

                      <SmoothCollapse open={systemDrawerOpen}>
                        <Suspense
                          fallback={
                            <div className="rounded-b-2xl border-t border-white/[0.06] bg-[#16181d] px-4 py-6 text-center text-xs text-zinc-500">
                              Loading Protection Controls...
                            </div>
                          }
                        >
                          <SystemProtectionDrawer mixer={mixer} onPatch={patchMixer} />
                        </Suspense>
                      </SmoothCollapse>
                    </div>

                    {/* Input (System Loopback) */}
                    <div className="relative z-20 flex flex-wrap items-center gap-3 rounded-2xl border border-white/[0.06] bg-[#1c1e24] px-4 py-2.5 sm:flex-nowrap">
                      <div className="flex min-w-[145px] items-center gap-2.5">
                        <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-white/[0.07] text-zinc-300">
                          <Record size={17} />
                        </div>
                        <div>
                          <div className="text-[13.5px] font-medium text-zinc-100">Input</div>
                          <div className="text-[11px] text-zinc-500">System Loopback</div>
                        </div>
                      </div>

                      <div className="min-w-[160px] flex-1">
                        <PillVolumeSlider
                          value={winVols.input ? Math.round(winVols.input.volume_pct) : 98}
                          min={0}
                          max={100}
                          muted={Boolean(winVols.input?.mute)}
                          disabled
                          ariaLabel="Capture Loopback Unity"
                          onChange={() => {}}
                        />
                      </div>

                      <div className="flex items-center gap-2.5">
                        <span className="w-10 text-right font-mono text-xs text-zinc-400 tabular-nums">
                          {winVols.input ? `${Math.round(winVols.input.volume_pct)}%` : "98%"}
                        </span>
                        <LiveLedMeter channel="in" />
                      </div>

                      <div className="w-full sm:w-[215px]">
                        <GlideSelect
                          options={inputOptions}
                          value={selected.input_id}
                          size="md"
                          menuWidth={240}
                          align="right"
                          accentColor="#ffffff"
                          surfaceColor="#272930"
                          highlightColor="#353842"
                          textColor="#eceef2"
                          className="w-full [&>button]:w-full [&>button]:justify-between"
                          onChange={(v) => void handleDeviceChange("input_id", v)}
                        />
                      </div>

                      <div className="flex h-8 w-8 shrink-0 items-center justify-center text-zinc-600">
                        <SlidersHorizontal size={16} />
                      </div>
                    </div>
                  </div>
                </section>

                {/* ACTIVE SPEAKERS SECTION */}
                <section>
                  <div className="mb-2 flex items-center justify-between px-1">
                    <h2 className="text-xs font-medium text-zinc-400">Active speakers</h2>
                    <span className="text-[11px] text-zinc-500">
                      Click sliders icon on any speaker for 3-Band EQ &amp; Delay
                    </span>
                  </div>

                  <div className="flex flex-col gap-2">
                    <SpeakerRow
                      role="intex"
                      name="Primary 2.1 / AUX"
                      sublabel="Wired • L/R + Subs"
                      icon={<SpeakerHigh size={17} />}
                      expanded={expandedRole === "intex"}
                      onToggleExpand={toggleIntexExpand}
                      mixer={mixer}
                      selectedDeviceId={selected.intex_id}
                      outputOptions={outputOptions}
                      winVol={winVols.intex}
                      onDeviceChange={onIntexDev}
                      onPatch={patchMixer}
                      onSetWinVol={handleSetWinVol}
                    />

                    <SpeakerRow
                      role="boat"
                      name="Center / Wireless"
                      sublabel="Bluetooth • Focal Driver"
                      icon={<Bluetooth size={17} />}
                      expanded={expandedRole === "boat"}
                      onToggleExpand={toggleBoatExpand}
                      mixer={mixer}
                      selectedDeviceId={selected.boat_id}
                      outputOptions={outputOptions}
                      winVol={winVols.boat}
                      onDeviceChange={onBoatDev}
                      onPatch={patchMixer}
                      onSetWinVol={handleSetWinVol}
                    />

                    <SpeakerRow
                      role="laptop"
                      name="Built-in Speakers"
                      sublabel="Internal • Desk Tweeters"
                      icon={<Laptop size={17} />}
                      expanded={expandedRole === "laptop"}
                      onToggleExpand={toggleLaptopExpand}
                      mixer={mixer}
                      selectedDeviceId={selected.laptop_id}
                      outputOptions={outputOptions}
                      winVol={winVols.laptop}
                      onDeviceChange={onLaptopDev}
                      onPatch={patchMixer}
                      onSetWinVol={handleSetWinVol}
                    />
                  </div>
                </section>
              </div>

              {/* RIGHT 5 COLS: 3D SOUNDSTAGE + POSITION PRESETS */}
              <div className="flex flex-col gap-4 xl:col-span-5">
                <section className="flex h-full flex-col">
                  <div className="mb-2 flex items-center justify-between px-1">
                    <h2 className="text-xs font-medium text-zinc-400">3D Soundstage &amp; Position Presets</h2>
                    <WarmTooltip
                      content="Toggle live 3D room soundstage visualizer"
                      surfaceColor="#242730"
                      inkColor="#f4f5f7"
                      size="sm"
                    >
                      <button
                        type="button"
                        onClick={() => setStageOpen((v) => !v)}
                        className="inline-flex items-center gap-1.5 rounded-lg bg-[#22242b] px-2.5 py-1 text-[11px] font-medium text-zinc-300 transition hover:bg-[#2b2e37] hover:text-white"
                      >
                        <Cube size={13} />
                        {stageOpen ? "Hide 3D Room" : "Show 3D Room"}
                      </button>
                    </WarmTooltip>
                  </div>

                  <div className="flex flex-1 flex-col gap-4 rounded-2xl border border-white/[0.06] bg-[#1a1c21] p-4">
                    <SmoothCollapse open={stageOpen}>
                      <Suspense
                        fallback={
                          <div className="flex h-[280px] items-center justify-center rounded-xl border border-white/[0.06] bg-[#141519] text-xs text-zinc-500 sm:h-[320px]">
                            Loading 3D Speaker Stage...
                          </div>
                        }
                      >
                        <SpeakerStage3D
                          mixer={mixer}
                          targetFps={perfMode === "eco" ? 15 : perfMode === "studio" ? 60 : 30}
                          paused={!stageOpen}
                        />
                      </Suspense>
                    </SmoothCollapse>

                    <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
                      {POSITION_PRESETS.map((p) => {
                        const active = mixer.position_preset === p.id;
                        return (
                          <button
                            key={p.id}
                            type="button"
                            onClick={() => applyPreset(p)}
                            className={`flex flex-col justify-between gap-1 rounded-xl border p-3 text-left transition ${
                              active
                                ? "border-white/30 bg-white/[0.09] shadow-sm"
                                : "border-white/[0.05] bg-[#141519] hover:border-white/[0.12] hover:bg-[#1d1f26]"
                            }`}
                          >
                            <div className="flex w-full items-center justify-between gap-2">
                              <span
                                className={`flex items-center gap-1.5 text-xs font-medium ${
                                  active ? "text-white" : "text-zinc-200"
                                }`}
                              >
                                {presetIcon(p.id)}
                                {p.name}
                              </span>
                              <span className="rounded bg-white/[0.07] px-1.5 py-0.5 font-mono text-[9px] text-zinc-400">
                                {p.badge}
                              </span>
                            </div>
                            <p className="text-[11px] leading-snug text-zinc-500">{p.subtitle}</p>
                          </button>
                        );
                      })}
                    </div>
                  </div>
                </section>
              </div>
            </div>

            <LiveTelemetryFooter />
          </div>
        </motion.div>
      </div>
    </WarmTooltipGroup>
  );
}

export default App;
