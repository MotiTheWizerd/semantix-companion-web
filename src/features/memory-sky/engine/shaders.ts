// The sky's three materials, as GLSL. Everything is ADDITIVE: light on a void,
// no sorting, no depth writes — which is what lets the whole graph be three
// draw calls (orbs, bolts, dust) instead of one object per memory. The
// "lightning" is a camera-facing ribbon per edge whose kinks are re-rolled
// `uFlicker` times a second in the vertex shader; the CPU only ever moves
// endpoints.

const NOISE = /* glsl */ `
  float hash21(vec2 p) {
    vec3 p3 = fract(vec3(p.xyx) * 0.1031);
    p3 += dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
  }
  // Value noise along the bolt: ~6 kinks, re-rolled whenever tq steps.
  float crackle(float seed, float t, float tq) {
    float x = t * 6.0;
    float i = floor(x);
    float f = fract(x);
    f = f * f * (3.0 - 2.0 * f);
    float a = hash21(vec2(i, seed + tq));
    float b = hash21(vec2(i + 1.0, seed + tq));
    return mix(a, b, f) * 2.0 - 1.0;
  }
`;

// ── ORBS ──────────────────────────────────────────────────────────────────
// One THREE.Points. Size = importance, heat = access_count, tint = type.
// `aLit` marks the current highlight set; `aPulse` is the time a pulse began
// (a huge negative means never) and decays in-shader, so a firing costs one
// float write and no per-frame CPU.

export const ORB_VERTEX = /* glsl */ `
  uniform float uTime;
  uniform float uProj;      // (viewport height / 2) / tan(fov / 2), in device pixels
  uniform float uMaxPoint;  // the GPU's largest point sprite
  uniform float uFocus;     // 1 while a search holds the sky; unlit orbs sink
  attribute vec3 aColor;
  attribute float aSize;    // world diameter
  attribute float aHeat;
  attribute float aSeed;
  attribute float aLit;
  attribute float aPulse;
  attribute float aGhost;   // archived
  attribute float aHide;    // the filter sank it (a hit or the open memory never is)
  varying vec3 vColor;
  varying float vHeat;
  varying float vEnergy;
  varying float vDim;
  varying float vGhost;

  void main() {
    vec4 mv = modelViewMatrix * vec4(position, 1.0);
    float pulse = uTime >= aPulse ? exp(-(uTime - aPulse) * 1.6) : 0.0;
    float energy = max(aLit, pulse) * (1.0 - aHide);
    float breathe = 1.0 + 0.06 * sin(uTime * 1.7 + aSeed * 6.2831);
    float world = aSize * breathe * (1.0 + 0.55 * energy);
    float depth = max(-mv.z, 1.0);
    // uMaxPoint is the smaller of the GPU's limit and a taste ceiling: an orb
    // you fly into should fill a hand, not the window.
    gl_PointSize = min(world * uProj / depth, uMaxPoint);
    gl_Position = projectionMatrix * mv;

    vColor = aColor;
    vHeat = aHeat;
    vEnergy = energy;
    vGhost = aGhost;
    float dim = mix(1.0, 0.10, uFocus * (1.0 - max(aLit, pulse)));
    dim *= 0.15 + 0.85 * smoothstep(1800.0, 250.0, depth);
    // sunk by the filter: a trace, so the shape of the mind stays legible
    dim *= mix(1.0, 0.06, aHide);
    vDim = dim;
  }
`;

export const ORB_FRAGMENT = /* glsl */ `
  precision highp float;
  varying vec3 vColor;
  varying float vHeat;
  varying float vEnergy;
  varying float vDim;
  varying float vGhost;

  void main() {
    vec2 uv = gl_PointCoord * 2.0 - 1.0;
    float d = length(uv);
    if (d > 1.0) discard;
    float core = smoothstep(0.22 + 0.08 * vEnergy, 0.0, d);
    float halo = pow(1.0 - d, 3.0) * (0.55 + 0.45 * vHeat);
    vec3 col = vColor * halo
             + vec3(0.90, 0.95, 1.0) * core * (0.85 + 0.5 * vEnergy)
             + vColor * vEnergy * pow(1.0 - d, 2.0) * 0.8;
    col *= vDim * mix(1.0, 0.35, vGhost);
    gl_FragColor = vec4(col, 1.0);
  }
`;

// ── BOLTS ─────────────────────────────────────────────────────────────────
// One INSTANCED mesh: the base geometry is a single ribbon of SEGMENTS quads
// (aT along it, aSide ±1 for the rail); every edge is one instance carrying
// its endpoints (aSrc/aDst — the only attributes the CPU rewrites, and only
// while the layout still moves), seed, kind, weight, colours and lighting.
// The kinks and the camera-facing width are computed here, per vertex.

export const BOLT_SEGMENTS = 9;

export const BOLT_VERTEX = /* glsl */ `
  uniform float uTime;
  uniform float uWidth;     // half-width of a resting semantic bolt, in DEVICE PIXELS
  uniform float uProj;      // (viewport height / 2) / tan(fov / 2), device pixels
  uniform float uFocus;
  uniform float uFlicker;   // re-rolls per second
  uniform float uDensity;   // 1 for a sparse mind, sinks as edges pile up (additive saturation)
  uniform float uTravel;    // seconds a strike's leader takes to cross a bolt
  attribute vec3 aSrc;
  attribute vec3 aDst;
  attribute float aT;
  attribute float aSide;
  attribute float aSeed;
  attribute float aKind;    // 0 semantic · 1 written link · 2 the spell (query → hit)
  attribute float aWeight;
  attribute float aLit;
  attribute float aPulse;
  attribute float aFrom;    // 0: the strike leaves aSrc · 1: it leaves aDst
  attribute float aHide;    // the filter sank an end of it
  attribute vec3 aColorA;
  attribute vec3 aColorB;
  varying float vSide;
  varying float vT;
  varying vec3 vColor;
  varying float vIntensity;

  ${NOISE}

  void main() {
    vec3 dir = aDst - aSrc;
    float len = length(dir);
    dir = len > 0.0001 ? dir / len : vec3(0.0, 1.0, 0.0);
    vec3 up = abs(dir.y) < 0.99 ? vec3(0.0, 1.0, 0.0) : vec3(1.0, 0.0, 0.0);
    vec3 n1 = normalize(cross(dir, up));
    vec3 n2 = cross(dir, n1);

    // A strike in two acts, like the real thing: a thin LEADER runs from the
    // firing end to the far end over uTravel, then the RETURN STROKE lights
    // the whole channel at once and decays. Before the leader reaches a
    // station the station is dark; behind it a short tail glows.
    float age = uTime - aPulse;
    float tt = mix(aT, 1.0 - aT, aFrom);          // distance from the firing end
    float front = age / uTravel;
    float leader = smoothstep(0.14, 0.0, abs(tt - front)) * 0.85
                 + step(tt, front) * 0.3 * exp(-(front - tt) * 5.0);
    float stroke = exp(-(age - uTravel) * 1.8);
    float pulse = age < 0.0 ? 0.0 : (age < uTravel ? leader : stroke);
    float energy = max(aLit, pulse) * (1.0 - aHide);
    float spell = step(1.5, aKind);
    // A written link crackles; a semantic neighbour barely stirs unless lit.
    float alive = mix(0.35 + 0.65 * aKind, 1.0, energy) + spell;
    float tq = floor(uTime * uFlicker * (0.5 + 0.5 * alive));
    float env = sin(aT * 3.14159);
    float amp = len * (0.012 + 0.05 * energy + 0.03 * spell) * env * (0.5 + 0.5 * alive);
    vec3 p = mix(aSrc, aDst, aT)
           + n1 * crackle(aSeed, aT, tq) * amp
           + n2 * crackle(aSeed + 17.0, aT, tq + 3.0) * amp
           + n1 * crackle(aSeed + 41.0, aT * 2.7, tq * 1.7) * amp * 0.35;

    vec4 mv = modelViewMatrix * vec4(p, 1.0);
    vec3 dirV = (modelViewMatrix * vec4(dir, 0.0)).xyz;
    vec3 perp = cross(normalize(dirV), vec3(0.0, 0.0, 1.0));
    float pl = length(perp);
    perp = pl > 0.0001 ? perp / pl : vec3(1.0, 0.0, 0.0);
    // Constant on screen: a bolt is a thin bright thing at every distance.
    // Close up it never becomes a highway; far away it never dissolves into
    // sub-pixel dashes.
    float depth = max(-mv.z, 1.0);
    float px = uWidth * (0.8 + 0.5 * min(aKind, 1.0)) * (1.0 + 1.6 * energy + 1.4 * spell);
    mv.xyz += perp * aSide * (px * depth / uProj);
    gl_Position = projectionMatrix * mv;

    vSide = aSide;
    vT = aT;
    vColor = mix(aColorA, aColorB, aT);
    float base = mix(0.26 * aWeight * aWeight, 0.7, min(aKind, 1.0)) * uDensity + 1.1 * spell;
    float dimmed = mix(1.0, 0.05, uFocus * (1.0 - max(aLit, spell)));
    dimmed *= mix(1.0, 0.04, aHide);
    vIntensity = (base * dimmed + energy * (0.9 + 0.6 * uDensity))
               * (0.25 + 0.75 * smoothstep(2200.0, 200.0, depth));
  }
`;

export const BOLT_FRAGMENT = /* glsl */ `
  precision highp float;
  varying float vSide;
  varying float vT;
  varying vec3 vColor;
  varying float vIntensity;

  void main() {
    float x = abs(vSide);
    float core = smoothstep(0.42, 0.0, x);
    float glow = pow(1.0 - x, 2.5);
    vec3 col = vColor * glow * 0.9 + vec3(0.86, 0.93, 1.0) * core * 1.1;
    float ends = smoothstep(0.0, 0.05, vT) * smoothstep(1.0, 0.95, vT);
    gl_FragColor = vec4(col * vIntensity * ends, 1.0);
  }
`;

// ── DUST ──────────────────────────────────────────────────────────────────
// Motes in the void: depth cues and a sense of air. Positions drift in the
// shader; the buffer is written once.

export const DUST_VERTEX = /* glsl */ `
  uniform float uTime;
  uniform float uProj;
  attribute float aSeed;
  varying float vFade;

  void main() {
    vec3 p = position;
    p.x += sin(uTime * 0.05 + aSeed * 6.2831) * 18.0;
    p.y += cos(uTime * 0.04 + aSeed * 12.566) * 14.0;
    p.z += sin(uTime * 0.03 + aSeed * 3.1415) * 18.0;
    vec4 mv = modelViewMatrix * vec4(p, 1.0);
    float depth = max(-mv.z, 1.0);
    gl_PointSize = clamp(2.2 * uProj / depth, 1.0, 3.0);
    gl_Position = projectionMatrix * mv;
    vFade = (0.35 + 0.65 * fract(aSeed * 7.13)) * smoothstep(2200.0, 200.0, depth);
    vFade *= 0.6 + 0.4 * sin(uTime * 0.8 + aSeed * 40.0);
  }
`;

export const DUST_FRAGMENT = /* glsl */ `
  precision highp float;
  varying float vFade;
  void main() {
    vec2 uv = gl_PointCoord * 2.0 - 1.0;
    float d = length(uv);
    if (d > 1.0) discard;
    gl_FragColor = vec4(vec3(0.55, 0.68, 1.0) * (1.0 - d) * vFade * 0.35, 1.0);
  }
`;
