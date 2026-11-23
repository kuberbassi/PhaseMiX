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

  return (
    <div className="rounded-b-2xl border-t border-white/[0.06] bg-[#16181d] px-4 py-4">
      <div className="grid grid-cols-1 gap-4 md:grid-cols-12">
        <div className="flex flex-col justify-between gap-3 rounded-xl border border-white/[0.05] bg-[#121317] p-3.5 md:col-span-7">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <div className="flex items-center gap-2.5">
              <SquishSwitch
                checked={mixer.protection_enabled}
                width={44}
                height={24}
                trackColor="#2e3139"
                trackOnColor="#22c55e"
                onChange={(c) => onPatch({ protection_enabled: c })}
              />
              <span className="text-xs font-medium text-zinc-200">Ear-Safe Volume Protection</span>
            </div>
            <div className="flex items-center gap-3">
              <WarmTooltip
                content="Launch PhaseMiX silently in the system tray at Windows login"
                surfaceColor="#242730"
                inkColor="#f4f5f7"
                size="sm"
              >
                <div className="flex items-center gap-1.5">
                  <SquishSwitch
                    checked={autostart}
                    width={38}
                    height={21}
                    trackColor="#2e3139"
                    trackOnColor="#22c55e"
                    onChange={onToggleAutostart}
                  />
                  <span className="text-xs text-zinc-400">Startup Tray</span>
                </div>
              </WarmTooltip>
              <div className="flex items-center gap-1.5">
                <SquishSwitch
                  checked={mixer.master_mute}
                  width={38}
                  height={21}
                  trackColor="#2e3139"
                  trackOnColor="#ef4444"
                  onChange={(c) => onPatch({ master_mute: c })}
                />
                <span className="text-xs text-zinc-400">Mute All</span>
              </div>
            </div>
          </div>

          <div className="flex items-center justify-around gap-4 pt-1">
            <div className="flex flex-col items-center gap-1">
              <CometDial
                value={mixer.protection_max_master}
                min={15}
                max={200}
                step={5}
                unit="%"
                label="Max Master Cap"
                size={78}
                thickness={7}
                speed={45}
                cometReach={100}
                accent="#f4f5f7"
                ink="#a1a1aa"
                disabled={!mixer.protection_enabled}
                onChange={(v) => onPatch({ protection_max_master: v })}
              />
              <span className="text-[11px] text-zinc-400">Max Master Cap</span>
            </div>
            <div className="flex flex-col items-center gap-1">
              <CometDial
                value={mixer.protection_ceiling_db}
                min={-24}
                max={-1}
                step={1}
                unit="dB"
                label="Peak Limiter Ceiling"
                size={78}
                thickness={7}
                speed={45}
                cometReach={100}
                accent="#f4f5f7"
                ink="#a1a1aa"
                disabled={!mixer.protection_enabled}
                onChange={(v) => onPatch({ protection_ceiling_db: v })}
              />
              <span className="text-[11px] text-zinc-400">Spike Ceiling</span>
            </div>
          </div>
        </div>

        <div className="flex flex-col justify-between gap-2.5 rounded-xl border border-white/[0.05] bg-[#121317] p-3.5 md:col-span-5">
          <div className="flex items-start justify-between gap-2">
            <div>
              <div className="text-xs font-medium text-zinc-200">Sync &amp; Mic Auto-Align</div>
              <div className="mt-0.5 text-[11px] text-zinc-500">
                Measure Bluetooth vs wired lag with your laptop mic, or tune by ear.
              </div>
            </div>
          </div>

          <WarmTooltip
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
              {calibrating ? "Listening via Mic (2.5 s)..." : "Auto-Align Delay via Mic"}
            </button>
          </WarmTooltip>

          {calStatus && (
            <div
              className={`truncate rounded-md px-2 py-1 text-[11px] font-medium ${
                calError
                  ? "bg-rose-500/10 text-rose-300"
                  : "bg-emerald-500/10 text-emerald-300"
              }`}
              title={calStatus}
            >
              {calStatus}
            </div>
          )}

          <div className="grid grid-cols-2 gap-2">
            {[
              { id: "click", label: "Sync Click", tip: "Play crisp 2-ms impulse click across all 3 outputs to dial out delay", icon: <Metronome size={14} /> },
              { id: "chirp", label: "Bass + Vocal", tip: "Sweep punchy sub-bass and spoken midrange across speakers", icon: <Waveform size={14} /> },
              { id: "lr", label: "L / C / R", tip: "Alternating stereo cue to verify left, center, and right imaging", icon: <ArrowsLeftRight size={14} /> },
              { id: "off", label: "Stop Test", tip: "Stop active test signal and return to live audio", icon: <Stop size={14} /> },
            ].map((btn) => {
              const active = mixer.test_mode === btn.id && btn.id !== "off";
              return (
                <WarmTooltip
                  key={btn.id}
                  content={btn.tip}
                  surfaceColor="#242730"
                  inkColor="#f4f5f7"
                  size="sm"
                >
                  <button
                    type="button"
                    onClick={() => onPatch({ test_mode: btn.id as MixerState["test_mode"] })}
                    className={`inline-flex items-center justify-center gap-1.5 rounded-lg border px-2.5 py-2 text-xs font-medium transition ${
                      active
                        ? "border-white/30 bg-white/15 text-white"
                        : "border-white/[0.06] bg-[#22242b] text-zinc-300 hover:bg-[#2b2e37]"
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
