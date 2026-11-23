import { memo, useEffect, useState } from "react";
import {
  ArrowCounterClockwise,
  ArrowsLeftRight,
  DownloadSimple,
  Keyboard,
  Metronome,
  Microphone,
  Stop,
  UploadSimple,
  Waveform,
} from "@phosphor-icons/react";
import CometDial from "@/components/CometDial";
import SquishSwitch from "@/components/SquishSwitch";
import WarmTooltip from "@/components/WarmTooltip";
import type { MixerState } from "@/types/mixer";

interface SystemProtectionDrawerProps {
  mixer: MixerState;
  autostart: boolean;
  calibrating: boolean;
  calStatus: string | null;
  calError: boolean;
  onPatch: (patch: Partial<MixerState>) => void;
  onToggleAutostart: (enabled: boolean) => void;
  onAutoCalibrate: () => void;
  onExportProfile: () => void;
  onImportProfile: () => void;
  onResetFactory: () => void;
}

export const SystemProtectionDrawer = memo(function SystemProtectionDrawer({
  mixer,
  autostart,
  calibrating,
  calStatus,
  calError,
  onPatch,
  onToggleAutostart,
  onAutoCalibrate,
  onExportProfile,
  onImportProfile,
  onResetFactory,
}: SystemProtectionDrawerProps) {
  const [confirmReset, setConfirmReset] = useState(false);

  useEffect(() => {
    if (!confirmReset) return;
    const t = window.setTimeout(() => setConfirmReset(false), 3200);
    return () => window.clearTimeout(t);
  }, [confirmReset]);

  const handleResetClick = () => {
    if (!confirmReset) {
      setConfirmReset(true);
      return;
    }
    setConfirmReset(false);
    onResetFactory();
  };

  const activeTest = mixer.test_mode !== "off";

  return (
    <div className="rounded-b-2xl border-t border-white/[0.06] bg-[#16181d] px-4 py-4">
      <div className="grid grid-cols-1 gap-3.5 md:grid-cols-12">
        {/* LEFT CARD: EAR-SAFE PROTECTION & SYSTEM CONTROLS */}
        <div className="flex flex-col justify-between gap-3 rounded-xl border border-white/[0.05] bg-[#121317] p-3.5 md:col-span-6">
          <div className="flex items-start justify-between gap-2">
            <div>
              <div className="text-xs font-semibold text-zinc-200">Ear-Safe Protection</div>
              <div className="mt-0.5 text-[11px] text-zinc-500">
                Master volume cap &amp; transparent transient ceiling.
              </div>
            </div>
            <span
              className={`shrink-0 rounded-md border px-2 py-0.5 font-mono text-[10px] font-medium ${
                mixer.protection_enabled
                  ? "border-emerald-500/30 bg-emerald-500/10 text-emerald-300"
                  : "border-amber-500/30 bg-amber-500/10 text-amber-300"
              }`}
            >
              {mixer.protection_enabled
                ? `${mixer.protection_max_master}% • ${mixer.protection_ceiling_db} dB`
                : "Bypassed"}
            </span>
          </div>

          <div className="grid grid-cols-2 items-center gap-3 rounded-xl border border-white/[0.04] bg-[#16181e]/75 px-3 py-3">
            <div className="flex flex-col items-center gap-1.5">
              <CometDial
                value={mixer.protection_max_master}
                min={15}
                max={200}
                step={5}
                unit="%"
                label="Max Master Cap"
                size={90}
                thickness={7}
                speed={45}
                cometReach={100}
                accent="#f4f5f7"
                ink="#a1a1aa"
                disabled={!mixer.protection_enabled}
                onChange={(v) => onPatch({ protection_max_master: v })}
              />
              <div className="text-center">
                <div className="text-[11px] font-medium text-zinc-200 leading-tight">Max Master Cap</div>
                <div className="text-[10px] text-zinc-500 leading-tight">Hard ceiling</div>
              </div>
            </div>
            <div className="flex flex-col items-center gap-1.5 border-l border-white/[0.05] pl-3">
              <CometDial
                value={mixer.protection_ceiling_db}
                min={-24}
                max={-1}
                step={1}
                unit="dB"
                label="Peak Limiter Ceiling"
                size={90}
                thickness={7}
                speed={45}
                cometReach={100}
                accent="#f4f5f7"
                ink="#a1a1aa"
                disabled={!mixer.protection_enabled}
                onChange={(v) => onPatch({ protection_ceiling_db: v })}
              />
              <div className="text-center">
                <div className="text-[11px] font-medium text-zinc-200 leading-tight">Spike Ceiling</div>
                <div className="text-[10px] text-zinc-500 leading-tight">Hold limiter</div>
              </div>
            </div>
          </div>

          <div className="grid grid-cols-3 gap-2">
            <div className="flex items-center justify-between gap-2 rounded-lg border border-white/[0.05] bg-[#16181e] px-2.5 py-2">
              <span className="text-[11px] font-medium text-zinc-300 truncate">Ear-Safe</span>
              <SquishSwitch
                checked={mixer.protection_enabled}
                width={36}
                height={20}
                trackColor="#2e3139"
                trackOnColor="#22c55e"
                onChange={(c) => onPatch({ protection_enabled: c })}
              />
            </div>

            <WarmTooltip
              className="w-full"
              content="Launch PhaseMiX silently in the system tray at Windows login"
              surfaceColor="#242730"
              inkColor="#f4f5f7"
              size="sm"
            >
              <div className="flex w-full items-center justify-between gap-2 rounded-lg border border-white/[0.05] bg-[#16181e] px-2.5 py-2">
                <span className="text-[11px] font-medium text-zinc-300 truncate">Tray Boot</span>
                <SquishSwitch
                  checked={autostart}
                  width={36}
                  height={20}
                  trackColor="#2e3139"
                  trackOnColor="#22c55e"
                  onChange={onToggleAutostart}
                />
              </div>
            </WarmTooltip>

            <div className="flex items-center justify-between gap-2 rounded-lg border border-white/[0.05] bg-[#16181e] px-2.5 py-2">
              <span className="text-[11px] font-medium text-zinc-300 truncate">Mute All</span>
              <SquishSwitch
                checked={mixer.master_mute}
                width={36}
                height={20}
                trackColor="#2e3139"
                trackOnColor="#ef4444"
                onChange={(c) => onPatch({ master_mute: c })}
              />
            </div>
          </div>
        </div>

        {/* RIGHT CARD: SYNC & LATENCY CHECK */}
        <div className="flex flex-col justify-between gap-3 rounded-xl border border-white/[0.05] bg-[#121317] p-3.5 md:col-span-6">
          <div className="flex items-start justify-between gap-2">
            <div>
              <div className="text-xs font-semibold text-zinc-200">Sync &amp; Latency Check</div>
              <div className="mt-0.5 text-[11px] text-zinc-500">
                Measure Bluetooth vs wired delay via mic, or tune by ear.
              </div>
            </div>
            {(calStatus || mixer.calibrated_delay_ms != null) && (
              <span
                className={`shrink-0 rounded-md border px-2 py-0.5 font-mono text-[10px] font-medium ${
                  calStatus && calError
                    ? "border-rose-500/30 bg-rose-500/10 text-rose-300"
                    : "border-emerald-500/30 bg-emerald-500/10 text-emerald-300"
                }`}
                title={
                  calStatus ??
                  `Retained mic calibration: ${mixer.calibrated_delay_ms} ms (kept across presets)`
                }
              >
                {mixer.calibrated_delay_ms != null && !calError
                  ? `Saved ${mixer.calibrated_delay_ms} ms`
                  : "Attention"}
              </span>
            )}
          </div>

          <div className="flex flex-col gap-2 rounded-xl border border-white/[0.04] bg-[#16181e]/75 p-2.5">
            <WarmTooltip
              className="w-full"
              content="Plays short BT & AUX frequency chirps and locks AUX / Built-in delay automatically"
              surfaceColor="#242730"
              inkColor="#f4f5f7"
              size="sm"
            >
              <button
                type="button"
                disabled={calibrating}
                onClick={onAutoCalibrate}
                className={`inline-flex w-full items-center justify-center gap-2 rounded-lg border px-3 py-2 text-xs font-semibold transition ${
                  calibrating
                    ? "cursor-wait border-emerald-500/40 bg-emerald-500/15 text-emerald-300 animate-pulse"
                    : "border-white/15 bg-white/[0.08] text-white hover:bg-white/[0.14]"
                }`}
              >
                <Microphone size={14} weight={calibrating ? "fill" : "regular"} />
                {calibrating ? "Measuring BT vs AUX (3 s)..." : "Auto-Align Delay via Mic"}
              </button>
            </WarmTooltip>

            {(calStatus || mixer.calibrated_delay_ms != null) && (
              <div
                className={`truncate rounded-lg px-2.5 py-1.5 text-[11px] font-medium ${
                  calStatus && calError
                    ? "bg-rose-500/10 text-rose-300"
                    : "bg-emerald-500/10 text-emerald-300"
                }`}
              >
                {calStatus ?? `Saved mic calibration: ${mixer.calibrated_delay_ms} ms (kept across presets)`}
              </div>
            )}
          </div>

          <div className="flex flex-col gap-2 rounded-xl border border-white/[0.04] bg-[#16181e]/75 p-2.5">
            <div className="flex items-center justify-between px-0.5 text-[10.5px]">
              <span className="font-medium text-zinc-400">Ear-Tuning Test Tones</span>
              <span
                className={`font-mono ${
                  activeTest ? "text-emerald-300" : "text-zinc-500"
                }`}
              >
                {activeTest ? `ACTIVE: ${mixer.test_mode.toUpperCase()}` : "Idle"}
              </span>
            </div>

            <div className="grid grid-cols-2 gap-2">
              {[
                { id: "click", label: "Sync Click", tip: "Play crisp 2-ms impulse click across all 3 outputs to dial out delay", icon: <Metronome size={14} /> },
                { id: "chirp", label: "Bass + Vocal", tip: "Sweep punchy sub-bass and spoken midrange across speakers", icon: <Waveform size={14} /> },
                { id: "lr", label: "L / C / R", tip: "Alternating stereo cue to verify left, center, and right imaging", icon: <ArrowsLeftRight size={14} /> },
                { id: "off", label: "Stop Test", tip: "Stop active test signal and return to live audio", icon: <Stop size={14} /> },
              ].map((btn) => {
                const isStop = btn.id === "off";
                const active = mixer.test_mode === btn.id && !isStop;
                return (
                  <WarmTooltip
                    key={btn.id}
                    className="w-full"
                    content={btn.tip}
                    surfaceColor="#242730"
                    inkColor="#f4f5f7"
                    size="sm"
                  >
                    <button
                      type="button"
                      onClick={() => onPatch({ test_mode: btn.id as MixerState["test_mode"] })}
                      className={`inline-flex w-full items-center justify-center gap-1.5 rounded-lg border px-2.5 py-2 text-xs font-medium transition ${
                        active
                          ? "border-emerald-400/40 bg-emerald-500/15 text-emerald-200"
                          : isStop && activeTest
                            ? "border-rose-500/35 bg-rose-500/15 text-rose-200 hover:bg-rose-500/25"
                            : isStop
                              ? "border-white/[0.05] bg-[#191b22] text-zinc-500 hover:bg-[#22252e] hover:text-zinc-300"
                              : "border-white/[0.06] bg-[#22242b] text-zinc-300 hover:bg-[#2b2e37] hover:text-white"
                      }`}
                    >
                      {btn.icon}
                      {btn.label}
                    </button>
                  </WarmTooltip>
                );
              })}
            </div>
          </div>
        </div>
      </div>

      <div className="mt-3 flex flex-wrap items-center justify-between gap-2.5 border-t border-white/[0.06] pt-3">
        <div className="flex flex-wrap items-center gap-1.5 text-[11px] text-zinc-400">
          <Keyboard size={14} className="text-zinc-500" />
          <span className="text-zinc-500">Hotkeys:</span>
          <span className="rounded bg-white/[0.06] px-1.5 py-0.5 font-mono text-[10px] text-zinc-200">
            Ctrl+Alt+M
          </span>
          <span className="text-zinc-500">Mute</span>
          <span className="text-zinc-600">·</span>
          <span className="rounded bg-white/[0.06] px-1.5 py-0.5 font-mono text-[10px] text-zinc-200">
            Ctrl+Alt+P
          </span>
          <span className="text-zinc-500">Cycle</span>
          <span className="text-zinc-600">·</span>
          <span className="rounded bg-white/[0.06] px-1.5 py-0.5 font-mono text-[10px] text-zinc-200">
            Ctrl+Alt+N
          </span>
          <span className="text-zinc-500">Night Safe</span>
        </div>

        <div className="flex flex-wrap items-center gap-1.5">
          <WarmTooltip
            content="Save your tuning, delays & device mapping as a shareable .json file"
            surfaceColor="#242730"
            inkColor="#f4f5f7"
            size="sm"
          >
            <button
              type="button"
              onClick={onExportProfile}
              className="inline-flex items-center gap-1.5 rounded-lg border border-white/[0.07] bg-[#1e2027] px-2.5 py-1.5 text-[11px] font-medium text-zinc-200 transition hover:bg-[#292c36]"
            >
              <DownloadSimple size={13} />
              Export .json
            </button>
          </WarmTooltip>

          <WarmTooltip
            content="Load a saved .json PhaseMiX profile (Ear-Safe caps are enforced on import)"
            surfaceColor="#242730"
            inkColor="#f4f5f7"
            size="sm"
          >
            <button
              type="button"
              onClick={onImportProfile}
              className="inline-flex items-center gap-1.5 rounded-lg border border-white/[0.07] bg-[#1e2027] px-2.5 py-1.5 text-[11px] font-medium text-zinc-200 transition hover:bg-[#292c36]"
            >
              <UploadSimple size={13} />
              Import .json
            </button>
          </WarmTooltip>

          <WarmTooltip
            content="Restore factory defaults (224 ms sync, Desk Triangle, Ear-Safe on)"
            surfaceColor="#242730"
            inkColor="#f4f5f7"
            size="sm"
          >
            <button
              type="button"
              onClick={handleResetClick}
              className={`inline-flex items-center gap-1.5 rounded-lg border px-2.5 py-1.5 text-[11px] font-medium transition ${
                confirmReset
                  ? "border-amber-500/45 bg-amber-500/15 text-amber-200"
                  : "border-white/[0.07] bg-[#1e2027] text-zinc-300 hover:bg-[#292c36]"
              }`}
            >
              <ArrowCounterClockwise size={13} />
              {confirmReset ? "Click Again to Reset" : "Reset Factory"}
            </button>
          </WarmTooltip>
        </div>
      </div>
    </div>
  );
});

export default SystemProtectionDrawer;
