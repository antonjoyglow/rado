/**
 * The header's shoebill: counter-logo.svg drawn inline in currentColor, with the beak split into the upper bill and a
 * lower jaw (`.jaw`) that can rotate about its hinge at (133.5, 121.5), the back edge of the original beak at t = 0.5.
 * Closed, the two parts cover exactly the original beak; the jaw's top edge runs 3 units inside the upper bill so no
 * anti-aliased seam shows between them.
 */
const MARKUP = `<svg class="bird" viewBox="40 16 200 240" aria-hidden="true" focusable="false">
  <mask id="bird-eye" maskUnits="userSpaceOnUse" x="0" y="0" width="256" height="256">
    <rect width="256" height="256" fill="#fff"/>
    <circle cx="112" cy="102" r="14" fill="#000"/>
  </mask>
  <g class="bird-body" fill="currentColor" mask="url(#bird-eye)">
    <path d="M58 92 L42 46 L72 64 L76 20 L98 56 L116 24 L122 60 L146 48 L138 90 Z"/>
    <path d="M150 92 C140 62 110 50 88 56 C58 64 44 92 50 122 C54 146 66 160 72 178 L70 256 L128 256 C124 200 128 170 140 156 Z"/>
    <path d="M144 84 C180 78 212 88 228 104 C238 114 236 130 226 134 C220 136 214 132 218 126 C190 128 160 124 133.5 121.5 C134.5 107.5 138 94 144 84 Z"/>
    <g class="jaw">
      <path d="M133.5 121.5 L134 118.5 C160 121 190 125 218 123 L218 126 C198 148 168 160 138 162 C134 150 132.5 135.5 133.5 121.5 Z"/>
    </g>
  </g>
</svg>`;

export function bird(): SVGSVGElement {
  const template = document.createElement("template");
  template.innerHTML = MARKUP;
  return template.content.firstElementChild as SVGSVGElement;
}
