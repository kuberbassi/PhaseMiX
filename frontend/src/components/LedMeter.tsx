import { useEffect, useRef, useState } from "react";

interface LedMeterProps {
  db: number;
  level?: number;
  muted?: boolean;
  segments?: number;
}

export function LedMeter({ db, level, muted = false, segments = 10 }: LedMeterProps) {
  const norm = muted
    ? 0
    : typeof level === "number"
      ? Math.max(0, Math.min(1, level))
      : db <= -89
        ? 0
        : Math.pow(Math.max(0, Math.min(1, (db + 54) / 48)), 0.56);
  const litCount = norm > 0.01 ? Math.max(1, Math.round(norm * segments)) : 0;

  const [peakIdx, setPeakIdx] = useState(-1);
  const peakHoldUntilRef = useRef(0);
  const peakRef = useRef(-1);

  useEffect(() => {
    const cur = litCount - 1;
    const now = performance.now();
    if (muted || litCount === 0) {
      if (peakRef.current >= 0) {
        peakRef.current = -1;
        setPeakIdx(-1);
      }
      return;
    }
    if (cur >= peakRef.current) {
      peakRef.current = cur;
      peakHoldUntilRef.current = now + 420;
      setPeakIdx(cur);
    } else if (now > peakHoldUntilRef.current && peakRef.current > cur) {
      const next = peakRef.current - 1;
      peakRef.current = next;
      peakHoldUntilRef.current = now + 50;
      setPeakIdx(next);
    }
  }, [litCount, muted]);

  return (
    <div
      className="flex flex-col-reverse gap-[1.5px]"
      title={db <= -89 ? "-∞ dB" : `${db.toFixed(1)} dB`}
    >
      {Array.from({ length: segments }, (_, i) => {
        const active = i < litCount;
        const isPeak = !active && i === peakIdx && peakIdx >= 0;
        let activeColor = "bg-[#22c55e] shadow-[0_0_5px_rgba(34,197,94,0.45)]";
        let peakColor = "bg-[#22c55e]/85";
        if (i >= segments - 2) {
          activeColor = "bg-[#ef4444] shadow-[0_0_5px_rgba(239,68,68,0.5)]";
          peakColor = "bg-[#ef4444]/90";
        } else if (i >= segments - 4) {
          activeColor = "bg-[#eab308] shadow-[0_0_5px_rgba(234,179,8,0.45)]";
          peakColor = "bg-[#eab308]/90";
        }

        return (
          <span
            key={i}
            className={`h-[2px] w-4 rounded-full ${
              active ? activeColor : isPeak ? peakColor : "bg-white/[0.08]"
            }`}
          />
        );
      })}
    </div>
  );
}

interface LedEqColumnProps {
  label: string;
  db: number;
  min?: number;
  max?: number;
}

export function LedEqColumn({ label, db, min = -12, max = 12 }: LedEqColumnProps) {
  const dots = 11;
  const pct = Math.max(0, Math.min(1, (db - min) / (max - min)));
  const activeIdx = Math.round(pct * (dots - 1));

  return (
    <div className="flex w-11 shrink-0 flex-col items-center gap-1.5">
      <div className="flex flex-col-reverse items-center gap-1 rounded-full border border-white/[0.05] bg-[#14161a] px-1.5 py-2">
        {Array.from({ length: dots }, (_, i) => {
          const isThumb = i === activeIdx;
          const isLit = i <= activeIdx;
          let dotColor = "bg-[#22c55e]";
          if (i >= dots - 2) dotColor = "bg-[#ef4444]";
          else if (i >= dots - 4) dotColor = "bg-[#eab308]";

          return (
            <span
              key={i}
              className={`block rounded-full transition-all duration-150 ${
                isThumb
                  ? "h-2.5 w-2.5 bg-white ring-2 ring-white/30 shadow"
                  : isLit
                    ? `h-1 w-1 ${dotColor}`
                    : "h-1 w-1 bg-white/15"
              }`}
            />
          );
        })}
      </div>
      <span className="w-full whitespace-nowrap text-center font-mono text-[10px] leading-none text-zinc-400">
        {label}
      </span>
    </div>
  );
}
