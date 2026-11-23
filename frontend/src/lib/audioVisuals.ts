/// Matches the backend Windows perceptual curve (`dsp::perceptual_master_gain`).
export function perceptualMasterGain(volPct: number): number {
  if (!Number.isFinite(volPct) || volPct <= 0) return 0;
  if (volPct <= 100) {
    const x = volPct / 100;
    return x * Math.pow(10, -(1 - x));
  }
  return Math.min(2, volPct / 100);
}

/**
 * Converts a live audio meter reading (dBFS, floor -92 dB) into a snappy 0..1 visual amplitude.
 * Partially compensates low perceptual Master % so output LEDs & 3D cones dance responsively at any listening volume.
 */
export function dbToVisualNorm(
  db: number,
  masterVolPct: number,
  isOutput = true,
  muted = false
): number {
  if (muted || !Number.isFinite(db) || db <= -89) return 0;
  let effectiveDb = db;
  if (isOutput) {
    const g = perceptualMasterGain(masterVolPct);
    if (g <= 0.0002) return 0;
    const attenDb = Math.max(0, -20 * Math.log10(g));
    effectiveDb = Math.min(0, db + attenDb * 0.78);
  }
  if (effectiveDb <= -54) return 0;
  const linear = Math.max(0, Math.min(1, (effectiveDb + 54) / 48));
  return Math.pow(linear, 0.56);
}
