// The memory sky — a companion's mind as a place you look into (s537).
//
// Owns the whole WebGL life: renderer, camera, controls, the d3-force-3d
// layout, and three draw calls — orbs (one Points), bolts (one instanced
// ribbon mesh), dust (one Points). React mounts a canvas and talks to this
// class; nothing in here knows about React.
//
// Why not one object per node like the studio's graph: a 2,400-memory mind
// with 14,000 edges must stay a handful of draw calls or WebKitGTK chokes
// (measured there: 29 node objects → 58ms/frame on software GL). Here the GPU
// does the crackle; the CPU touches endpoints only while the layout moves.

import {
  BufferAttribute,
  BufferGeometry,
  Color,
  InstancedBufferAttribute,
  InstancedBufferGeometry,
  Mesh,
  PerspectiveCamera,
  Points,
  Scene,
  ShaderMaterial,
  Vector3,
  WebGLRenderer,
  AdditiveBlending,
  Float32BufferAttribute,
} from "three";
import { OrbitControls } from "three/examples/jsm/controls/OrbitControls.js";
import {
  forceCenter,
  forceLink,
  forceManyBody,
  forceSimulation,
  forceX,
  forceY,
  forceZ,
  type Simulation,
} from "d3-force-3d";

import type { MemoryGraph, MemoryGraphNode } from "../../memory/organService";
import { typeColor, VOID_COLOR } from "../palette";
import {
  BOLT_FRAGMENT,
  BOLT_SEGMENTS,
  BOLT_VERTEX,
  DUST_FRAGMENT,
  DUST_VERTEX,
  ORB_FRAGMENT,
  ORB_VERTEX,
} from "./shaders";

export interface SkyNode {
  index: number;
  name: string;
  description: string;
  memType: string;
  importance: number;
  accessCount: number;
  archived: boolean;
  links: string[];
  hint: [number, number, number] | null;
  // d3-force-3d state
  x: number;
  y: number;
  z: number;
  vx?: number;
  vy?: number;
  vz?: number;
}

export interface SkyEdge {
  index: number;
  source: SkyNode;
  target: SkyNode;
  kind: "link" | "semantic";
  weight: number;
}

export interface SkyHit {
  name: string;
  score: number;
}

export interface SkyStats {
  nodes: number;
  edges: number;
  fps: number;
  renderScale: number;
  settled: boolean;
}

export interface SkyCallbacks {
  onHover?: (node: SkyNode | null, x: number, y: number) => void;
  onSelect?: (node: SkyNode | null) => void;
  onStats?: (stats: SkyStats) => void;
}

/** Room kept at the end of the buffers for the spell: one orb for the query,
 *  one bolt per hit. */
const MAX_HITS = 24;
const NEVER = -1e9;
const DUST_COUNT = 1800;
const PICK_RADIUS_PX = 18;
const IDLE_RESUME_MS = 7000;
const FIRE_EVERY_MS: [number, number] = [2200, 4600];

const RENDER_SCALES = [1, 0.8, 0.6];
/** A resting semantic bolt's half-width on screen; links and lit bolts widen from here. */
const BOLT_HALF_WIDTH_PX = 2.4;
/** An orb flown into fills a hand, not the window. */
const ORB_MAX_PX = 150;
/** Edge count at which additive stacking starts to wash out; intensity sinks past it. */
const DENSE_EDGES = 1500;

interface Tween {
  start: number;
  duration: number;
  update: (t: number) => void;
  done?: () => void;
}

function easeInOut(t: number): number {
  return t < 0.5 ? 4 * t * t * t : 1 - Math.pow(-2 * t + 2, 3) / 2;
}

export class MemorySky {
  private readonly canvas: HTMLCanvasElement;
  private readonly callbacks: SkyCallbacks;
  private readonly renderer: WebGLRenderer;
  private readonly scene = new Scene();
  private readonly camera: PerspectiveCamera;
  private readonly controls: OrbitControls;

  private nodes: SkyNode[] = [];
  private edges: SkyEdge[] = [];
  private byName = new Map<string, SkyNode>();
  private adjacency: number[][] = []; // node index → edge indices
  private simulation: Simulation<SkyNode> | null = null;
  private settled = false;
  private radius = 200;

  // orbs
  private orbGeometry: BufferGeometry | null = null;
  private orbMaterial: ShaderMaterial;
  private orbs: Points | null = null;
  private orbPosition!: Float32BufferAttribute;
  private orbLit!: Float32BufferAttribute;
  private orbPulse!: Float32BufferAttribute;

  // bolts
  private boltGeometry: InstancedBufferGeometry | null = null;
  private boltMaterial: ShaderMaterial;
  private bolts: Mesh | null = null;
  private boltSrc!: InstancedBufferAttribute;
  private boltDst!: InstancedBufferAttribute;
  private boltLit!: InstancedBufferAttribute;
  private boltPulse!: InstancedBufferAttribute;
  private boltWeight!: InstancedBufferAttribute;

  // dust
  private dustMaterial: ShaderMaterial;
  private dust: Points | null = null;

  // the spell
  private hits: SkyHit[] = [];
  private hitNodes: SkyNode[] = [];
  private focus = 0;
  private focusTarget = 0;
  private selected: SkyNode | null = null;

  private tweens: Tween[] = [];
  private frame = 0;
  private startedAt = performance.now();
  private lastFrameAt = performance.now();
  private fps = 60;
  private slowFrames = 0;
  private calmFrames = 0;
  private renderScaleIndex = 0;
  private pointer: { x: number; y: number } | null = null;
  private pointerDirty = false;
  private hovered: SkyNode | null = null;
  private lastInteractionAt = performance.now();
  private nextFireAt = performance.now() + 2500;
  private disposed = false;
  private readonly resizeObserver: ResizeObserver;

  constructor(canvas: HTMLCanvasElement, callbacks: SkyCallbacks = {}) {
    this.canvas = canvas;
    this.callbacks = callbacks;

    this.renderer = new WebGLRenderer({
      canvas,
      antialias: false,
      alpha: false,
      powerPreference: "high-performance",
    });
    this.renderer.setClearColor(new Color(VOID_COLOR), 1);
    this.renderer.autoClear = true;

    this.camera = new PerspectiveCamera(50, 1, 1, 8000);
    this.camera.position.set(0, 60, 520);

    this.controls = new OrbitControls(this.camera, canvas);
    this.controls.enableDamping = true;
    this.controls.dampingFactor = 0.06;
    this.controls.rotateSpeed = 0.55;
    this.controls.zoomSpeed = 0.8;
    this.controls.minDistance = 12;
    this.controls.maxDistance = 5000;
    this.controls.autoRotate = true;
    this.controls.autoRotateSpeed = 0.3;
    this.controls.addEventListener("start", () => {
      this.controls.autoRotate = false;
      this.lastInteractionAt = performance.now();
    });
    this.controls.addEventListener("end", () => {
      this.lastInteractionAt = performance.now();
    });

    const gl = this.renderer.getContext();
    const pointRange = gl.getParameter(gl.ALIASED_POINT_SIZE_RANGE) as Float32Array | number[];
    const maxPoint = Math.min(Math.max(32, Number(pointRange?.[1] ?? 64)), ORB_MAX_PX);

    this.orbMaterial = new ShaderMaterial({
      vertexShader: ORB_VERTEX,
      fragmentShader: ORB_FRAGMENT,
      uniforms: {
        uTime: { value: 0 },
        uProj: { value: 500 },
        uMaxPoint: { value: maxPoint },
        uFocus: { value: 0 },
      },
      blending: AdditiveBlending,
      transparent: true,
      depthWrite: false,
      depthTest: false,
    });
    this.boltMaterial = new ShaderMaterial({
      vertexShader: BOLT_VERTEX,
      fragmentShader: BOLT_FRAGMENT,
      uniforms: {
        uTime: { value: 0 },
        uWidth: { value: BOLT_HALF_WIDTH_PX },
        uProj: { value: 500 },
        uFocus: { value: 0 },
        uFlicker: { value: 11 },
        uDensity: { value: 1 },
      },
      blending: AdditiveBlending,
      transparent: true,
      depthWrite: false,
      depthTest: false,
    });
    this.dustMaterial = new ShaderMaterial({
      vertexShader: DUST_VERTEX,
      fragmentShader: DUST_FRAGMENT,
      uniforms: { uTime: { value: 0 }, uProj: { value: 500 } },
      blending: AdditiveBlending,
      transparent: true,
      depthWrite: false,
      depthTest: false,
    });

    this.buildDust();

    canvas.addEventListener("pointermove", this.onPointerMove);
    canvas.addEventListener("pointerleave", this.onPointerLeave);
    canvas.addEventListener("click", this.onClick);

    this.resizeObserver = new ResizeObserver(() => this.resize());
    this.resizeObserver.observe(canvas.parentElement ?? canvas);
    this.resize();
    this.frame = requestAnimationFrame(this.loop);
  }

  // ── public ──────────────────────────────────────────────────────────────

  setGraph(graph: MemoryGraph): void {
    this.clearScene();
    const nodes: SkyNode[] = graph.nodes.map((n, index) => this.toSkyNode(n, index));
    const byName = new Map(nodes.map((n) => [n.name, n]));
    const edges: SkyEdge[] = [];
    for (const e of graph.edges) {
      const source = byName.get(e.source);
      const target = byName.get(e.target);
      if (!source || !target || source === target) continue;
      edges.push({ index: edges.length, source, target, kind: e.kind, weight: e.weight });
    }
    this.nodes = nodes;
    this.edges = edges;
    this.byName = byName;
    this.adjacency = nodes.map(() => []);
    for (const e of edges) {
      this.adjacency[e.source.index].push(e.index);
      this.adjacency[e.target.index].push(e.index);
    }

    // The sky's scale grows with the mind: ~40 units per cube-root memory.
    this.radius = Math.max(120, 40 * Math.cbrt(Math.max(nodes.length, 1)));
    this.seedPositions();
    this.buildOrbs();
    this.buildBolts();
    this.buildSimulation();
    this.settled = false;
    // A new mind gets a fresh reading at full scale — the last mind's verdict
    // says nothing about this one.
    this.slowFrames = 0;
    this.calmFrames = 0;
    if (this.renderScaleIndex !== 0) {
      this.renderScaleIndex = 0;
      this.resize();
    }
    // 14k bolts stacked additively wash to white at the intensity that makes
    // 300 read as lightning. Sink the resting intensity with density; pulses
    // and the spell keep most of their light so they still stand out.
    this.boltMaterial.uniforms.uDensity.value = Math.min(
      1,
      Math.pow(DENSE_EDGES / Math.max(edges.length, 1), 0.6),
    );

    const distance = this.radius * 2.6;
    this.camera.position.set(0, this.radius * 0.35, distance);
    this.controls.target.set(0, 0, 0);
    this.controls.autoRotate = true;
    this.controls.update();
    this.emitStats();
  }

  /** The spell: light the hits, sink everything else, draw the query as an
   *  orb tethered to what it found, and fly there. Pass an empty list to
   *  clear. */
  setHits(hits: SkyHit[]): void {
    if (!this.orbGeometry) return;
    this.hits = hits.slice(0, MAX_HITS);
    this.hitNodes = this.hits
      .map((h) => this.byName.get(h.name))
      .filter((n): n is SkyNode => Boolean(n));
    this.selected = null;
    this.applyLighting();
    if (this.hitNodes.length === 0) {
      this.focusTarget = 0;
      this.controls.autoRotate = true;
      return;
    }
    this.focusTarget = 1;
    const now = this.now();
    this.hitNodes.forEach((node, rank) => {
      this.orbPulse.setX(node.index, now + 0.12 * rank);
      for (const edgeIndex of this.adjacency[node.index]) {
        this.boltPulse.setX(edgeIndex, now + 0.12 * rank + 0.08);
      }
    });
    for (let i = 0; i < this.hitNodes.length; i += 1) {
      this.boltPulse.setX(this.edges.length + i, now + 0.12 * i);
    }
    this.orbPulse.needsUpdate = true;
    this.boltPulse.needsUpdate = true;
    this.flyToCentroid(this.hitNodes);
  }

  select(name: string | null): void {
    if (!this.orbGeometry) return;
    const node = name ? (this.byName.get(name) ?? null) : null;
    this.selected = node;
    this.applyLighting();
    if (node) {
      this.orbPulse.setX(node.index, this.now());
      this.orbPulse.needsUpdate = true;
      this.focusTarget = this.hitNodes.length ? 1 : 0.55;
      this.flyTo(node);
    } else if (this.hitNodes.length === 0) {
      this.focusTarget = 0;
    }
  }

  flyTo(node: SkyNode): void {
    this.flyToPoint(new Vector3(node.x, node.y, node.z), Math.max(150, this.radius * 0.55));
  }

  dispose(): void {
    this.disposed = true;
    cancelAnimationFrame(this.frame);
    this.resizeObserver.disconnect();
    this.canvas.removeEventListener("pointermove", this.onPointerMove);
    this.canvas.removeEventListener("pointerleave", this.onPointerLeave);
    this.canvas.removeEventListener("click", this.onClick);
    this.controls.dispose();
    this.clearScene();
    this.dust?.geometry.dispose();
    this.orbMaterial.dispose();
    this.boltMaterial.dispose();
    this.dustMaterial.dispose();
    this.renderer.dispose();
  }

  // ── graph → buffers ───────────────────────────────────────────────────

  private toSkyNode(n: MemoryGraphNode, index: number): SkyNode {
    return {
      index,
      name: n.name,
      description: n.description,
      memType: n.mem_type,
      importance: n.importance,
      accessCount: n.access_count,
      archived: n.archived_at != null,
      links: n.links,
      hint: n.pos,
      x: 0,
      y: 0,
      z: 0,
    };
  }

  private seedPositions(): void {
    const r = this.radius;
    for (const node of this.nodes) {
      if (node.hint) {
        node.x = node.hint[0] * r * 0.85 + (Math.random() - 0.5) * r * 0.2;
        node.y = node.hint[1] * r * 0.85 + (Math.random() - 0.5) * r * 0.2;
        node.z = node.hint[2] * r * 0.85 + (Math.random() - 0.5) * r * 0.2;
      } else {
        const u = Math.random() * 2 - 1;
        const phi = Math.random() * Math.PI * 2;
        const rr = r * 0.6 * Math.cbrt(Math.random());
        const s = Math.sqrt(1 - u * u);
        node.x = rr * s * Math.cos(phi);
        node.y = rr * s * Math.sin(phi);
        node.z = rr * u;
      }
    }
  }

  private buildOrbs(): void {
    const count = this.nodes.length + 1; // + the query orb
    const position = new Float32Array(count * 3);
    const color = new Float32Array(count * 3);
    const size = new Float32Array(count);
    const heat = new Float32Array(count);
    const seed = new Float32Array(count);
    const lit = new Float32Array(count);
    const pulse = new Float32Array(count).fill(NEVER);
    const ghost = new Float32Array(count);

    const maxAccess = this.nodes.reduce((m, n) => Math.max(m, n.accessCount), 1);
    for (const n of this.nodes) {
      const i = n.index;
      const c = typeColor(n.memType);
      color[i * 3] = c.r;
      color[i * 3 + 1] = c.g;
      color[i * 3 + 2] = c.b;
      size[i] = 7 + 17 * n.importance;
      heat[i] = Math.log1p(n.accessCount) / Math.log1p(maxAccess);
      seed[i] = Math.random();
      ghost[i] = n.archived ? 1 : 0;
    }
    // The query orb: white-violet, large, always hot; drawn only while a
    // spell holds (draw range excludes it otherwise).
    const q = this.nodes.length;
    color[q * 3] = 0.86;
    color[q * 3 + 1] = 0.8;
    color[q * 3 + 2] = 1.0;
    size[q] = 30;
    heat[q] = 1;
    lit[q] = 1;

    const geometry = new BufferGeometry();
    this.orbPosition = new Float32BufferAttribute(position, 3);
    this.orbLit = new Float32BufferAttribute(lit, 1);
    this.orbPulse = new Float32BufferAttribute(pulse, 1);
    geometry.setAttribute("position", this.orbPosition);
    geometry.setAttribute("aColor", new Float32BufferAttribute(color, 3));
    geometry.setAttribute("aSize", new Float32BufferAttribute(size, 1));
    geometry.setAttribute("aHeat", new Float32BufferAttribute(heat, 1));
    geometry.setAttribute("aSeed", new Float32BufferAttribute(seed, 1));
    geometry.setAttribute("aLit", this.orbLit);
    geometry.setAttribute("aPulse", this.orbPulse);
    geometry.setAttribute("aGhost", new Float32BufferAttribute(ghost, 1));
    geometry.setDrawRange(0, this.nodes.length);
    this.orbGeometry = geometry;
    this.orbs = new Points(geometry, this.orbMaterial);
    this.orbs.frustumCulled = false;
    this.scene.add(this.orbs);
    this.writeOrbPositions();
  }

  private buildBolts(): void {
    const count = this.edges.length + MAX_HITS;
    const geometry = new InstancedBufferGeometry();

    // Base ribbon: (SEGMENTS + 1) stations × 2 rails.
    const stations = BOLT_SEGMENTS + 1;
    const t = new Float32Array(stations * 2);
    const side = new Float32Array(stations * 2);
    const base = new Float32Array(stations * 2 * 3); // unused by the shader, required by three
    for (let s = 0; s < stations; s += 1) {
      t[s * 2] = s / BOLT_SEGMENTS;
      t[s * 2 + 1] = s / BOLT_SEGMENTS;
      side[s * 2] = -1;
      side[s * 2 + 1] = 1;
    }
    const indices: number[] = [];
    for (let s = 0; s < BOLT_SEGMENTS; s += 1) {
      const a = s * 2;
      const b = a + 1;
      const c = a + 2;
      const d = a + 3;
      indices.push(a, b, c, b, d, c);
    }
    geometry.setAttribute("position", new Float32BufferAttribute(base, 3));
    geometry.setAttribute("aT", new Float32BufferAttribute(t, 1));
    geometry.setAttribute("aSide", new Float32BufferAttribute(side, 1));
    geometry.setIndex(new BufferAttribute(new Uint16Array(indices), 1));

    const src = new Float32Array(count * 3);
    const dst = new Float32Array(count * 3);
    const seed = new Float32Array(count);
    const kind = new Float32Array(count);
    const weight = new Float32Array(count);
    const lit = new Float32Array(count);
    const pulse = new Float32Array(count).fill(NEVER);
    const colorA = new Float32Array(count * 3);
    const colorB = new Float32Array(count * 3);
    for (const e of this.edges) {
      const i = e.index;
      seed[i] = Math.random() * 100;
      kind[i] = e.kind === "link" ? 1 : 0;
      weight[i] = e.weight;
      const ca = typeColor(e.source.memType);
      const cb = typeColor(e.target.memType);
      colorA.set([ca.r, ca.g, ca.b], i * 3);
      colorB.set([cb.r, cb.g, cb.b], i * 3);
    }
    for (let i = this.edges.length; i < count; i += 1) {
      seed[i] = Math.random() * 100;
      kind[i] = 2;
      weight[i] = 1;
      lit[i] = 1;
      colorA.set([0.86, 0.8, 1.0], i * 3);
    }

    this.boltSrc = new InstancedBufferAttribute(src, 3);
    this.boltDst = new InstancedBufferAttribute(dst, 3);
    this.boltLit = new InstancedBufferAttribute(lit, 1);
    this.boltPulse = new InstancedBufferAttribute(pulse, 1);
    this.boltWeight = new InstancedBufferAttribute(weight, 1);
    geometry.setAttribute("aSrc", this.boltSrc);
    geometry.setAttribute("aDst", this.boltDst);
    geometry.setAttribute("aSeed", new InstancedBufferAttribute(seed, 1));
    geometry.setAttribute("aKind", new InstancedBufferAttribute(kind, 1));
    geometry.setAttribute("aWeight", this.boltWeight);
    geometry.setAttribute("aLit", this.boltLit);
    geometry.setAttribute("aPulse", this.boltPulse);
    geometry.setAttribute("aColorA", new InstancedBufferAttribute(colorA, 3));
    geometry.setAttribute("aColorB", new InstancedBufferAttribute(colorB, 3));
    geometry.instanceCount = this.edges.length;

    this.boltGeometry = geometry;
    this.bolts = new Mesh(geometry, this.boltMaterial);
    this.bolts.frustumCulled = false;
    this.scene.add(this.bolts);
    this.writeBoltEndpoints();
  }

  private buildDust(): void {
    const position = new Float32Array(DUST_COUNT * 3);
    const seed = new Float32Array(DUST_COUNT);
    for (let i = 0; i < DUST_COUNT; i += 1) {
      const u = Math.random() * 2 - 1;
      const phi = Math.random() * Math.PI * 2;
      const r = 1400 * Math.cbrt(Math.random());
      const s = Math.sqrt(1 - u * u);
      position[i * 3] = r * s * Math.cos(phi);
      position[i * 3 + 1] = r * s * Math.sin(phi) * 0.7;
      position[i * 3 + 2] = r * u;
      seed[i] = Math.random();
    }
    const geometry = new BufferGeometry();
    geometry.setAttribute("position", new Float32BufferAttribute(position, 3));
    geometry.setAttribute("aSeed", new Float32BufferAttribute(seed, 1));
    this.dust = new Points(geometry, this.dustMaterial);
    this.dust.frustumCulled = false;
    this.scene.add(this.dust);
  }

  private buildSimulation(): void {
    this.simulation?.stop();
    const r = this.radius;
    const link = forceLink<SkyNode>(this.edges.map((e) => ({ source: e.source, target: e.target })))
      .distance((_l, i) => {
        const e = this.edges[i];
        return e.kind === "link" ? 26 : 22 + (1 - e.weight) * 70;
      })
      .strength((_l, i) => {
        const e = this.edges[i];
        return e.kind === "link" ? 0.45 : 0.18 + 0.25 * e.weight;
      });
    const simulation = forceSimulation<SkyNode>(this.nodes, 3)
      .force("link", link)
      .force("charge", forceManyBody<SkyNode>().strength(-28).distanceMax(r * 1.6).theta(0.9))
      .force("center", forceCenter<SkyNode>(0, 0, 0).strength(0.04))
      // Meaning-space anchors: weak, so the layout keeps its own life but a
      // cluster stays in the same quarter of the sky from one visit to the next.
      .force("hx", forceX<SkyNode>((n) => (n.hint ? n.hint[0] * r * 0.85 : 0)).strength((n) => (n.hint ? 0.03 : 0)))
      .force("hy", forceY<SkyNode>((n) => (n.hint ? n.hint[1] * r * 0.85 : 0)).strength((n) => (n.hint ? 0.03 : 0)))
      .force("hz", forceZ<SkyNode>((n) => (n.hint ? n.hint[2] * r * 0.85 : 0)).strength((n) => (n.hint ? 0.03 : 0)))
      .alpha(1)
      // ~380 ticks for a small mind, ~230 for a big one — the big one's ticks
      // cost more, and nobody wants to watch 2,400 memories settle for 12s.
      .alphaDecay(this.nodes.length > 1200 ? 0.03 : 0.018)
      .velocityDecay(0.4)
      .stop();
    this.simulation = simulation;
  }

  private clearScene(): void {
    this.simulation?.stop();
    this.simulation = null;
    if (this.orbs) {
      this.scene.remove(this.orbs);
      this.orbGeometry?.dispose();
      this.orbs = null;
      this.orbGeometry = null;
    }
    if (this.bolts) {
      this.scene.remove(this.bolts);
      this.boltGeometry?.dispose();
      this.bolts = null;
      this.boltGeometry = null;
    }
    this.hits = [];
    this.hitNodes = [];
    this.selected = null;
    this.hovered = null;
    this.focus = 0;
    this.focusTarget = 0;
    this.tweens = [];
  }

  // ── per-frame ─────────────────────────────────────────────────────────

  private readonly loop = (): void => {
    if (this.disposed) return;
    this.frame = requestAnimationFrame(this.loop);
    const nowMs = performance.now();
    const dt = nowMs - this.lastFrameAt;
    this.lastFrameAt = nowMs;
    this.fps = this.fps * 0.92 + (1000 / Math.max(dt, 1)) * 0.08;
    this.guardPerformance(dt);

    const time = this.now();
    this.orbMaterial.uniforms.uTime.value = time;
    this.boltMaterial.uniforms.uTime.value = time;
    this.dustMaterial.uniforms.uTime.value = time;

    // focus eases toward its target; the sky sinks and rises, never snaps
    this.focus += (this.focusTarget - this.focus) * Math.min(1, dt / 220);
    this.orbMaterial.uniforms.uFocus.value = this.focus;
    this.boltMaterial.uniforms.uFocus.value = this.focus;

    if (this.simulation && !this.settled) {
      this.simulation.tick(1);
      if (this.simulation.alpha() < this.simulation.alphaMin()) {
        this.settled = true;
        this.emitStats();
      }
      this.writeOrbPositions();
      this.writeBoltEndpoints();
    }
    if (this.hitNodes.length) this.writeSpell();

    this.runTweens(nowMs);
    if (!this.controls.autoRotate && nowMs - this.lastInteractionAt > IDLE_RESUME_MS && !this.tweens.length) {
      this.controls.autoRotate = true;
    }
    this.controls.update();

    if (this.pointerDirty) this.pick();
    if (this.hitNodes.length === 0 && !this.selected && nowMs >= this.nextFireAt) this.fire(nowMs);

    this.renderer.render(this.scene, this.camera);
    if ((nowMs | 0) % 500 < dt) this.emitStats();
  };

  private now(): number {
    return (performance.now() - this.startedAt) / 1000;
  }

  private writeOrbPositions(): void {
    const arr = this.orbPosition.array as Float32Array;
    for (const n of this.nodes) {
      arr[n.index * 3] = n.x;
      arr[n.index * 3 + 1] = n.y;
      arr[n.index * 3 + 2] = n.z;
    }
    this.orbPosition.needsUpdate = true;
  }

  private writeBoltEndpoints(): void {
    const src = this.boltSrc.array as Float32Array;
    const dst = this.boltDst.array as Float32Array;
    for (const e of this.edges) {
      const i = e.index * 3;
      src[i] = e.source.x;
      src[i + 1] = e.source.y;
      src[i + 2] = e.source.z;
      dst[i] = e.target.x;
      dst[i + 1] = e.target.y;
      dst[i + 2] = e.target.z;
    }
    this.boltSrc.needsUpdate = true;
    this.boltDst.needsUpdate = true;
  }

  /** The query orb sits at the hits' centre, pulled toward the viewer, with
   *  a bolt to each hit. Re-written every frame — it follows the camera. */
  private writeSpell(): void {
    const centre = this.centroid(this.hitNodes);
    const toCamera = this.camera.position.clone().sub(centre).normalize();
    const q = centre.clone().add(toCamera.multiplyScalar(Math.max(40, this.radius * 0.18)));
    const qi = this.nodes.length;
    const pos = this.orbPosition.array as Float32Array;
    pos[qi * 3] = q.x;
    pos[qi * 3 + 1] = q.y;
    pos[qi * 3 + 2] = q.z;
    this.orbPosition.needsUpdate = true;

    const src = this.boltSrc.array as Float32Array;
    const dst = this.boltDst.array as Float32Array;
    this.hitNodes.forEach((n, k) => {
      const i = (this.edges.length + k) * 3;
      src[i] = q.x;
      src[i + 1] = q.y;
      src[i + 2] = q.z;
      dst[i] = n.x;
      dst[i + 1] = n.y;
      dst[i + 2] = n.z;
    });
    this.boltSrc.needsUpdate = true;
    this.boltDst.needsUpdate = true;
  }

  /** aLit for orbs and bolts from the current hits + selection. */
  private applyLighting(): void {
    const lit = this.orbLit.array as Float32Array;
    lit.fill(0, 0, this.nodes.length);
    const blit = this.boltLit.array as Float32Array;
    blit.fill(0, 0, this.edges.length);

    const bright = new Set<number>();
    for (const n of this.hitNodes) bright.add(n.index);
    if (this.selected) bright.add(this.selected.index);

    for (const index of bright) {
      lit[index] = 1;
      for (const ei of this.adjacency[index]) {
        const e = this.edges[ei];
        const other = e.source.index === index ? e.target : e.source;
        blit[ei] = Math.max(blit[ei], bright.has(other.index) ? 1 : 0.45);
        lit[other.index] = Math.max(lit[other.index], 0.32);
      }
    }
    this.orbLit.needsUpdate = true;
    this.boltLit.needsUpdate = true;

    // spell bolts: weight = score, instance count = hits
    const weight = this.boltWeight.array as Float32Array;
    this.hits.forEach((h, k) => {
      weight[this.edges.length + k] = Math.max(0.25, Math.min(1, h.score));
    });
    this.boltWeight.needsUpdate = true;
    if (this.boltGeometry) this.boltGeometry.instanceCount = this.edges.length + this.hitNodes.length;
    this.orbGeometry?.setDrawRange(0, this.nodes.length + (this.hitNodes.length ? 1 : 0));
  }

  /** A resting mind still fires: one memory lights, its bolts carry the
   *  charge, its neighbours catch it a beat later. Chain lightning. */
  private fire(nowMs: number): void {
    const [lo, hi] = FIRE_EVERY_MS;
    this.nextFireAt = nowMs + lo + Math.random() * (hi - lo);
    if (!this.nodes.length) return;
    // importance-weighted pick, cheap rejection sampling
    let node: SkyNode | null = null;
    for (let tries = 0; tries < 8 && !node; tries += 1) {
      const candidate = this.nodes[(Math.random() * this.nodes.length) | 0];
      if (Math.random() < 0.3 + 0.7 * candidate.importance) node = candidate;
    }
    node ??= this.nodes[(Math.random() * this.nodes.length) | 0];
    const t = this.now();
    this.orbPulse.setX(node.index, t);
    for (const ei of this.adjacency[node.index]) {
      const e = this.edges[ei];
      this.boltPulse.setX(ei, t + 0.1);
      const other = e.source === node ? e.target : e.source;
      this.orbPulse.setX(other.index, Math.max(this.orbPulse.getX(other.index), t + 0.28));
    }
    this.orbPulse.needsUpdate = true;
    this.boltPulse.needsUpdate = true;
  }

  // ── camera ────────────────────────────────────────────────────────────

  private centroid(nodes: SkyNode[]): Vector3 {
    const c = new Vector3();
    for (const n of nodes) c.add(new Vector3(n.x, n.y, n.z));
    return c.divideScalar(Math.max(nodes.length, 1));
  }

  private flyToCentroid(nodes: SkyNode[]): void {
    const centre = this.centroid(nodes);
    let spread = 0;
    for (const n of nodes) spread = Math.max(spread, centre.distanceTo(new Vector3(n.x, n.y, n.z)));
    this.flyToPoint(centre, Math.max(180, spread * 2.2 + 90));
  }

  private flyToPoint(target: Vector3, distance: number): void {
    this.controls.autoRotate = false;
    this.lastInteractionAt = performance.now();
    const fromTarget = this.controls.target.clone();
    const fromPosition = this.camera.position.clone();
    const direction = fromPosition.clone().sub(fromTarget).normalize();
    if (direction.lengthSq() < 0.001) direction.set(0, 0.2, 1).normalize();
    const toPosition = target.clone().add(direction.multiplyScalar(distance));
    this.tweens = [{
      start: performance.now(),
      duration: 1300,
      update: (t) => {
        const k = easeInOut(t);
        this.controls.target.lerpVectors(fromTarget, target, k);
        this.camera.position.lerpVectors(fromPosition, toPosition, k);
      },
    }];
  }

  private runTweens(nowMs: number): void {
    if (!this.tweens.length) return;
    const keep: Tween[] = [];
    for (const tw of this.tweens) {
      const t = Math.min(1, (nowMs - tw.start) / tw.duration);
      tw.update(t);
      if (t < 1) keep.push(tw);
      else tw.done?.();
    }
    this.tweens = keep;
  }

  // ── picking ───────────────────────────────────────────────────────────

  private readonly onPointerMove = (event: PointerEvent): void => {
    const rect = this.canvas.getBoundingClientRect();
    this.pointer = { x: event.clientX - rect.left, y: event.clientY - rect.top };
    this.pointerDirty = true;
  };

  private readonly onPointerLeave = (): void => {
    this.pointer = null;
    this.pointerDirty = false;
    if (this.hovered) {
      this.hovered = null;
      this.callbacks.onHover?.(null, 0, 0);
    }
  };

  private readonly onClick = (): void => {
    if (this.pointer) this.pick();
    this.callbacks.onSelect?.(this.hovered);
  };

  /** Nearest orb to the pointer in screen space, within PICK_RADIUS_PX.
   *  Projecting every node is cheap (thousands, once per frame at most) and
   *  works for Points, which a raycaster cannot see. */
  private pick(): void {
    this.pointerDirty = false;
    if (!this.pointer) return;
    const width = this.canvas.clientWidth;
    const height = this.canvas.clientHeight;
    const v = new Vector3();
    let best: SkyNode | null = null;
    let bestD = PICK_RADIUS_PX * PICK_RADIUS_PX;
    const dimmed = this.focus > 0.5;
    const lit = this.orbLit.array as Float32Array;
    for (const n of this.nodes) {
      if (dimmed && lit[n.index] < 0.3) continue; // sunk orbs are not there
      v.set(n.x, n.y, n.z).project(this.camera);
      if (v.z > 1) continue;
      const sx = (v.x + 1) * 0.5 * width;
      const sy = (1 - v.y) * 0.5 * height;
      const dx = sx - this.pointer.x;
      const dy = sy - this.pointer.y;
      const d = dx * dx + dy * dy;
      if (d < bestD) {
        bestD = d;
        best = n;
      }
    }
    if (best !== this.hovered) {
      this.hovered = best;
      this.canvas.style.cursor = best ? "pointer" : "";
      this.callbacks.onHover?.(best, this.pointer.x, this.pointer.y);
    } else if (best) {
      this.callbacks.onHover?.(best, this.pointer.x, this.pointer.y);
    }
  }

  // ── housekeeping ──────────────────────────────────────────────────────

  private resize(): void {
    const parent = this.canvas.parentElement ?? this.canvas;
    const width = Math.max(1, parent.clientWidth);
    const height = Math.max(1, parent.clientHeight);
    const ratio = Math.min(window.devicePixelRatio || 1, 1.5) * RENDER_SCALES[this.renderScaleIndex];
    this.renderer.setPixelRatio(ratio);
    this.renderer.setSize(width, height, false);
    this.camera.aspect = width / height;
    this.camera.updateProjectionMatrix();
    const proj = (height * ratio) / 2 / Math.tan((this.camera.fov * Math.PI) / 360);
    this.orbMaterial.uniforms.uProj.value = proj;
    this.boltMaterial.uniforms.uProj.value = proj;
    this.dustMaterial.uniforms.uProj.value = proj;
  }

  /** Step the render scale down when frames run long for a while, and back
   *  up after a long calm. Pixels are the bill on a weak GPU. */
  private guardPerformance(dt: number): void {
    // While the layout settles, the long frames are d3's — 2,400 nodes of
    // force ticks and 14k endpoint writes on the CPU. Fewer pixels can't
    // shorten them, so a step-down taken now is a step-down for nothing (s537
    // measured Studio pinned at 60% by exactly this). Judge only settled frames.
    if (!this.settled) {
      this.slowFrames = 0;
      this.calmFrames = 0;
      return;
    }
    if (dt > 40) {
      this.slowFrames += 1;
      this.calmFrames = 0;
    } else {
      this.slowFrames = Math.max(0, this.slowFrames - 0.25);
      this.calmFrames += 1;
    }
    if (this.slowFrames > 45 && this.renderScaleIndex < RENDER_SCALES.length - 1) {
      this.renderScaleIndex += 1;
      this.slowFrames = 0;
      this.calmFrames = 0;
      this.resize();
    } else if (this.calmFrames > 240 && this.fps > 55 && this.renderScaleIndex > 0) {
      this.renderScaleIndex -= 1;
      this.calmFrames = 0;
      this.resize();
    }
  }

  private emitStats(): void {
    this.callbacks.onStats?.({
      nodes: this.nodes.length,
      edges: this.edges.length,
      fps: Math.round(this.fps),
      renderScale: RENDER_SCALES[this.renderScaleIndex],
      settled: this.settled,
    });
  }
}
