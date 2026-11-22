import { memo } from "react";
import { ShieldCheck, ShieldWarning, SpeakerHigh, SpeakerSlash } from "@phosphor-icons/react";
import { LedMeter } from "@/components/LedMeter";
import Orb from "@/components/Orb";
import { useTelemetrySlice } from "@/lib/telemetryStore";
import type { Role } from "@/types/mixer";

export const LiveLedMeter = memo(function LiveLedMeter({
  channel,
  muted = false,
}: {
  channel: "in" | "master" | Role;
  muted?: boolean;
}) {
  const db = useTelemetrySlice((s) =>
    channel === "master"
      ? Math.max(s.meters.intex_db, s.meters.boat_db, s.meters.laptop_db)
      : s.meters[`${channel}_db`]
  );
  return <LedMeter db={db} muted={muted} segments={10} />;
});

export const LiveActualMs = memo(function LiveActualMs({ role }: { role: Role }) {
  const ms = useTelemetrySlice((s) => s.telemetry[`${role}_actual_ms`]);
  return <span className="text-zinc-500">(live {ms.toFixed(0)} ms)</span>;
});

export const LiveGuardBadge = memo(function LiveGuardBadge({ role }: { role: Role }) {
  const gr = useTelemetrySlice((s) => s.telemetry[`${role}_gr_db`] ?? 0);
  if (gr >= -0.2) return null;
  return <span className="ml-1.5 text-[11px] text-amber-300">• Guard {gr.toFixed(1)} dB</span>;
});

export const LiveOrbBadge = memo(function LiveOrbBadge({
  gpuEnabled,
  muted = false,
  onToggleMute,
}: {
  gpuEnabled: boolean;
  muted?: boolean;
  onToggleMute?: () => void;
}) {
  // Quantize energy to 16 discrete steps so silence or tiny ripples never wake Orb WebGL uniforms unnecessarily
  const quantizedEnergy = useTelemetrySlice((s) => {
    if (muted) return 0;
    const db = Math.max(s.meters.intex_db, s.meters.boat_db, s.meters.laptop_db);
    if (db <= -55) return 0;
    const norm = Math.max(0, Math.min(1, (db + 55) / 48));
    return Math.round(norm * 16) / 16;
  });

  if (!gpuEnabled || muted) {
    return (
      <button
        type="button"
        onClick={onToggleMute}
        aria-label={muted ? "Unmute Master Output" : "Mute Master Output"}
        className={`flex h-8 w-8 shrink-0 items-center justify-center rounded-lg transition ${
          muted
            ? "bg-red-500/20 text-red-300 ring-1 ring-red-500/40 hover:bg-red-500/30"
            : "bg-white/[0.07] text-zinc-200 hover:bg-white/[0.12]"
        }`}
      >
        {muted ? <SpeakerSlash size={17} /> : <SpeakerHigh size={17} />}
      </button>
    );
  }

  return (
    <button
      type="button"
      onClick={onToggleMute}
      aria-label="Mute Master Output"
      className="relative h-8 w-8 shrink-0 overflow-hidden rounded-full border border-white/10 bg-[#141519] transition hover:border-white/25"
    >
      <Orb
        colors={["#e4e4e7", "#71717a", "#27272a"]}
        wobble={0.1 + quantizedEnergy * 0.4}
        glow={0.3 + quantizedEnergy * 0.55}
        speed={0.3 + quantizedEnergy * 1.0}
        dpr={0.75}
        followCursor={false}
        rotateOnHover={false}
        hoverIntensity={0}
      />
    </button>
  );
});

export const LiveProtectionSummary = memo(function LiveProtectionSummary({
  enabled,
  maxMaster,
  ceilingDb,
}: {
  enabled: boolean;
  maxMaster: number;
  ceilingDb: number;
}) {
  const worstGr = useTelemetrySlice((s) =>
    Math.min(s.telemetry.intex_gr_db ?? 0, s.telemetry.boat_gr_db ?? 0, s.telemetry.laptop_gr_db ?? 0)
  );
  return (
    <span className="flex items-center gap-1.5 text-[11px] text-zinc-500">
      {enabled ? (
        <>
          <ShieldCheck size={13} className="text-[#22c55e]" />
          Ear-Safe Cap {maxMaster}% • {ceilingDb} dB
          {worstGr < -0.2 ? ` (Limiting ${worstGr.toFixed(1)} dB)` : ""}
        </>
      ) : (
        <>
          <ShieldWarning size={13} className="text-amber-400" />
          Protection Off
        </>
      )}
    </span>
  );
});

export const LiveTelemetryFooter = memo(function LiveTelemetryFooter() {
  const tel = useTelemetrySlice((s) => s.telemetry);
  return (
    <footer className="mt-4 flex flex-wrap items-center justify-between gap-2 px-1 font-mono text-[10.5px] text-zinc-500">
      <span>
        {tel.sample_rate} Hz • {tel.input_status}
      </span>
      <span className="flex flex-wrap gap-3">
        <span>Primary: {tel.intex_actual_ms.toFixed(0)} ms</span>
        <span>Center: {tel.boat_actual_ms.toFixed(0)} ms</span>
        <span>Built-in: {tel.laptop_actual_ms.toFixed(0)} ms</span>
      </span>
    </footer>
  );
});
