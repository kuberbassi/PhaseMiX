import { Suspense, lazy, memo, type ReactNode } from "react";
import { SlidersHorizontal, SpeakerSlash, WarningCircle, X } from "@phosphor-icons/react";
import GlideSelect, { type GlideSelectOption } from "@/components/GlideSelect";
import { LiveLedMeter } from "@/components/LiveWidgets";
import { PillVolumeSlider } from "@/components/PillVolumeSlider";
import { SmoothCollapse } from "@/components/SmoothCollapse";
import WarmTooltip from "@/components/WarmTooltip";
import type { MixerState, Role, WinEndpointVol } from "@/types/mixer";

const SpeakerEqDrawer = lazy(() => import("@/components/SpeakerEqDrawer"));

interface SpeakerRowProps {
  role: Role;
  name: string;
  sublabel: string;
  icon: ReactNode;
  expanded: boolean;
  onToggleExpand: () => void;
  mixer: MixerState;
  selectedDeviceId: string;
  outputOptions: GlideSelectOption[];
  winVol: WinEndpointVol | null;
  intexIsWireless?: boolean;
  showThirdSpeaker?: boolean;
  onDeviceChange: (deviceId: string) => void;
  onPatch: (patch: Partial<MixerState>) => void;
  onSetWinVol: (role: Role, pct: number) => void;
  onPinWinVol: (role: Role, pin: boolean) => void;
}

export const SpeakerRow = memo(function SpeakerRow({
  role,
  name,
  sublabel,
  icon,
  expanded,
  onToggleExpand,
  mixer,
  selectedDeviceId,
  outputOptions,
  winVol,
  intexIsWireless = false,
  showThirdSpeaker = false,
  onDeviceChange,
  onPatch,
  onSetWinVol,
  onPinWinVol,
}: SpeakerRowProps) {
  const volKey = `${role}_vol` as keyof MixerState;
  const muteKey = `${role}_mute` as keyof MixerState;
  const delayKey = `${role}_delay_ms` as keyof MixerState;

  const vol = Number(mixer[volKey] ?? 100);
  const channelMuted = Boolean(mixer[muteKey]);
  const soloBlocked = mixer.solo !== null && mixer.solo !== role;
  const muted = channelMuted || Boolean(mixer.master_mute) || soloBlocked;
  const delayMs = Number(mixer[delayKey] ?? 0);
  const lowWinAtten = Boolean(
    selectedDeviceId &&
      winVol &&
      (winVol.mute || (winVol.volume_pct < 75 && winVol.db != null && winVol.db < -8))
  );

  const toggleMute = () => {
    if (soloBlocked) {
      onPatch({ solo: null, [muteKey]: false } as Partial<MixerState>);
    } else {
      onPatch({ [muteKey]: !channelMuted } as Partial<MixerState>);
    }
  };

  return (
    <div
      className={`@container relative z-10 rounded-2xl border transition-colors duration-200 focus-within:z-50 ${
        expanded
          ? "border-white/[0.12] bg-[#1e2026] shadow-2xl"
          : "border-white/[0.06] bg-[#1c1e24] hover:border-white/[0.1]"
      }`}
    >
      <div className="flex flex-wrap items-center gap-x-3 gap-y-2 px-3.5 py-2.5 @[600px]:flex-nowrap sm:px-4">
        {/* Identity */}
        <div className="flex min-w-0 flex-1 items-center gap-2.5 @[600px]:w-[162px] @[600px]:flex-none @[720px]:w-[180px]">
          <WarmTooltip
            content={muted ? `Unmute ${name}` : `Mute ${name}`}
            side="right"
            surfaceColor="#242730"
            inkColor="#f4f5f7"
            size="sm"
          >
            <button
              type="button"
              onClick={toggleMute}
              aria-label={muted ? `Unmute ${name}` : `Mute ${name}`}
              className={`flex h-8 w-8 shrink-0 items-center justify-center rounded-lg transition ${
                muted
                  ? "bg-red-500/20 text-red-300 ring-1 ring-red-500/40 hover:bg-red-500/30"
                  : "bg-white/[0.07] text-zinc-200 hover:bg-white/[0.14]"
              }`}
            >
              {muted ? <SpeakerSlash size={16} /> : icon}
            </button>
          </WarmTooltip>
          <div className="min-w-0">
            <div className="truncate text-[13px] font-medium text-zinc-100">{name}</div>
            <div className="truncate text-[11px] text-zinc-500">
              {sublabel} · {delayMs} ms
            </div>
          </div>
        </div>

        {/* Volume Slider — full width on second row below 600px, inline flex-1 above */}
        <div className="order-3 flex w-full items-center gap-2.5 @[600px]:order-2 @[600px]:min-w-[115px] @[600px]:w-auto @[600px]:flex-1">
          <div className="min-w-0 flex-1">
            <PillVolumeSlider
              value={vol}
              min={0}
              max={200}
              muted={muted}
              ariaLabel={`${name} Volume`}
              onChange={(v) => onPatch({ [volKey]: v } as Partial<MixerState>)}
              onToggleMute={toggleMute}
              onUnmute={() => onPatch({ [muteKey]: false } as Partial<MixerState>)}
            />
          </div>
          <div className="flex shrink-0 items-center gap-2">
            <span className="w-9 text-right font-mono text-xs text-zinc-400 tabular-nums">{vol}%</span>
            <LiveLedMeter channel={role} muted={muted} isWireless={role === "intex" && intexIsWireless} />
          </div>
        </div>

        {/* Device Selector + EQ Drawer Trigger */}
        <div className="order-2 flex shrink-0 items-center gap-2 @[600px]:order-3">
          <div className="w-[155px] @[420px]:w-[185px] @[720px]:w-[210px]">
            <GlideSelect
              options={outputOptions}
              value={selectedDeviceId}
              size="md"
              menuWidth={260}
              align="right"
              accentColor="#ffffff"
              surfaceColor="#272930"
              highlightColor="#353842"
              textColor="#eceef2"
              className="w-full [&>button]:w-full [&>button]:justify-between"
              onChange={onDeviceChange}
            />
          </div>

          <WarmTooltip
            content={expanded ? "Close Equalizer & Sync" : "Open 3-Band EQ & Delay"}
            side="left"
            surfaceColor="#242730"
            inkColor="#f4f5f7"
            size="sm"
          >
            <button
              type="button"
              onClick={onToggleExpand}
              aria-label={expanded ? "Close Equalizer" : "Open Equalizer"}
              className={`flex h-8 w-8 shrink-0 items-center justify-center rounded-lg transition ${
                expanded
                  ? "bg-white/10 text-white hover:bg-white/15"
                  : "text-zinc-500 hover:bg-white/[0.06] hover:text-zinc-200"
              }`}
            >
              {expanded ? <X size={15} weight="bold" /> : <SlidersHorizontal size={16} />}
            </button>
          </WarmTooltip>
        </div>
      </div>

      {/* Inline Low Windows Endpoint Volume Alert (catches -130 dB Bluetooth attenuation immediately) */}
      {lowWinAtten && winVol && (
        <div className="flex flex-wrap items-center justify-between gap-2 border-t border-amber-500/15 bg-amber-500/[0.07] px-3.5 py-1.5 text-[11px] text-amber-200/95 sm:px-4">
          <span className="flex min-w-0 items-center gap-1.5">
            <WarningCircle size={13} className="shrink-0 text-amber-400" weight="fill" />
            <span className="truncate">
              {winVol.mute
                ? "Windows speaker endpoint is muted"
                : `Windows level is low (${Math.round(winVol.volume_pct)}% · ${winVol.db} dB) — reduces output`}
            </span>
          </span>
          <button
            type="button"
            onClick={() => onSetWinVol(role, 100)}
            className="shrink-0 rounded-md border border-amber-400/35 bg-amber-500/20 px-2 py-0.5 font-mono text-[10.5px] font-medium text-amber-100 transition hover:bg-amber-500/30"
          >
            Set 100%
          </button>
        </div>
      )}

      {/* Layout-free GPU-smooth drawer open/close transition */}
      <SmoothCollapse open={expanded}>
        <Suspense
          fallback={
            <div className="rounded-b-2xl border-t border-white/[0.06] bg-[#16181d] px-4 py-6 text-center text-xs text-zinc-500">
              Loading Equalizer...
            </div>
          }
        >
          <SpeakerEqDrawer
            role={role}
            mixer={mixer}
            winVol={winVol}
            intexIsWireless={intexIsWireless}
            showThirdSpeaker={showThirdSpeaker}
            onPatch={onPatch}
            onSetWinVol={onSetWinVol}
            onPinWinVol={onPinWinVol}
          />
        </Suspense>
      </SmoothCollapse>
    </div>
  );
});
