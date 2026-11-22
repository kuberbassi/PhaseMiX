import { Suspense, lazy, memo, type ReactNode } from "react";
import { SlidersHorizontal, X } from "@phosphor-icons/react";
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
  onDeviceChange: (deviceId: string) => void;
  onPatch: (patch: Partial<MixerState>) => void;
  onSetWinVol: (role: Role, pct: number) => void;
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
  onDeviceChange,
  onPatch,
  onSetWinVol,
}: SpeakerRowProps) {
  const volKey = `${role}_vol` as keyof MixerState;
  const muteKey = `${role}_mute` as keyof MixerState;
  const delayKey = `${role}_delay_ms` as keyof MixerState;

  const vol = Number(mixer[volKey] ?? 100);
  const muted = Boolean(mixer[muteKey]) || mixer.master_mute || (mixer.solo !== null && mixer.solo !== role);
  const delayMs = Number(mixer[delayKey] ?? 0);

  return (
    <div
      className={`relative z-10 focus-within:z-50 rounded-2xl border transition-colors duration-200 ${
        expanded
          ? "border-white/[0.12] bg-[#1e2026] shadow-2xl"
          : "border-white/[0.06] bg-[#1c1e24] hover:border-white/[0.1]"
      }`}
    >
      <div className="flex flex-wrap items-center gap-3 px-4 py-2.5 sm:flex-nowrap">
        <div className="flex min-w-[145px] items-center gap-2.5">
          <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-white/[0.07] text-zinc-200">
            {icon}
          </div>
          <div className="min-w-0">
            <div className="truncate text-[13.5px] font-medium text-zinc-100">{name}</div>
            <div className="truncate text-[11px] text-zinc-500">
              {sublabel} • {delayMs} ms
            </div>
          </div>
        </div>

        <div className="min-w-[160px] flex-1">
          <PillVolumeSlider
            value={vol}
            min={0}
            max={200}
            muted={muted}
            ariaLabel={`${name} Volume`}
            onChange={(v) => onPatch({ [volKey]: v } as Partial<MixerState>)}
          />
        </div>

        <div className="flex items-center gap-2.5">
          <span className="w-10 text-right font-mono text-xs text-zinc-400 tabular-nums">{vol}%</span>
          <LiveLedMeter channel={role} muted={muted} />
        </div>

        <div className="w-full sm:w-[215px]">
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
            onPatch={onPatch}
            onSetWinVol={onSetWinVol}
          />
        </Suspense>
      </SmoothCollapse>
    </div>
  );
});
