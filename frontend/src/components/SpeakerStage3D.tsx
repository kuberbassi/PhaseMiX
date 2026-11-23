import { memo, useEffect, useRef, useState } from "react";
import * as THREE from "three";
import { dbToVisualNorm } from "@/lib/audioVisuals";
import { telemetryStore } from "@/lib/telemetryStore";
import { POSITION_PRESETS, type MixerState, type Role } from "@/types/mixer";

interface SpeakerStage3DProps {
  mixer: MixerState;
  targetFps?: number;
  paused?: boolean;
  onSoloRole?: (role: Role) => void;
}

interface HoverHud {
  role: Role;
  title: string;
  detail: string;
}

function tagRole(obj: THREE.Object3D, role: Role) {
  obj.userData.role = role;
  obj.traverse((child) => {
    child.userData.role = role;
  });
}

function drawLaptopScreen(
  ctx: CanvasRenderingContext2D,
  w: number,
  h: number,
  muted: boolean,
  ampIn: number,
  ampI: number,
  ampB: number,
  ampL: number
) {
  ctx.fillStyle = "#0e1015";
  ctx.fillRect(0, 0, w, h);

  ctx.strokeStyle = muted ? "rgba(239,68,68,0.45)" : "rgba(34,197,94,0.4)";
  ctx.lineWidth = 2;
  ctx.strokeRect(4, 4, w - 8, h - 8);

  ctx.fillStyle = "#d4d4d8";
  ctx.font = "bold 12px sans-serif";
  ctx.fillText("PHASEMIX", 14, 24);

  ctx.fillStyle = muted ? "#ef4444" : "#22c55e";
  ctx.beginPath();
  ctx.arc(w - 20, 20, 4, 0, Math.PI * 2);
  ctx.fill();

  const bars = [
    { label: "IN", val: ampIn, col: "#e4e4e7" },
    { label: "AUX", val: ampI, col: "#22c55e" },
    { label: "BT", val: ampB, col: "#60a5fa" },
    { label: "PC", val: ampL, col: "#a1a1aa" },
  ];
  const baseBottom = h - 22;
  const maxBarH = 56;
  const barW = 34;
  const startX = 24;
  const gap = 22;

  bars.forEach((b, i) => {
    const x = startX + i * (barW + gap);
    ctx.fillStyle = "#1c1f27";
    ctx.fillRect(x, baseBottom - maxBarH, barW, maxBarH);
    const bh = Math.max(3, Math.round(b.val * maxBarH));
    ctx.fillStyle = muted ? "#52525b" : b.col;
    ctx.fillRect(x, baseBottom - bh, barW, bh);
    ctx.fillStyle = "#71717a";
    ctx.font = "bold 9px monospace";
    ctx.fillText(b.label, x + 7, h - 9);
  });
}

export const SpeakerStage3D = memo(function SpeakerStage3D({
  mixer,
  targetFps = 60,
  paused = false,
  onSoloRole,
}: SpeakerStage3DProps) {
  const mountRef = useRef<HTMLDivElement | null>(null);
  const mixerRef = useRef(mixer);
  const fpsRef = useRef(targetFps);
  const pausedRef = useRef(paused);
  const soloCbRef = useRef(onSoloRole);
  const wakeRef = useRef<(() => void) | null>(null);
  mixerRef.current = mixer;
  fpsRef.current = targetFps;
  pausedRef.current = paused;
  soloCbRef.current = onSoloRole;

  const [hoverHud, setHoverHud] = useState<HoverHud | null>(null);
  const [cursorStyle, setCursorStyle] = useState<"grab" | "pointer">("grab");

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

    const renderer = new THREE.WebGLRenderer({
      antialias: true,
      alpha: false,
      powerPreference: "high-performance",
      stencil: false,
    });
    renderer.setClearColor(0x141519, 1);
    renderer.setPixelRatio(Math.min(window.devicePixelRatio || 1, 1.5));
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

    const matGeo = trackGeo(new THREE.CircleGeometry(2.65, 40));
    const matMaterial = trackMat(
      new THREE.MeshBasicMaterial({ color: 0x1b1d24, transparent: true, opacity: 0.65 })
    );
    const deskMat = new THREE.Mesh(matGeo, matMaterial);
    deskMat.rotation.x = -Math.PI / 2;
    deskMat.position.y = -0.818;
    scene.add(deskMat);

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

    const satBoxGeo = trackGeo(new THREE.BoxGeometry(0.42, 0.76, 0.44));
    const satBezelGeo = trackGeo(new THREE.BoxGeometry(0.37, 0.69, 0.03));
    const satConeGeo = trackGeo(new THREE.CylinderGeometry(0.11, 0.06, 0.06, 16));
    const satTweetGeo = trackGeo(new THREE.SphereGeometry(0.052, 12, 10));
    const satRingGeo = trackGeo(new THREE.TorusGeometry(0.13, 0.012, 8, 20));

    const subBoxGeo = trackGeo(new THREE.BoxGeometry(0.68, 0.72, 0.72));
    const subConeGeo = trackGeo(new THREE.CylinderGeometry(0.22, 0.11, 0.08, 18));
    const subPortGeo = trackGeo(new THREE.CylinderGeometry(0.058, 0.058, 0.05, 12));
    const subRingGeo = trackGeo(new THREE.TorusGeometry(0.235, 0.014, 8, 22));

    const waveGeo = trackGeo(new THREE.RingGeometry(0.28, 0.308, 32));
    const listenerGeo = trackGeo(new THREE.TorusGeometry(0.32, 0.012, 8, 32));

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
      tagRole(group, "intex");
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
      tagRole(group, "intex");
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
      tagRole(group, "boat");
      return { group, capL, capR, ringMat };
    }

    const screenCanvas = document.createElement("canvas");
    screenCanvas.width = 256;
    screenCanvas.height = 128;
    const screenCtx = screenCanvas.getContext("2d");
    const screenTex = new THREE.CanvasTexture(screenCanvas);
    if (screenCtx) {
      drawLaptopScreen(screenCtx, 256, 128, false, 0, 0, 0, 0);
      screenTex.needsUpdate = true;
    }

    function buildLaptop() {
      const group = new THREE.Group();
      const baseGeo = trackGeo(new THREE.BoxGeometry(1.05, 0.05, 0.72));
      const lidGeo = trackGeo(new THREE.BoxGeometry(1.05, 0.64, 0.03));
      const screenGeo = trackGeo(new THREE.PlaneGeometry(0.95, 0.56));
      const barGeo = trackGeo(new THREE.BoxGeometry(0.055, 0.016, 0.44));
      const screenMat = trackMat(new THREE.MeshBasicMaterial({ map: screenTex }));

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
      tagRole(group, "laptop");
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
      new THREE.MeshBasicMaterial({ color: 0x22c55e, transparent: true, opacity: 0.55 })
    );
    const listener = new THREE.Mesh(listenerGeo, listenerMat);
    listener.rotation.x = Math.PI / 2;
    listener.position.set(0, -0.79, 1.4);

    const pickTargets = [intexL.group, intexR.group, subL.group, subR.group, boat.group, laptop.group];
    scene.add(...pickTargets);
    scene.add(waveIntex.mesh, waveBoat.mesh, waveLaptop.mesh, listener);

    let yaw = 0.0;
    let pitch = 0.56;
    let dist = 5.4;
    let targetDist = 5.4;
    let dragging = false;
    let downX = 0;
    let downY = 0;
    let lastX = 0;
    let lastY = 0;
    let visible = true;
    let activeHoverRole: Role | null = null;

    const raycaster = new THREE.Raycaster();
    const ndc = new THREE.Vector2();

    const findRoleAtPointer = (e: PointerEvent): Role | null => {
      const rect = renderer.domElement.getBoundingClientRect();
      if (rect.width <= 0 || rect.height <= 0) return null;
      ndc.x = ((e.clientX - rect.left) / rect.width) * 2 - 1;
      ndc.y = -((e.clientY - rect.top) / rect.height) * 2 + 1;
      raycaster.setFromCamera(ndc, camera);
      const hits = raycaster.intersectObjects(pickTargets, true);
      const r = hits[0]?.object?.userData?.role;
      return r === "intex" || r === "boat" || r === "laptop" ? r : null;
    };

    const syncHoverHud = (r: Role | null) => {
      if (r === activeHoverRole) return;
      activeHoverRole = r;
      setCursorStyle(r ? "pointer" : "grab");
      if (!r) {
        setHoverHud(null);
        return;
      }
      const mx = mixerRef.current;
      const isSolo = mx.solo === r;
      const meta: Record<Role, { title: string; delay: number }> = {
        intex: { title: "Primary 2.1 / AUX", delay: mx.intex_delay_ms },
        boat: { title: "Center / Wireless", delay: mx.boat_delay_ms },
        laptop: { title: "Built-in Speakers", delay: mx.laptop_delay_ms },
      };
      setHoverHud({
        role: r,
        title: meta[r].title,
        detail: `${meta[r].delay} ms · ${isSolo ? "Solo active (click to clear)" : "Click to Solo"}`,
      });
    };

    const onDown = (e: PointerEvent) => {
      dragging = true;
      downX = e.clientX;
      downY = e.clientY;
      lastX = e.clientX;
      lastY = e.clientY;
    };
    const onMove = (e: PointerEvent) => {
      if (dragging) {
        yaw -= (e.clientX - lastX) * 0.007;
        pitch = Math.max(0.22, Math.min(1.25, pitch + (e.clientY - lastY) * 0.006));
        lastX = e.clientX;
        lastY = e.clientY;
        return;
      }
      syncHoverHud(findRoleAtPointer(e));
    };
    const onUp = (e: PointerEvent) => {
      if (!dragging) return;
      dragging = false;
      const moved = Math.hypot(e.clientX - downX, e.clientY - downY);
      if (moved < 5) {
        const role = findRoleAtPointer(e);
        if (role) {
          soloCbRef.current?.(role);
        }
      }
    };
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      targetDist = Math.max(3.6, Math.min(7.4, targetDist + e.deltaY * 0.003));
    };
    const onDblClick = () => {
      yaw = 0.0;
      pitch = 0.56;
      targetDist = 5.4;
    };
    const onLeave = () => syncHoverHud(null);

    const dom = renderer.domElement;
    dom.addEventListener("pointerdown", onDown);
    dom.addEventListener("pointerleave", onLeave);
    dom.addEventListener("wheel", onWheel, { passive: false });
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
    let lastScreenDraw = 0;
    let elapsed = 0;
    const tmpVec = new THREE.Vector3();

    const colWhite = new THREE.Color(0xe4e4e7);
    const colGreen = new THREE.Color(0x22c55e);
    const colAmber = new THREE.Color(0xf59e0b);
    const colDim = new THREE.Color(0x3f3f46);

    const animate = (now: number) => {
      if (pausedRef.current) {
        raf = 0;
        lastDraw = 0;
        return;
      }
      raf = requestAnimationFrame(animate);
      if (!visible || document.hidden) return;

      const snap = telemetryStore.getSnapshot();
      const m = snap.meters;
      const mx = mixerRef.current;
      const mVol = snap.master_vol ?? mx.master_vol;
      const mMute = snap.master_mute ?? mx.master_mute;
      const solo = mx.solo;
      const ampIn = dbToVisualNorm(m.in_db, mVol, false, false);
      const ampI = dbToVisualNorm(
        m.intex_db,
        mVol,
        true,
        mMute || mx.intex_mute || (solo !== null && solo !== "intex")
      );
      const ampB = dbToVisualNorm(
        m.boat_db,
        mVol,
        true,
        mMute || mx.boat_mute || (solo !== null && solo !== "boat")
      );
      const ampL = dbToVisualNorm(
        m.laptop_db,
        mVol,
        true,
        mMute || mx.laptop_mute || (solo !== null && solo !== "laptop")
      );

      const preset = POSITION_PRESETS.find((p) => p.id === mx.position_preset) ?? POSITION_PRESETS[0];
      const L = preset.layout;
      const widthScale = 0.72 + ((mx.intex_width - 50) / 100) * 0.56;
      const moving = intexL.group.position.distanceToSquared(tmpVec.set(L.intexL[0] * widthScale, L.intexL[1], L.intexL[2])) > 0.001;

      const audioActive = ampIn > 0.01 || ampI + ampB + ampL > 0.01;
      const activeFps = dragging || moving || Math.abs(dist - targetDist) > 0.01 || audioActive
        ? fpsRef.current
        : Math.min(fpsRef.current, 30);
      if (activeFps < 55 && !dragging) {
        const interval = 1000 / Math.max(12, activeFps) - 1.5;
        if (lastDraw && now - lastDraw < interval) return;
      }

      const dt = Math.min(0.1, lastDraw ? (now - lastDraw) / 1000 : 0.0167);
      lastDraw = now;
      elapsed += dt;

      const lerpAlpha = 1 - Math.exp(-dt * 8.5);
      dist += (targetDist - dist) * lerpAlpha;

      const moveObj = (obj: THREE.Object3D, x: number, y: number, z: number, lookAtListener = true) => {
        tmpVec.set(x, y, z);
        obj.position.lerp(tmpVec, lerpAlpha);
        if (lookAtListener) obj.lookAt(0, obj.position.y, 1.35);
      };

      moveObj(intexL.group, L.intexL[0] * widthScale, L.intexL[1], L.intexL[2]);
      moveObj(intexR.group, L.intexR[0] * widthScale, L.intexR[1], L.intexR[2]);
      moveObj(subL.group, L.subL[0] * (0.85 + 0.15 * widthScale), L.subL[1], L.subL[2]);
      moveObj(subR.group, L.subR[0] * (0.85 + 0.15 * widthScale), L.subR[1], L.subR[2]);
      moveObj(boat.group, L.boat[0], L.boat[1], L.boat[2], false);
      moveObj(laptop.group, L.laptop[0], L.laptop[1], L.laptop[2], false);

      const bal = Math.max(-100, Math.min(100, mx.intex_balance)) / 100;
      const balL = bal > 0 ? 1 - bal * 0.65 : 1;
      const balR = bal < 0 ? 1 + bal * 0.65 : 1;

      const pulseI = Math.sin(elapsed * 26) * 0.036 * ampI;
      intexL.cone.position.z = 0.23 + pulseI * balL;
      intexR.cone.position.z = 0.23 + pulseI * balR;
      subL.cone.position.z = 0.35 + pulseI * 1.55 * balL;
      subR.cone.position.z = 0.35 + pulseI * 1.55 * balR;

      const intexHighlight = solo === "intex" || activeHoverRole === "intex";
      const intexTargetCol = solo !== null && solo !== "intex" ? colDim : intexHighlight ? colGreen : colWhite;
      intexL.ringMat.emissive.copy(intexTargetCol);
      intexR.ringMat.emissive.copy(intexTargetCol);
      intexL.ringMat.emissiveIntensity = (intexHighlight ? 0.35 : 0.15) + ampI * balL * 0.95;
      intexR.ringMat.emissiveIntensity = (intexHighlight ? 0.35 : 0.15) + ampI * balR * 0.95;
      subL.ringMat.emissive.copy(intexTargetCol);
      subR.ringMat.emissive.copy(intexTargetCol);
      subL.ringMat.emissiveIntensity = 0.15 + ampI * balL * 1.05;
      subR.ringMat.emissiveIntensity = 0.15 + ampI * balR * 1.05;

      const boatSign = mx.boat_invert ? -1 : 1;
      const pulseB = Math.sin(elapsed * 30) * 0.028 * ampB * boatSign;
      boat.capL.position.x = -0.46 - pulseB;
      boat.capR.position.x = 0.46 + pulseB;
      const boatCol = mx.boat_invert
        ? colAmber
        : solo !== null && solo !== "boat"
          ? colDim
          : solo === "boat" || activeHoverRole === "boat"
            ? colGreen
            : colWhite;
      boat.ringMat.emissive.copy(boatCol);
      boat.ringMat.emissiveIntensity = (solo === "boat" || activeHoverRole === "boat" ? 0.38 : 0.15) + ampB * 0.95;
      ledGreen.emissive.copy(mx.boat_invert ? colAmber : colGreen);
      ledGreen.emissiveIntensity = 0.4 + ampB * 1.25;

      const laptopCol = mx.laptop_invert
        ? colAmber
        : solo !== null && solo !== "laptop"
          ? colDim
          : solo === "laptop" || activeHoverRole === "laptop"
            ? colGreen
            : colWhite;
      laptop.barMat.emissive.copy(laptopCol);
      laptop.barMat.emissiveIntensity = (solo === "laptop" || activeHoverRole === "laptop" ? 0.38 : 0.15) + ampL * 0.9;

      const delayDiff = Math.abs(mx.intex_delay_ms - mx.laptop_delay_ms);
      const syncGood = delayDiff <= 8 && mx.intex_delay_ms > 0;
      listenerMat.color.copy(mx.master_mute ? colDim : syncGood ? colGreen : colAmber);
      listenerMat.opacity = mx.master_mute ? 0.25 : 0.4 + (ampI + ampB + ampL) * 0.14;

      const updateWave = (
        w: { mesh: THREE.Mesh; mat: THREE.MeshBasicMaterial },
        x: number,
        z: number,
        amp: number,
        speed: number,
        col: THREE.Color
      ) => {
        w.mesh.position.x = x;
        w.mesh.position.z = z;
        const phase = (elapsed * speed) % 1;
        const scale = 0.7 + phase * (1.7 + amp * 2.0);
        w.mesh.scale.set(scale, scale, 1);
        w.mat.color.copy(col);
        w.mat.opacity = (0.04 + amp * 0.3) * (1 - phase);
      };
      updateWave(waveIntex, intexL.group.position.x * 0 + bal * 0.45, (L.subL[2] + L.subR[2]) * 0.5, ampI, 0.85, syncGood ? colGreen : colWhite);
      updateWave(waveBoat, L.boat[0], L.boat[2], ampB, 1.1, mx.boat_invert ? colAmber : colWhite);
      updateWave(waveLaptop, L.laptop[0], L.laptop[2], ampL, 1.3, mx.laptop_invert ? colAmber : colWhite);

      if (screenCtx && now - lastScreenDraw >= 28) {
        lastScreenDraw = now;
        drawLaptopScreen(screenCtx, 256, 128, mMute || mx.master_mute, ampIn, ampI, ampB, ampL);
        screenTex.needsUpdate = true;
      }

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
      dom.removeEventListener("pointerleave", onLeave);
      dom.removeEventListener("wheel", onWheel);
      dom.removeEventListener("dblclick", onDblClick);
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
      screenTex.dispose();
      geometries.forEach((g) => g.dispose());
      materials.forEach((m) => m.dispose());
      renderer.dispose();
      if (dom.parentNode === container) container.removeChild(dom);
    };
  }, []);

  const delayDiff = Math.abs(mixer.intex_delay_ms - mixer.laptop_delay_ms);
  const synced = delayDiff <= 8 && mixer.intex_delay_ms > 0;

  return (
    <div className="relative h-[280px] sm:h-[320px] w-full overflow-hidden rounded-xl border border-white/[0.06] bg-[#141519]">
      <div
        ref={mountRef}
        className={`h-full w-full ${cursorStyle === "pointer" ? "cursor-pointer" : "cursor-grab active:cursor-grabbing"}`}
      />
      <div className="pointer-events-none absolute top-3 left-3 right-3 flex items-center justify-between gap-2">
        <span className="rounded-md border border-white/[0.08] bg-black/55 px-2.5 py-1 text-[11px] font-medium text-zinc-300">
          3D Room · Click speaker to Solo · Scroll zoom
        </span>
        <span
          className={`rounded-md border px-2 py-0.5 font-mono text-[10px] font-semibold ${
            synced
              ? "border-emerald-500/30 bg-emerald-500/15 text-emerald-300"
              : "border-amber-500/30 bg-amber-500/15 text-amber-300"
          }`}
        >
          {synced ? `SYNC ${mixer.intex_delay_ms} ms` : `OFFSET ${delayDiff} ms`}
        </span>
      </div>
      {hoverHud && (
        <div className="pointer-events-none absolute bottom-3 left-3 rounded-lg border border-white/15 bg-black/75 px-3 py-1.5 text-[11px] text-zinc-100 backdrop-blur-sm">
          <div className="font-semibold">{hoverHud.title}</div>
          <div className="text-[10px] text-zinc-400">{hoverHud.detail}</div>
        </div>
      )}
    </div>
  );
});

export default SpeakerStage3D;
