import type { CSSProperties } from "react";

/**
 * The welcome-hero mark for the empty/new-chat state — the Semantix icon
 * presented as a floating, lit 3D object. Ported from Semantix Studio's
 * shipping chat welcome state and built around the icon silhouette.
 *
 * Four composited motions stay on transform/opacity/background-position:
 * lm-enter, lm-float, lm-sweep, and lm-halo. The mirror below ripples through
 * an SVG water filter. Reduced-motion freezes every layer at rest.
 */

const ICON = "/semantix-icon.svg";
const ASPECT = "263 / 298";

const CSS = `
@keyframes lm-enter {
  0%   { opacity: 0; transform: translateY(26px) scale(0.9) rotate(-4deg); filter: blur(6px); }
  100% { opacity: 1; transform: translateY(0) scale(1) rotate(0deg); filter: blur(0); }
}
@keyframes lm-float {
  0%,100% { transform: translateY(0) rotate(0deg); }
  50%     { transform: translateY(-12px) rotate(0.6deg); }
}
@keyframes lm-sweep {
  0%   { background-position: 165% 0; }
  38%  { background-position: -65% 0; }
  100% { background-position: -65% 0; }
}
@keyframes lm-halo {
  0%,100% { opacity: 0.35; transform: translate(-50%,-50%) scale(1); }
  50%     { opacity: 0.7;  transform: translate(-50%,-50%) scale(1.12); }
}
.lm-stage  { animation: lm-enter 1.15s cubic-bezier(.2,.8,.2,1) both; }
.lm-floaty { animation: lm-float 7s ease-in-out infinite; animation-delay: 1.15s; }
.lm-halo   { animation: lm-halo 7s ease-in-out infinite; }
.lm-glint  { animation: lm-sweep 5.5s cubic-bezier(.5,0,.2,1) infinite; animation-delay: 1.4s; }
@media (prefers-reduced-motion: reduce) {
  .lm-stage, .lm-floaty, .lm-halo, .lm-glint { animation: none; }
}
`;

interface LogoMarkProps {
  size?: number;
}

export function LogoMark({ size = 96 }: LogoMarkProps) {
  const width = `${size}px`;

  return (
    <div
      className="lm-stage"
      style={{ display: "flex", flexDirection: "column", alignItems: "center" }}
    >
      <svg
        width="0"
        height="0"
        style={{ position: "absolute", pointerEvents: "none" }}
        aria-hidden="true"
      >
        <filter id="lm-water" x="-15%" y="-15%" width="130%" height="130%">
          <feTurbulence
            type="fractalNoise"
            baseFrequency="0.009 0.052"
            numOctaves={2}
            seed={7}
            result="noise"
          >
            <animate
              attributeName="baseFrequency"
              dur="9s"
              values="0.009 0.052; 0.011 0.064; 0.008 0.046; 0.009 0.052"
              repeatCount="indefinite"
            />
          </feTurbulence>
          <feDisplacementMap
            in="SourceGraphic"
            in2="noise"
            scale={18}
            xChannelSelector="R"
            yChannelSelector="G"
          >
            <animate
              attributeName="scale"
              dur="6s"
              values="12; 26; 12"
              repeatCount="indefinite"
            />
          </feDisplacementMap>
        </filter>
      </svg>

      <div className="lm-floaty" style={{ position: "relative", width, aspectRatio: ASPECT }}>
        <div
          className="lm-halo"
          style={{
            position: "absolute",
            left: "50%",
            top: "44%",
            width: "78%",
            height: "78%",
            transform: "translate(-50%,-50%)",
            borderRadius: "50%",
            background:
              "radial-gradient(circle, rgba(150,170,180,0.30), rgba(150,170,180,0) 70%)",
            filter: "blur(14px)",
            pointerEvents: "none",
          }}
        />
        <img
          src={ICON}
          alt="Semantix"
          draggable={false}
          style={{
            position: "relative",
            display: "block",
            width: "100%",
            height: "auto",
            filter: "drop-shadow(0 24px 60px rgba(0,0,0,0.6))",
          }}
        />
        <div
          className="lm-glint"
          style={{
            position: "absolute",
            inset: 0,
            WebkitMaskImage: `url(${ICON})`,
            maskImage: `url(${ICON})`,
            WebkitMaskRepeat: "no-repeat",
            maskRepeat: "no-repeat",
            WebkitMaskPosition: "center",
            maskPosition: "center",
            WebkitMaskSize: "contain",
            maskSize: "contain",
            background: `linear-gradient(108deg,
              rgba(255,255,255,0) 42%,
              rgba(255,255,255,0.35) 48%,
              rgba(255,255,255,0.95) 50%,
              rgba(255,255,255,0.35) 52%,
              rgba(255,255,255,0) 58%)`,
            backgroundSize: "260% 100%",
            backgroundRepeat: "no-repeat",
            mixBlendMode: "screen",
            pointerEvents: "none",
          } as CSSProperties}
        />
      </div>

      <div
        className="lm-reflection"
        style={{
          width,
          aspectRatio: ASPECT,
          marginTop: "-14px",
          WebkitMaskImage:
            "linear-gradient(to bottom, rgba(0,0,0,0.9), rgba(0,0,0,0.28) 42%, transparent 80%)",
          maskImage:
            "linear-gradient(to bottom, rgba(0,0,0,0.9), rgba(0,0,0,0.28) 42%, transparent 80%)",
          pointerEvents: "none",
        }}
      >
        <img
          src={ICON}
          alt=""
          aria-hidden="true"
          style={{
            display: "block",
            width: "100%",
            height: "auto",
            transform: "scaleY(-1)",
            opacity: 0.75,
            filter: "url(#lm-water)",
          }}
        />
      </div>

      <style>{CSS}</style>
    </div>
  );
}
