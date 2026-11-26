import { memo } from "react";
import { ArrowsCounterClockwise } from "@phosphor-icons/react";
import CometDial from "@/components/CometDial";
import EqCurveChart from "@/components/EqCurveChart";
import GlideSelect from "@/components/GlideSelect";
import { LedEqColumn } from "@/components/LedMeter";
import { LiveActualMs, LiveGuardBadge } from "@/components/LiveWidgets";
import SquishSwitch from "@/components/SquishSwitch";
import WakeSlider from "@/components/WakeSlider";
import WarmTooltip from "@/components/WarmTooltip";
import type { AuxMode, MixerState, Role, WinEndpointVol } from "@/types/mixer";

interface SpeakerEqDrawerProps {
  role: Role;
  mixer: MixerState;
  winVol: WinEndpointVol | null;
  intexIsWireless?: boolean;
  showThirdSpeaker?: boolean;
  onPatch: (patch: Partial<MixerState>) => void;
  onSetWinVol: (role: Role, pct: number) => void;
  onPinWinVol: (role: Role, pin: boolean) => void;
}

const ROLE_MODE_OPTIONS: { value: AuxMode; label: string; tag: string }[] = [
  { value: "stereo", label: "Full Stereo", tag: "L / R" },
  { value: "vocal_mid", label: "Vocal / Mid Focus", tag: "CENTER" },
  { value: "mono_center", label: "Mono Center", tag: "MONO" },
  { value: "side_ambience", label: "Side Ambience", tag: "WIDE" },
];

export const SpeakerEqDrawer = memo(function SpeakerEqDrawer({
  role,
  mixer,
  winVol,
  intexIsWireless = false,
  showThirdSpeaker = false,
  onPatch,
  onSetWinVol,
  onPinWinVol,
}: SpeakerEqDrawerProps) {
  const muteKey = `${role}_mute` as keyof MixerState;
  const delayKey = `${role}_delay_ms` as keyof MixerState;
  const bassKey = `${role}_bass_db` as keyof MixerState;
  const midKey = `${role}_mid_db` as keyof MixerState;
  const trebleKey = `${role}_treble_db` as keyof MixerState;

  const selfMuted = Boolean(mixer[muteKey]);
  const delayMs = Number(mixer[delayKey] ?? 0);
  const bassDb = Number(mixer[bassKey] ?? 0);
  const midDb = Number(mixer[midKey] ?? 0);
  const trebleDb = Number(mixer[trebleKey] ?? 0);
  const isSolo = mixer.solo === role;
  const isAux = role !== "intex";
  const btTargetMs = Math.round(mixer.calibrated_delay_ms ?? 224);
  const isBtEndpoint = Boolean(
    winVol?.name && /bt|bluetooth|stone|anc|earbuds|soundcore|jbl|sony|bose|marshall|airpods/i.test(winVol.name)
  );
  const lowWinDb = Boolean(
    winVol && (winVol.mute || (winVol.volume_pct < 75 && winVol.db != null && winVol.db < -8))
  );

  const modeKey = `${role}_mode` as keyof MixerState;
  const hpfKey = `${role}_hpf_hz` as keyof MixerState;
  const lpfKey = `${role}_lpf_hz` as keyof MixerState;
  const invertKey = `${role}_invert` as keyof MixerState;

  const resetEq = () =>
    onPatch({
      [bassKey]: 0,
      [midKey]: 0,
      [trebleKey]: 0,
    } as Partial<MixerState>);

  const snapDelayTarget =
    role === "laptop"
      ? intexIsWireless
        ? btTargetMs
        : 0
      : role === "intex"
        ? 0
        : 0;

  return (
    <div className="@container rounded-b-2xl border-t border-white/[0.06] bg-[#16181d] px-3.5 py-3.5 sm:px-4">
      {/* Top Bar */}
      <div className="mb-3 flex flex-wrap items-center justify-between gap-2.5">
        <div className="flex flex-wrap items-center gap-3">
          <div className="flex items-center gap-2">
            <SquishSwitch
              checked={!selfMuted}
              width={40}
              height={22}
              trackColor="#2e3139"
              trackOnColor="#22c55e"
              onChange={(active) => onPatch({ [muteKey]: !active } as Partial<MixerState>)}
            />
            <span className="whitespace-nowrap text-xs font-medium text-zinc-200">EQ &amp; Output</span>
          </div>

          <div className="flex items-center gap-1.5">
            <SquishSwitch
              checked={isSolo}
              width={36}
              height={20}
              trackColor="#2e3139"
              trackOnColor="#e4e4e7"
              onChange={(c) => onPatch({ solo: c ? role : null })}
            />
            <span className="text-xs text-zinc-400">Solo</span>
          </div>

          {isAux && (
            <div className="flex items-center gap-1.5">
              <SquishSwitch
                checked={Boolean(mixer[invertKey])}
                width={36}
                height={20}
                trackColor="#2e3139"
                trackOnColor="#eab308"
                onChange={(c) => onPatch({ [invertKey]: c } as Partial<MixerState>)}
              />
              <span className="whitespace-nowrap text-xs text-zinc-400">Phase 180°</span>
            </div>
          )}
        </div>

        <div className="flex flex-wrap items-center gap-2">
          <LiveGuardBadge role={role} />
          {isAux && (
            <GlideSelect
              options={ROLE_MODE_OPTIONS}
              value={String(mixer[modeKey] ?? "stereo")}
              size="sm"
              menuWidth={195}
              align="right"
              accentColor="#ffffff"
              surfaceColor="#272930"
              highlightColor="#353842"
              textColor="#eceef2"
              onChange={(v) => onPatch({ [modeKey]: v as AuxMode } as Partial<MixerState>)}
            />
          )}
          <WarmTooltip
            content="Reset Bass, Mids & Treble to 0 dB"
            surfaceColor="#242730"
            inkColor="#f4f5f7"
            size="sm"
          >
            <button
              type="button"
              onClick={resetEq}
              className="inline-flex items-center gap-1 whitespace-nowrap rounded-lg border border-white/[0.08] bg-[#25272e] px-2.5 py-1 text-xs font-medium text-zinc-300 transition hover:bg-[#2f323b] hover:text-white"
            >
              <ArrowsCounterClockwise size={12} />
              Flat EQ
            </button>
          </WarmTooltip>
        </div>
      </div>

      {/* Main EQ + Acoustic Controls Grid — Container-responsive so cards NEVER clip or collide */}
      <div className="grid grid-cols-1 gap-3.5 @[780px]:grid-cols-12">
        {/* 3-Band EQ Knobs + Segmented LED Columns + Live Frequency Response Curve */}
        <div className="flex min-w-0 flex-col justify-between gap-2 overflow-hidden rounded-xl border border-white/[0.05] bg-[#121317] p-3 @[780px]:col-span-6">
          <div className="grid grid-cols-3 items-end justify-items-center gap-1.5 @[420px]:gap-2">
            {[
              { label: "Bass", freq: "110 Hz", val: bassDb, key: bassKey },
              { label: "Mids", freq: "1.0 kHz", val: midDb, key: midKey },
              { label: "Treble", freq: "4.5 kHz", val: trebleDb, key: trebleKey },
            ].map((band) => (
              <div key={band.label} className="flex min-w-0 items-end justify-center gap-1">
                <div
                  onDoubleClick={() => onPatch({ [band.key]: 0 } as Partial<MixerState>)}
                  title="Double-click to reset 0 dB"
                  className="flex flex-col items-center gap-1.5"
                >
                  <CometDial
                    value={band.val}
                    min={-12}
                    max={12}
                    step={0.5}
                    unit="dB"
                    label={band.label}
                    size={62}
                    thickness={7}
                    speed={45}
                    cometReach={100}
                    accent="#f4f5f7"
                    ink="#a1a1aa"
                    onChange={(v) => onPatch({ [band.key]: v } as Partial<MixerState>)}
                  />
                  <span className="text-[11px] font-medium leading-none text-zinc-300">{band.label}</span>
                </div>
                <div className="hidden @[360px]:block">
                  <LedEqColumn label={band.freq} db={band.val} />
                </div>
              </div>
            ))}
          </div>
          <EqCurveChart role={role} mixer={mixer} showThirdSpeaker={showThirdSpeaker} />
        </div>

        {/* Delay Sync, Crossovers & Windows HW Level */}
        <div className="flex min-w-0 flex-col justify-between gap-3 overflow-hidden rounded-xl border border-white/[0.05] bg-[#121317] p-3 @[780px]:col-span-6">
          <div className="min-w-0">
            <div className="mb-1 flex flex-wrap items-center justify-between gap-1 text-xs">
              <span className="text-zinc-300">Sync Delay</span>
              <span className="font-mono text-zinc-400">
                {delayMs} ms <LiveActualMs role={role} />
              </span>
            </div>
            <div className="flex flex-wrap items-center gap-2">
              <div className="min-w-[120px] flex-1">
                <WakeSlider
                  value={delayMs}
                  min={0}
                  max={450}
                  step={1}
                  bars={22}
                  height={22}
                  restHeight={5}
                  gap={3}
                  glide={0.08}
                  smoothing={35}
                  fillColor="#f4f5f7"
                  trackColor="#2a2c32"
                  crestColor="#22c55e"
                  onChange={(v) => onPatch({ [delayKey]: v } as Partial<MixerState>)}
                />
              </div>
              <div className="flex shrink-0 gap-1">
                {[-2, 2].map((d) => (
                  <button
                    key={d}
                    type="button"
                    onClick={() =>
                      onPatch({ [delayKey]: Math.max(0, Math.min(450, delayMs + d)) } as Partial<MixerState>)
                    }
                    className="rounded-md bg-[#25272e] px-2 py-1 font-mono text-[10px] text-zinc-300 hover:bg-[#31343d]"
                  >
                    {d > 0 ? `+${d}` : d}
                  </button>
                ))}
                <WarmTooltip
                  content={
                    role === "laptop" && intexIsWireless
                      ? `Snap Built-in delay to ${btTargetMs} ms to match Bluetooth`
                      : "Snap to 0 ms reference"
                  }
                  surfaceColor="#242730"
                  inkColor="#f4f5f7"
                  size="sm"
                >
                  <button
                    type="button"
                    onClick={() => onPatch({ [delayKey]: snapDelayTarget } as Partial<MixerState>)}
                    className="rounded-md bg-white/10 px-2 py-1 font-mono text-[10px] font-medium text-zinc-200 hover:bg-white/20"
                  >
                    {snapDelayTarget > 0 ? `${snapDelayTarget}` : "0 ms"}
                  </button>
                </WarmTooltip>
              </div>
            </div>
          </div>

          {role === "intex" ? (
            <div className="grid grid-cols-1 gap-3 @[340px]:grid-cols-2">
              <div className="min-w-0">
                <div className="mb-1 flex justify-between text-[11px] text-zinc-400">
                  <span>Stereo Width</span>
                  <span className="font-mono">{mixer.intex_width}%</span>
                </div>
                <WakeSlider
                  value={mixer.intex_width}
                  min={50}
                  max={150}
                  step={1}
                  bars={14}
                  height={18}
                  restHeight={5}
                  gap={3}
                  glide={0.08}
                  smoothing={35}
                  fillColor="#d4d4d8"
                  trackColor="#2a2c32"
                  onChange={(v) => onPatch({ intex_width: v })}
                />
              </div>
              <div className="min-w-0">
                <div className="mb-1 flex justify-between text-[11px] text-zinc-400">
                  <span>Balance</span>
                  <span className="font-mono">
                    {mixer.intex_balance === 0
                      ? "Center"
                      : mixer.intex_balance < 0
                        ? `L ${-mixer.intex_balance}`
                        : `R ${mixer.intex_balance}`}
                  </span>
                </div>
                <WakeSlider
                  value={mixer.intex_balance}
                  min={-100}
                  max={100}
                  step={1}
                  bars={14}
                  height={18}
                  restHeight={5}
                  gap={3}
                  glide={0.08}
                  smoothing={35}
                  fillColor="#d4d4d8"
                  trackColor="#2a2c32"
                  onChange={(v) => onPatch({ intex_balance: v })}
                />
              </div>
            </div>
          ) : (
            <div className="grid grid-cols-1 gap-3 @[340px]:grid-cols-2">
              <div className="min-w-0">
                <div className="mb-1 flex justify-between text-[11px] text-zinc-400">
                  <span>High-Pass</span>
                  <span className="font-mono">{Number(mixer[hpfKey]) <= 20 ? "Off" : `${mixer[hpfKey]} Hz`}</span>
                </div>
                <WakeSlider
                  value={Number(mixer[hpfKey] ?? 210)}
                  min={0}
                  max={500}
                  step={10}
                  bars={14}
                  height={18}
                  restHeight={5}
                  gap={3}
                  glide={0.08}
                  smoothing={35}
                  fillColor="#d4d4d8"
                  trackColor="#2a2c32"
                  onChange={(v) => onPatch({ [hpfKey]: v } as Partial<MixerState>)}
                />
              </div>
              <div className="min-w-0">
                <div className="mb-1 flex justify-between text-[11px] text-zinc-400">
                  <span>High-Cut</span>
                  <span className="font-mono">
                    {Number(mixer[lpfKey]) >= 17500 ? "Off" : `${(Number(mixer[lpfKey]) / 1000).toFixed(1)} kHz`}
                  </span>
                </div>
                <WakeSlider
                  value={Number(mixer[lpfKey] ?? 18000)}
                  min={3000}
                  max={18000}
                  step={500}
                  bars={14}
                  height={18}
                  restHeight={5}
                  gap={3}
                  glide={0.08}
                  smoothing={35}
                  fillColor="#d4d4d8"
                  trackColor="#2a2c32"
                  onChange={(v) => onPatch({ [lpfKey]: v } as Partial<MixerState>)}
                />
              </div>
            </div>
          )}

          <div className="min-w-0">
            <div className="mb-1 flex flex-wrap items-center justify-between gap-1.5 text-[11px] text-zinc-400">
              <div className="flex items-center gap-1.5">
                <span>Windows Device Level</span>
                {winVol && winVol.name && (
                  <WarmTooltip
                    content={
                      winVol.saved_pct != null
                        ? `Auto-restores ${winVol.saved_pct}% on startup & BT reconnect — click to unpin`
                        : "Save current Windows level so PhaseMiX restores it across boots & BT reconnects"
                    }
                    surfaceColor="#242730"
                    inkColor="#f4f5f7"
                    size="sm"
                  >
                    <button
                      type="button"
                      onClick={() => onPinWinVol(role, winVol.saved_pct == null)}
                      className={`rounded-md px-1.5 py-0.5 font-mono text-[10px] transition ${
                        winVol.saved_pct != null
                          ? "bg-emerald-500/15 text-emerald-300 ring-1 ring-emerald-500/30 hover:bg-emerald-500/25"
                          : "bg-white/[0.06] text-zinc-400 hover:bg-white/[0.12] hover:text-zinc-200"
                      }`}
                    >
                      {winVol.saved_pct != null
                        ? `Auto ${Math.round(winVol.saved_pct)}%`
                        : `Pin ${Math.round(winVol.volume_pct)}%`}
                    </button>
                  </WarmTooltip>
                )}
              </div>
              <span
                className={`font-mono tabular-nums ${
                  lowWinDb ? "text-amber-300" : isBtEndpoint && winVol && winVol.volume_pct >= 85 ? "text-emerald-300" : ""
                }`}
              >
                {winVol ? `${winVol.volume_pct}%${winVol.db != null ? ` (${winVol.db} dB)` : ""}` : "--"}
              </span>
            </div>
            <div className="flex flex-wrap items-center gap-2">
              <div className="min-w-[110px] flex-1">
                <WakeSlider
                  value={Math.min(100, winVol ? Math.round(winVol.volume_pct) : 80)}
                  min={0}
                  max={100}
                  step={1}
                  bars={24}
                  height={20}
                  restHeight={5}
                  gap={3}
                  glide={0.08}
                  smoothing={35}
                  fillColor={lowWinDb ? "#fbbf24" : "#d4d4d8"}
                  trackColor="#2a2c32"
                  onChange={(v) => onSetWinVol(role, v)}
                />
              </div>
              <div className="flex shrink-0 gap-1">
                {[35, 65, 85, 100].map((pct) => (
                  <button
                    key={pct}
                    type="button"
                    onClick={() => onSetWinVol(role, pct)}
                    className={`rounded-md px-1.5 py-0.5 font-mono text-[10px] transition ${
                      isBtEndpoint && pct === 100 && lowWinDb
                        ? "bg-amber-500/20 text-amber-200 ring-1 ring-amber-400/40 hover:bg-amber-500/30"
                        : "bg-[#25272e] text-zinc-300 hover:bg-[#31343d]"
                    }`}
                  >
                    {pct}%
                  </button>
                ))}
              </div>
            </div>
            {isBtEndpoint && lowWinDb && (
              <div className="mt-1.5 text-[10.5px] leading-snug text-amber-300/90">
                Windows BT uses a steep -130 dB taper — keep this at 85–100% and control loudness with Master Bus.
              </div>
            )}
          </div>
        </div>
      </div>
    </div>
  );
});

export default SpeakerEqDrawer;
