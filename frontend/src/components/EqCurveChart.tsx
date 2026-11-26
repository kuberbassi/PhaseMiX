import { memo, useMemo } from "react";
import type { AuxMode, MixerState, Role } from "@/types/mixer";

interface Biquad {
  b0: number;
  b1: number;
  b2: number;
  a1: number;
  a2: number;
}

const FS = 48000;
const W_MIN = Math.log10(20);
const W_MAX = Math.log10(20000);
const SAMPLES = 72;

const FREQ_GRID = Array.from({ length: SAMPLES }, (_, i) =>
  Math.pow(10, W_MIN + ((W_MAX - W_MIN) * i) / (SAMPLES - 1))
);

function norm(b0: number, b1: number, b2: number, a0: number, a1: number, a2: number): Biquad {
  return { b0: b0 / a0, b1: b1 / a0, b2: b2 / a0, a1: a1 / a0, a2: a2 / a0 };
}

function lowShelf(fc: number, db: number, q: number): Biquad | null {
  if (Math.abs(db) < 0.05) return null;
  const a = Math.pow(10, db / 40);
  const w0 = (2 * Math.PI * Math.max(20, Math.min(FS * 0.4, fc))) / FS;
  const sinW = Math.sin(w0);
  const cosW = Math.cos(w0);
  const alpha = sinW / (2 * q);
  const sA = 2 * Math.sqrt(a) * alpha;
  return norm(
    a * (a + 1 - (a - 1) * cosW + sA),
    2 * a * (a - 1 - (a + 1) * cosW),
    a * (a + 1 - (a - 1) * cosW - sA),
    a + 1 + (a - 1) * cosW + sA,
    -2 * (a - 1 + (a + 1) * cosW),
    a + 1 + (a - 1) * cosW - sA
  );
}

function peaking(fc: number, db: number, q: number): Biquad | null {
  if (Math.abs(db) < 0.05) return null;
  const a = Math.pow(10, db / 40);
  const w0 = (2 * Math.PI * Math.max(20, Math.min(FS * 0.45, fc))) / FS;
  const sinW = Math.sin(w0);
  const cosW = Math.cos(w0);
  const alpha = sinW / (2 * q);
  return norm(
    1 + alpha * a,
    -2 * cosW,
    1 - alpha * a,
    1 + alpha / a,
    -2 * cosW,
    1 - alpha / a
  );
}

function highShelf(fc: number, db: number, q: number): Biquad | null {
  if (Math.abs(db) < 0.05) return null;
  const a = Math.pow(10, db / 40);
  const w0 = (2 * Math.PI * Math.max(200, Math.min(FS * 0.45, fc))) / FS;
  const sinW = Math.sin(w0);
  const cosW = Math.cos(w0);
  const alpha = sinW / (2 * q);
  const sA = 2 * Math.sqrt(a) * alpha;
  return norm(
    a * (a + 1 + (a - 1) * cosW + sA),
    -2 * a * (a - 1 + (a + 1) * cosW),
    a * (a + 1 + (a - 1) * cosW - sA),
    a + 1 - (a - 1) * cosW + sA,
    2 * (a - 1 - (a + 1) * cosW),
    a + 1 - (a - 1) * cosW - sA
  );
}

function butterworthHpf(fc: number): Biquad {
  const w0 = (2 * Math.PI * Math.max(20, Math.min(FS * 0.45, fc))) / FS;
  const sinW = Math.sin(w0);
  const cosW = Math.cos(w0);
  const alpha = sinW / Math.SQRT2;
  return norm((1 + cosW) * 0.5, -(1 + cosW), (1 + cosW) * 0.5, 1 + alpha, -2 * cosW, 1 - alpha);
}

function butterworthLpf(fc: number): Biquad {
  const w0 = (2 * Math.PI * Math.max(100, Math.min(FS * 0.45, fc))) / FS;
  const sinW = Math.sin(w0);
  const cosW = Math.cos(w0);
  const alpha = sinW / Math.SQRT2;
  return norm((1 - cosW) * 0.5, 1 - cosW, (1 - cosW) * 0.5, 1 + alpha, -2 * cosW, 1 - alpha);
}

function evalMag2(q: Biquad, w: number): number {
  const c1 = Math.cos(w);
  const s1 = Math.sin(w);
  const c2 = Math.cos(2 * w);
  const s2 = Math.sin(2 * w);
  const nRe = q.b0 + q.b1 * c1 + q.b2 * c2;
  const nIm = -(q.b1 * s1 + q.b2 * s2);
  const dRe = 1 + q.a1 * c1 + q.a2 * c2;
  const dIm = -(q.a1 * s1 + q.a2 * s2);
  return (nRe * nRe + nIm * nIm) / Math.max(1e-12, dRe * dRe + dIm * dIm);
}

function buildRoleFilters(role: Role, mx: MixerState): Biquad[] {
  const filters: Biquad[] = [];
  const bass = Number(mx[`${role}_bass_db` as keyof MixerState] ?? 0);
  const mid = Number(mx[`${role}_mid_db` as keyof MixerState] ?? 0);
  const treble = Number(mx[`${role}_treble_db` as keyof MixerState] ?? 0);

  const fBass = lowShelf(110, bass, 0.707);
  const fMid = peaking(1000, mid, 0.75);
  const fTreble = highShelf(4500, treble, 0.707);
  if (fBass) filters.push(fBass);
  if (fMid) filters.push(fMid);
  if (fTreble) filters.push(fTreble);

  if (role !== "intex") {
    const mode = String(mx[`${role}_mode` as keyof MixerState] ?? "stereo") as AuxMode;
    const hpf = Number(mx[`${role}_hpf_hz` as keyof MixerState] ?? 0);
    const lpf = Number(mx[`${role}_lpf_hz` as keyof MixerState] ?? 18000);
    if (mode === "vocal_mid") {
      const fVoc = peaking(1800, 3.5, 0.75);
      if (fVoc) filters.push(fVoc);
    }
    if (hpf > 20) filters.push(butterworthHpf(hpf));
    if (lpf < 17500) filters.push(butterworthLpf(lpf));
  }
  return filters;
}

const W = 320;
const H = 68;
const DB_MAX = 14;
const DB_MIN = -18;

function rolePath(filters: Biquad[]): string {
  return FREQ_GRID.map((f, idx) => {
    const w = (2 * Math.PI * f) / FS;
    let mag2 = 1;
    for (let i = 0; i < filters.length; i++) {
      mag2 *= evalMag2(filters[i], w);
    }
    const db = Math.max(DB_MIN, Math.min(DB_MAX, 10 * Math.log10(Math.max(1e-9, mag2))));
    const x = ((idx / (SAMPLES - 1)) * W).toFixed(1);
    const y = (H - ((db - DB_MIN) / (DB_MAX - DB_MIN)) * H).toFixed(1);
    return `${idx === 0 ? "M" : "L"}${x},${y}`;
  }).join(" ");
}

const ZERO_Y = H - ((0 - DB_MIN) / (DB_MAX - DB_MIN)) * H;

function freqX(hz: number): number {
  return ((Math.log10(hz) - W_MIN) / (W_MAX - W_MIN)) * W;
}

const ROLE_META: Record<Role, { label: string; color: string }> = {
  intex: { label: "Primary 2.2", color: "#34d399" },
  boat: { label: "Aux 3rd", color: "#60a5fa" },
  laptop: { label: "Built-in 2.0", color: "#e4e4e7" },
};

export const EqCurveChart = memo(function EqCurveChart({
  role,
  mixer,
  showThirdSpeaker = false,
}: {
  role: Role;
  mixer: MixerState;
  showThirdSpeaker?: boolean;
}) {
  const curves = useMemo(() => {
    const roles: Role[] =
      showThirdSpeaker || role === "boat" ? ["intex", "boat", "laptop"] : ["intex", "laptop"];
    return roles.map((r) => ({
      role: r,
      path: rolePath(buildRoleFilters(r, mixer)),
      active: r === role,
      color: ROLE_META[r].color,
    }));
  }, [role, mixer, showThirdSpeaker]);

  const inverted = role !== "intex" && Boolean(mixer[`${role}_invert` as keyof MixerState]);
  const activeColor = inverted ? "#f59e0b" : ROLE_META[role].color;

  return (
    <div className="mt-2.5 rounded-lg border border-white/[0.05] bg-[#0e1014] px-2.5 py-2">
      <div className="mb-1 flex items-center justify-between text-[10px] text-zinc-500">
        <span>Frequency Curve (20 Hz – 20 kHz)</span>
        <div className="flex items-center gap-2">
          {inverted && <span className="font-mono text-amber-300">180° INV</span>}
          <span className="font-mono" style={{ color: activeColor }}>
            {ROLE_META[role].label}
          </span>
        </div>
      </div>
      <svg viewBox={`0 0 ${W} ${H}`} className="h-[62px] w-full overflow-visible">
        {[100, 1000, 10000].map((hz) => {
          const x = freqX(hz).toFixed(1);
          return (
            <g key={hz}>
              <line x1={x} y1={0} x2={x} y2={H} stroke="rgba(255,255,255,0.06)" strokeWidth={1} />
              <text x={x} y={H - 2} fill="#52525b" fontSize={8} textAnchor="middle">
                {hz >= 1000 ? `${hz / 1000}k` : hz}
              </text>
            </g>
          );
        })}
        <line
          x1={0}
          y1={ZERO_Y.toFixed(1)}
          x2={W}
          y2={ZERO_Y.toFixed(1)}
          stroke="rgba(255,255,255,0.12)"
          strokeDasharray="3 3"
          strokeWidth={1}
        />
        {curves
          .filter((c) => !c.active)
          .map((c) => (
            <path
              key={c.role}
              d={c.path}
              fill="none"
              stroke={c.color}
              strokeOpacity={0.22}
              strokeWidth={1.2}
            />
          ))}
        {curves
          .filter((c) => c.active)
          .map((c) => (
            <path
              key={c.role}
              d={c.path}
              fill="none"
              stroke={activeColor}
              strokeWidth={2}
              strokeLinecap="round"
            />
          ))}
      </svg>
    </div>
  );
});

export default EqCurveChart;
