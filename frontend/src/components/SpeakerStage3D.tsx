import { memo, useEffect, useRef } from "react";
import * as THREE from "three";
import { telemetryStore } from "@/lib/telemetryStore";
import { POSITION_PRESETS, type MixerState } from "@/types/mixer";

interface SpeakerStage3DProps {
  mixer: MixerState;
  targetFps?: number;
  paused?: boolean;
}

function dbToAmp(db: number): number {
  if (db <= -58) return 0;
  return Math.max(0, Math.min(1, (db + 55) / 50));
}

export const SpeakerStage3D = memo(function SpeakerStage3D({
  mixer,
  targetFps = 30,
  paused = false,
}: SpeakerStage3DProps) {
  const mountRef = useRef<HTMLDivElement | null>(null);
  const mixerRef = useRef(mixer);
  const fpsRef = useRef(targetFps);
  const pausedRef = useRef(paused);
  const wakeRef = useRef<(() => void) | null>(null);
  mixerRef.current = mixer;
  fpsRef.current = targetFps;
  pausedRef.current = paused;

  useEffect(() => {
    wakeRef.current?.();
  }, [paused]);

  useEffect(() => {
    const container = mountRef.current;
    if (!container) return;

    const scene = new THREE.Scene();
    scene.fog = new THREE.FogExp2(0x141519, 0.08);

    const camera = new THREE.PerspectiveCamera(
      40,
      container.clientWidth / Math.max(1, container.clientHeight),
      0.1,
      40
    );
    camera.position.set(0, 3.0, 5.1);

    // Lightweight low-power GPU config at dpr=1 with zero shadow-pass overhead
    const renderer = new THREE.WebGLRenderer({
      antialias: true,
      alpha: false,
      powerPreference: "low-power",
      stencil: false,
    });
    renderer.setClearColor(0x141519, 1);
    renderer.setPixelRatio(1);
    renderer.setSize(container.clientWidth, container.clientHeight);
    renderer.shadowMap.enabled = false;
    container.appendChild(renderer.domElement);

    const hemi = new THREE.HemisphereLight(0xf4f5f7, 0x16181d, 0.95);
    scene.add(hemi);
    const keyLight = new THREE.DirectionalLight(0xffffff, 1.3);
    keyLight.position.set(3.2, 6, 4.2);
    scene.add(keyLight);
    const rimLight = new THREE.DirectionalLight(0xd4d8e2, 0.65);
    rimLight.position.set(-3.5, 2.5, -3.5);
    scene.add(rimLight);

    const grid = new THREE.GridHelper(10, 16, 0x2a2d35, 0x1c1e24);
    grid.position.y = -0.82;
    scene.add(grid);

    // Shared geometries & materials for minimal GPU buffer footprint and deterministic disposal
    const geometries: THREE.BufferGeometry[] = [];
    const materials: THREE.Material[] = [];
    const trackGeo = <T extends THREE.BufferGeometry>(g: T): T => {
      geometries.push(g);
      return g;
    };
    const trackMat = <T extends THREE.Material>(m: T): T => {
      materials.push(m);
      return m;
    };

    const floorGeo = trackGeo(new THREE.CircleGeometry(3.8, 36));
    const floorMat = trackMat(
      new THREE.MeshStandardMaterial({ color: 0x16181c, roughness: 0.85, metalness: 0.12 })
    );
    const floor = new THREE.Mesh(floorGeo, floorMat);
    floor.rotation.x = -Math.PI / 2;
    floor.position.y = -0.83;
    scene.add(floor);

    const matteCabinet = trackMat(new THREE.MeshStandardMaterial({ color: 0x1e2026, roughness: 0.52, metalness: 0.35 }));
    const anodizedSilver = trackMat(new THREE.MeshStandardMaterial({ color: 0x626773, roughness: 0.32, metalness: 0.78 }));
    const coneMat = trackMat(new THREE.MeshStandardMaterial({ color: 0x121318, roughness: 0.68, metalness: 0.2 }));
    const makeRingMat = () =>
      trackMat(
        new THREE.MeshStandardMaterial({
          color: 0xd4d4d8,
          emissive: 0xe4e4e7,
          emissiveIntensity: 0.15,
          roughness: 0.25,
          metalness: 0.85,
        })
      );
    const ledGreen = trackMat(
      new THREE.MeshStandardMaterial({ color: 0x22c55e, emissive: 0x22c55e, emissiveIntensity: 0.85 })
    );

    // Shared speaker geometries
    const satBoxGeo = trackGeo(new THREE.BoxGeometry(0.42, 0.76, 0.44));
    const satBezelGeo = trackGeo(new THREE.BoxGeometry(0.37, 0.69, 0.03));
    const satConeGeo = trackGeo(new THREE.CylinderGeometry(0.11, 0.06, 0.06, 16));
    const satTweetGeo = trackGeo(new THREE.SphereGeometry(0.052, 12, 10));
    const satRingGeo = trackGeo(new THREE.TorusGeometry(0.13, 0.012, 8, 20));

    const subBoxGeo = trackGeo(new THREE.BoxGeometry(0.68, 0.72, 0.72));
    const subConeGeo = trackGeo(new THREE.CylinderGeometry(0.22, 0.11, 0.08, 18));
    const subPortGeo = trackGeo(new THREE.CylinderGeometry(0.058, 0.058, 0.05, 12));
    const subRingGeo = trackGeo(new THREE.TorusGeometry(0.235, 0.014, 8, 22));

    const waveGeo = trackGeo(new THREE.RingGeometry(0.28, 0.305, 28));
    const listenerGeo = trackGeo(new THREE.TorusGeometry(0.3, 0.01, 8, 28));

    function buildSatellite() {
      const group = new THREE.Group();
      group.add(new THREE.Mesh(satBoxGeo, matteCabinet));
      const bezel = new THREE.Mesh(satBezelGeo, anodizedSilver);
      bezel.position.z = 0.22;
      group.add(bezel);
      const cone = new THREE.Mesh(satConeGeo, coneMat);
      cone.rotation.x = Math.PI / 2;
      cone.position.set(0, -0.08, 0.23);
      group.add(cone);
      const tweeter = new THREE.Mesh(satTweetGeo, anodizedSilver);
      tweeter.position.set(0, 0.18, 0.23);
      group.add(tweeter);
      const ringMat = makeRingMat();
      const ring = new THREE.Mesh(satRingGeo, ringMat);
      ring.position.set(0, -0.08, 0.242);
      group.add(ring);
      return { group, cone, ringMat };
    }

    function buildSubwoofer() {
      const group = new THREE.Group();
      group.add(new THREE.Mesh(subBoxGeo, matteCabinet));
      const cone = new THREE.Mesh(subConeGeo, coneMat);
      cone.rotation.x = Math.PI / 2;
      cone.position.set(0, -0.04, 0.35);
      group.add(cone);
      const port = new THREE.Mesh(subPortGeo, anodizedSilver);
      port.rotation.x = Math.PI / 2;
      port.position.set(0, 0.23, 0.35);
      group.add(port);
      const ringMat = makeRingMat();
      const ring = new THREE.Mesh(subRingGeo, ringMat);
      ring.position.set(0, -0.04, 0.365);
      group.add(ring);
      return { group, cone, ringMat };
    }

    function buildBoatStone() {
      const group = new THREE.Group();
      const cylGeo = trackGeo(new THREE.CylinderGeometry(0.23, 0.23, 0.92, 18));
      const grillGeo = trackGeo(new THREE.CylinderGeometry(0.235, 0.235, 0.66, 18, 1, true));
      const bRingGeo = trackGeo(new THREE.TorusGeometry(0.21, 0.018, 8, 22));
      const capGeo = trackGeo(new THREE.CylinderGeometry(0.17, 0.17, 0.03, 16));
      const ledGeo = trackGeo(new THREE.BoxGeometry(0.14, 0.025, 0.025));

      const cyl = new THREE.Mesh(cylGeo, matteCabinet);
      cyl.rotation.z = Math.PI / 2;
      group.add(cyl);
      const grill = new THREE.Mesh(grillGeo, anodizedSilver);
      grill.rotation.z = Math.PI / 2;
      group.add(grill);

      const ringMat = makeRingMat();
      const ringL = new THREE.Mesh(bRingGeo, ringMat);
      ringL.rotation.y = Math.PI / 2;
      ringL.position.x = -0.45;
      group.add(ringL);
      const ringR = new THREE.Mesh(bRingGeo, ringMat);
      ringR.rotation.y = Math.PI / 2;
      ringR.position.x = 0.45;
      group.add(ringR);

      const capL = new THREE.Mesh(capGeo, coneMat);
      capL.rotation.z = Math.PI / 2;
      capL.position.x = -0.46;
      group.add(capL);
      const capR = new THREE.Mesh(capGeo, coneMat);
      capR.rotation.z = Math.PI / 2;
      capR.position.x = 0.46;
      group.add(capR);

      const led = new THREE.Mesh(ledGeo, ledGreen);
      led.position.set(0, 0.22, 0.08);
      group.add(led);
      return { group, capL, capR, ringMat };
    }

    function buildLaptop() {
      const group = new THREE.Group();
      const baseGeo = trackGeo(new THREE.BoxGeometry(1.05, 0.05, 0.72));
      const lidGeo = trackGeo(new THREE.BoxGeometry(1.05, 0.64, 0.03));
      const screenGeo = trackGeo(new THREE.PlaneGeometry(0.95, 0.56));
      const barGeo = trackGeo(new THREE.BoxGeometry(0.055, 0.016, 0.44));
      const screenMat = trackMat(
        new THREE.MeshStandardMaterial({ color: 0x16181e, emissive: 0x272a34, emissiveIntensity: 0.45 })
      );

      group.add(new THREE.Mesh(baseGeo, anodizedSilver));
      const lidGroup = new THREE.Group();
      lidGroup.position.set(0, 0.02, -0.35);
      lidGroup.rotation.x = -0.28;
      const lid = new THREE.Mesh(lidGeo, matteCabinet);
      lid.position.set(0, 0.32, 0);
      lidGroup.add(lid);
      const screen = new THREE.Mesh(screenGeo, screenMat);
      screen.position.set(0, 0.32, 0.018);
      lidGroup.add(screen);
      group.add(lidGroup);

      const barMat = makeRingMat();
      const barL = new THREE.Mesh(barGeo, barMat);
      barL.position.set(-0.46, 0.032, 0);
      group.add(barL);
      const barR = new THREE.Mesh(barGeo, barMat);
      barR.position.set(0.46, 0.032, 0);
      group.add(barR);
      return { group, barMat };
    }

    function buildWaveRing() {
      const mat = trackMat(
        new THREE.MeshBasicMaterial({ color: 0xe4e4e7, side: THREE.DoubleSide, transparent: true, opacity: 0.2 })
      );
      const m = new THREE.Mesh(waveGeo, mat);
      m.rotation.x = -Math.PI / 2;
      m.position.y = -0.8;
      return { mesh: m, mat };
    }

    const intexL = buildSatellite();
    const intexR = buildSatellite();
    const subL = buildSubwoofer();
    const subR = buildSubwoofer();
    const boat = buildBoatStone();
    const laptop = buildLaptop();

    const waveIntex = buildWaveRing();
    const waveBoat = buildWaveRing();
    const waveLaptop = buildWaveRing();

    const listenerMat = trackMat(
      new THREE.MeshBasicMaterial({ color: 0x71717a, transparent: true, opacity: 0.45 })
    );
    const listener = new THREE.Mesh(listenerGeo, listenerMat);
    listener.rotation.x = Math.PI / 2;
    listener.position.set(0, -0.79, 1.4);

    scene.add(intexL.group, intexR.group, subL.group, subR.group, boat.group, laptop.group);
    scene.add(waveIntex.mesh, waveBoat.mesh, waveLaptop.mesh, listener);

    let yaw = 0.0;
    let pitch = 0.56;
    const dist = 5.4;
    let dragging = false;
    let lastX = 0;
    let lastY = 0;
    let visible = true;

    const onDown = (e: PointerEvent) => {
      dragging = true;
      lastX = e.clientX;
      lastY = e.clientY;
    };
    const onMove = (e: PointerEvent) => {
      if (!dragging) return;
      yaw -= (e.clientX - lastX) * 0.007;
      pitch = Math.max(0.22, Math.min(1.25, pitch + (e.clientY - lastY) * 0.006));
      lastX = e.clientX;
      lastY = e.clientY;
    };
    const onUp = () => {
      dragging = false;
    };
    const onDblClick = () => {
      yaw = 0.0;
      pitch = 0.56;
    };
    const dom = renderer.domElement;
    dom.addEventListener("pointerdown", onDown);
    dom.addEventListener("dblclick", onDblClick);
    window.addEventListener("pointermove", onMove, { passive: true });
    window.addEventListener("pointerup", onUp, { passive: true });

    const ro = new ResizeObserver(() => {
      if (!container) return;
      const w = container.clientWidth;
      const h = Math.max(1, container.clientHeight);
      camera.aspect = w / h;
      camera.updateProjectionMatrix();
      renderer.setSize(w, h);
    });
    ro.observe(container);

    const io = new IntersectionObserver((entries) => {
      visible = entries.some((e) => e.isIntersecting);
    });
    io.observe(container);

    let raf = 0;
    let lastDraw = 0;
    let elapsed = 0;
    const tmpVec = new THREE.Vector3();

    const animate = (now: number) => {
      if (pausedRef.current) {
        raf = 0;
        lastDraw = 0;
        return;
      }
      raf = requestAnimationFrame(animate);
      if (!visible || document.hidden) return;

      const m = telemetryStore.getSnapshot().meters;
      const mx = mixerRef.current;
      const solo = mx.solo;
      const ampI = mx.master_mute || mx.intex_mute || (solo !== null && solo !== "intex") ? 0 : dbToAmp(m.intex_db);
      const ampB = mx.master_mute || mx.boat_mute || (solo !== null && solo !== "boat") ? 0 : dbToAmp(m.boat_db);
      const ampL = mx.master_mute || mx.laptop_mute || (solo !== null && solo !== "laptop") ? 0 : dbToAmp(m.laptop_db);

      const preset = POSITION_PRESETS.find((p) => p.id === mx.position_preset) ?? POSITION_PRESETS[0];
      const L = preset.layout;
      const moving = intexL.group.position.distanceToSquared(tmpVec.set(...L.intexL)) > 0.001;

      // Adaptive FPS: drop to 8 fps when completely silent & settled, ramp to targetFps when active or dragging
      const activeFps = dragging || moving || ampI + ampB + ampL > 0.02 ? fpsRef.current : 8;
      const interval = 1000 / Math.max(6, activeFps);
      if (!dragging && now - lastDraw < interval) return;

      const dt = Math.min(0.1, lastDraw ? (now - lastDraw) / 1000 : 0.033);
      lastDraw = now;
      elapsed += dt;

      const lerpAlpha = 1 - Math.exp(-dt * 7.0);
      const moveObj = (obj: THREE.Object3D, target: [number, number, number], lookAtListener = true) => {
        tmpVec.set(target[0], target[1], target[2]);
        obj.position.lerp(tmpVec, lerpAlpha);
        if (lookAtListener) obj.lookAt(0, obj.position.y, 1.35);
      };

      moveObj(intexL.group, L.intexL);
      moveObj(intexR.group, L.intexR);
      moveObj(subL.group, L.subL);
      moveObj(subR.group, L.subR);
      moveObj(boat.group, L.boat, false);
      moveObj(laptop.group, L.laptop, false);

      const pulseI = Math.sin(elapsed * 26) * 0.03 * ampI;
      intexL.cone.position.z = 0.23 + pulseI;
      intexR.cone.position.z = 0.23 + pulseI;
      subL.cone.position.z = 0.35 + pulseI * 1.5;
      subR.cone.position.z = 0.35 + pulseI * 1.5;

      intexL.ringMat.emissiveIntensity = 0.15 + ampI * 0.9;
      intexR.ringMat.emissiveIntensity = 0.15 + ampI * 0.9;
      subL.ringMat.emissiveIntensity = 0.15 + ampI * 1.0;
      subR.ringMat.emissiveIntensity = 0.15 + ampI * 1.0;

      const pulseB = Math.sin(elapsed * 30) * 0.022 * ampB;
      boat.capL.position.x = -0.46 - pulseB;
      boat.capR.position.x = 0.46 + pulseB;
      boat.ringMat.emissiveIntensity = 0.15 + ampB * 0.9;
      ledGreen.emissiveIntensity = 0.4 + ampB * 1.2;

      laptop.barMat.emissiveIntensity = 0.15 + ampL * 0.85;

      const updateWave = (
        w: { mesh: THREE.Mesh; mat: THREE.MeshBasicMaterial },
        pos: [number, number, number],
        amp: number,
        speed: number
      ) => {
        w.mesh.position.x = pos[0];
        w.mesh.position.z = pos[2];
        const phase = (elapsed * speed) % 1;
        const scale = 0.7 + phase * (1.7 + amp * 1.9);
        w.mesh.scale.set(scale, scale, 1);
        w.mat.opacity = (0.04 + amp * 0.26) * (1 - phase);
      };
      updateWave(waveIntex, [0, -0.8, (L.subL[2] + L.subR[2]) * 0.5], ampI, 0.85);
      updateWave(waveBoat, L.boat, ampB, 1.1);
      updateWave(waveLaptop, L.laptop, ampL, 1.3);

      if (!dragging) {
        yaw = Math.sin(elapsed * 0.2) * 0.2;
      }
      camera.position.set(
        Math.sin(yaw) * Math.cos(pitch) * dist,
        Math.sin(pitch) * dist + 0.25,
        Math.cos(yaw) * Math.cos(pitch) * dist - 0.15
      );
      camera.lookAt(0, -0.1, -0.35);

      renderer.render(scene, camera);
    };
    raf = pausedRef.current ? 0 : requestAnimationFrame(animate);
    wakeRef.current = () => {
      if (!pausedRef.current && !raf) {
        lastDraw = 0;
        raf = requestAnimationFrame(animate);
      }
    };

    return () => {
      cancelAnimationFrame(raf);
      wakeRef.current = null;
      ro.disconnect();
      io.disconnect();
      dom.removeEventListener("pointerdown", onDown);
      dom.removeEventListener("dblclick", onDblClick);
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
      geometries.forEach((g) => g.dispose());
      materials.forEach((m) => m.dispose());
      renderer.dispose();
      if (dom.parentNode === container) container.removeChild(dom);
    };
  }, []);

  return (
    <div className="relative h-[280px] sm:h-[320px] w-full overflow-hidden rounded-xl border border-white/[0.06] bg-[#141519]">
      <div ref={mountRef} className="h-full w-full cursor-grab active:cursor-grabbing" />
      <div className="pointer-events-none absolute top-3 left-3 flex items-center gap-2">
        <span className="rounded-md border border-white/[0.08] bg-black/50 px-2.5 py-1 text-[11px] font-medium text-zinc-300">
          3D Room Stage • Drag to orbit · Double-click reset
        </span>
      </div>
    </div>
  );
});

export default SpeakerStage3D;
