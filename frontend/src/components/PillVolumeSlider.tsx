import { memo, useRef, useState, type PointerEvent } from "react";
import { SpeakerHigh, SpeakerSlash } from "@phosphor-icons/react";

interface PillVolumeSliderProps {
  value: number;
  min?: number;
  max?: number;
  muted?: boolean;
  disabled?: boolean;
  ariaLabel?: string;
  onChange: (value: number) => void;
  onToggleMute?: () => void;
  onUnmute?: () => void;
}

export const PillVolumeSlider = memo(function PillVolumeSlider({
  value,
  min = 0,
  max = 100,
  muted = false,
  disabled = false,
  ariaLabel = "Volume",
  onChange,
  onToggleMute,
  onUnmute,
}: PillVolumeSliderProps) {
  const trackRef = useRef<HTMLDivElement>(null);
  const draggingRef = useRef(false);
  const [dragging, setDragging] = useState(false);

  const clamped = Math.max(min, Math.min(max, value));
  const ratio = max > min ? (clamped - min) / (max - min) : 0;
  const effectiveRatio = muted ? 0 : ratio;

  const updateFromClientX = (clientX: number) => {
    const el = trackRef.current;
    if (!el || disabled) return;
    const rect = el.getBoundingClientRect();
    const nextRatio = Math.max(0, Math.min(1, (clientX - rect.left) / Math.max(1, rect.width)));
    onChange(Math.round(min + nextRatio * (max - min)));
  };

  const onDown = (e: PointerEvent<HTMLDivElement>) => {
    if (disabled) return;
    e.currentTarget.setPointerCapture(e.pointerId);
    draggingRef.current = true;
    setDragging(true);
    if (muted && onUnmute) onUnmute();
    updateFromClientX(e.clientX);
  };

  const onMove = (e: PointerEvent<HTMLDivElement>) => {
    if (!draggingRef.current) return;
    updateFromClientX(e.clientX);
  };

  const onUp = (e: PointerEvent<HTMLDivElement>) => {
    if (!draggingRef.current) return;
    draggingRef.current = false;
    setDragging(false);
    try {
      e.currentTarget.releasePointerCapture(e.pointerId);
    } catch {
      // Ignore
    }
  };

  return (
    <div
      ref={trackRef}
      role="slider"
      tabIndex={disabled ? -1 : 0}
      aria-label={ariaLabel}
      aria-valuemin={min}
      aria-valuemax={max}
      aria-valuenow={clamped}
      onPointerDown={onDown}
      onPointerMove={onMove}
      onPointerUp={onUp}
      onPointerCancel={onUp}
      onKeyDown={(e) => {
        if (disabled) return;
        const step = e.shiftKey ? 5 : 1;
        if (e.key === "ArrowRight" || e.key === "ArrowUp") {
          e.preventDefault();
          onChange(Math.min(max, clamped + step));
        } else if (e.key === "ArrowLeft" || e.key === "ArrowDown") {
          e.preventDefault();
          onChange(Math.max(min, clamped - step));
        }
      }}
      className={`relative h-8 w-full select-none touch-none overflow-hidden rounded-full bg-[#2a2c32] shadow-inner outline-none focus-visible:ring-2 focus-visible:ring-white/30 ${
        disabled ? "cursor-not-allowed opacity-45" : "cursor-pointer"
      }`}
    >
      <div
        className={`relative flex h-full items-center justify-end rounded-full will-change-[width] ${
          dragging ? "" : "transition-[width] duration-150 ease-out"
        } ${muted ? "bg-zinc-600/75" : "bg-[#f4f5f7] shadow-[0_1px_6px_rgba(0,0,0,0.45)]"}`}
        style={{ width: `calc(32px + (100% - 32px) * ${effectiveRatio.toFixed(4)})` }}
      >
        <span
          className="mr-1 h-6 w-6 shrink-0 rounded-full bg-white shadow-[0_1px_3px_rgba(0,0,0,0.22)]"
          style={{ opacity: effectiveRatio > 0.14 ? 1 : 0 }}
        />
      </div>

      {onToggleMute && !disabled ? (
        <button
          type="button"
          onPointerDown={(e) => e.stopPropagation()}
          onClick={(e) => {
            e.stopPropagation();
            onToggleMute();
          }}
          aria-label={muted ? `Unmute ${ariaLabel}` : `Mute ${ariaLabel}`}
          title={muted ? "Click to unmute (restores level)" : "Click to mute"}
          className={`absolute top-1/2 left-[4px] z-10 flex h-6 w-6 -translate-y-1/2 items-center justify-center rounded-full transition ${
            muted ? "text-zinc-100 hover:bg-white/15" : "text-zinc-900 hover:bg-black/10"
          }`}
        >
          {muted || clamped === 0 ? <SpeakerSlash size={14} weight="fill" /> : <SpeakerHigh size={14} weight="fill" />}
        </button>
      ) : (
        <span
          className={`pointer-events-none absolute top-1/2 left-[9px] -translate-y-1/2 ${
            muted ? "text-zinc-200" : "text-zinc-900"
          }`}
        >
          {muted || clamped === 0 ? <SpeakerSlash size={14} weight="fill" /> : <SpeakerHigh size={14} weight="fill" />}
        </span>
      )}
    </div>
  );
});
