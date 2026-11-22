import { memo } from "react";
import { ArrowsLeftRight, Metronome, Stop, Waveform } from "@phosphor-icons/react";
import CometDial from "@/components/CometDial";
import SquishSwitch from "@/components/SquishSwitch";
import WarmTooltip from "@/components/WarmTooltip";
import type { MixerState } from "@/types/mixer";

interface SystemProtectionDrawerProps {
  mixer: MixerState;
  autostart: boolean;
  onPatch: (patch: Partial<MixerState>) => void;
  onToggleAutostart: (enabled: boolean) => void;
}

export const SystemProtectionDrawer = memo(function SystemProtectionDrawer({
  mixer,
  autostart,
  onPatch,
  onToggleAutostart,
}: SystemProtectionDrawerProps) {
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

        <div className="flex flex-col justify-between gap-3 rounded-xl border border-white/[0.05] bg-[#121317] p-3.5 md:col-span-5">
          <div>
            <div className="text-xs font-medium text-zinc-200">Sync Calibration Test</div>
            <div className="mt-0.5 text-[11px] text-zinc-500">
              Use Sync Click to align wired and built-in speakers to Bluetooth latency.
            </div>
          </div>
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
    </div>
  );
});

export default SystemProtectionDrawer;
