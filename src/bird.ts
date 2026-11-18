/**
 * The header's shoebill: counter-logo.svg drawn inline in currentColor, with the beak split into the upper bill and a
 * lower jaw (`.jaw`) that can rotate about its hinge at (133.5, 121.5), the back edge of the original beak at t = 0.5.
 * Closed, the two parts cover exactly the original beak; the jaw's top edge runs 3 units inside the upper bill so no
 * anti-aliased seam shows between them.
 */
const HEAD = `<path d="M58 92 L42 46 L72 64 L76 20 L98 56 L116 24 L122 60 L146 48 L138 90 Z"/>
    <path d="M150 92 C140 62 110 50 88 56 C58 64 44 92 50 122 C54 146 66 160 72 178 L70 256 L128 256 C124 200 128 170 140 156 Z"/>
    <path d="M144 84 C180 78 212 88 228 104 C238 114 236 130 226 134 C220 136 214 132 218 126 C190 128 160 124 133.5 121.5 C134.5 107.5 138 94 144 84 Z"/>
    <g class="jaw">
      <path d="M133.5 121.5 L134 118.5 C160 121 190 125 218 123 L218 126 C198 148 168 160 138 162 C134 150 132.5 135.5 133.5 121.5 Z"/>
    </g>`;
const EYE = `<circle cx="112" cy="102" r="14" fill="#000"/>`;

/*
 * The glow: `.sweep` is a 600 × 240 band (three bird widths, as a CSS background at 300% 100%) whose gradient matches
 * linear-gradient(105deg) in the band's own space, shown only through `bird-shine`, a second copy of the bird. Moving
 * `.sweep` 400 units right is background-position 100% → 0. The copy keeps its own `.jaw`, so a yawn must move both.
 */
const MARKUP = `<svg class="bird" viewBox="40 16 200 240" aria-hidden="true" focusable="false">
  <mask id="bird-eye" maskUnits="userSpaceOnUse" x="0" y="0" width="256" height="256">
    <rect width="256" height="256" fill="#fff"/>
    ${EYE}
  </mask>
  <mask id="bird-shine" maskUnits="userSpaceOnUse" x="0" y="0" width="256" height="256">
    <g fill="#fff">${HEAD}</g>
    ${EYE}
  </mask>
  <linearGradient id="bird-sweep" gradientUnits="userSpaceOnUse" x1="-369.9" y1="53" x2="249.9" y2="219">
    <stop offset="0.35" class="sweep-edge"/>
    <stop offset="0.5"/>
    <stop offset="0.65" class="sweep-edge"/>
  </linearGradient>
  <g class="bird-body" fill="currentColor" mask="url(#bird-eye)">
    ${HEAD}
  </g>
  <g mask="url(#bird-shine)">
    <rect class="sweep" x="-360" y="16" width="600" height="240" fill="url(#bird-sweep)"/>
  </g>
</svg>`;

export function bird(): SVGSVGElement {
  const template = document.createElement("template");
  template.innerHTML = MARKUP;
  return template.content.firstElementChild as SVGSVGElement;
}

const reducedMotion = () => matchMedia("(prefers-reduced-motion: reduce)").matches;

/** Under reduced motion, the glow and the yawn are both this fade to the accent and back. */
const fade = (svg: SVGSVGElement) => svg.animate({ color: ["var(--fg)", "var(--accent)", "var(--fg)"] }, 1500);

/** An accent band sweeping across the bird, for a successful refresh. */
export function glow(svg: SVGSVGElement): Animation {
  if (reducedMotion()) return fade(svg);
  return svg.querySelector(".sweep")!.animate(
    { transform: ["translateX(0)", "translateX(400px)"], visibility: ["visible", "visible"] },
    { duration: 600, easing: "cubic-bezier(.2,.7,.2,1)" },
  );
}

/** The jaw opening 20° and closing: 300 ms open, 400 ms hold, 500 ms close. */
export function yawn(svg: SVGSVGElement): Animation {
  if (reducedMotion()) return fade(svg);
  const keyframes: Keyframe[] = [
    { transform: "rotate(0deg)", easing: "ease-out" },
    { transform: "rotate(20deg)", offset: 3 / 12 },
    { transform: "rotate(20deg)", offset: 7 / 12, easing: "cubic-bezier(.4,0,.2,1)" },
    { transform: "rotate(0deg)" },
  ];
  // The visible jaw and its copy in the glow's mask; both run the same timing.
  return [...svg.querySelectorAll(".jaw")].map((jaw) => jaw.animate(keyframes, 1200))[0];
}
