interface LedMeterProps {
  db: number;
  muted?: boolean;
  segments?: number;
}

export function LedMeter({ db, muted = false, segments = 6 }: LedMeterProps) {
  const norm = muted || db <= -58 ? 0 : Math.max(0, Math.min(1, (db + 55) / 52));
  const litCount = Math.round(norm * segments);

  return (
    <div className="flex flex-col-reverse gap-[2px]" title={db <= -59 ? "-∞ dB" : `${db.toFixed(1)} dB`}>
      {Array.from({ length: segments }, (_, i) => {
        const active = i < litCount;
        let activeColor = "bg-[#22c55e] shadow-[0_0_5px_rgba(34,197,94,0.45)]";
        if (i >= segments - 1) activeColor = "bg-[#ef4444] shadow-[0_0_5px_rgba(239,68,68,0.45)]";
        else if (i >= segments - 2) activeColor = "bg-[#eab308] shadow-[0_0_5px_rgba(234,179,8,0.4)]";

        return (
          <span
            key={i}
            className={`h-[3px] w-3.5 rounded-full transition-colors duration-75 ${
              active ? activeColor : "bg-white/[0.08]"
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
