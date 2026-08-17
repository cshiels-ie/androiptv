// Inline SVG icons. Emoji placeholders (📺 🎬 ☰) render as tofu boxes on
// Android devices without emoji fonts, so we use stroke icons instead.

function Svg(props: React.SVGProps<SVGSVGElement>) {
  return (
    <svg
      viewBox="0 0 24 24"
      width="20"
      height="20"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      {...props}
    />
  );
}

export function TvIcon() {
  return (
    <Svg>
      <rect x="2" y="6" width="20" height="13" rx="2" />
      <path d="m17 3-5 5-5-5" />
    </Svg>
  );
}

export function FilmIcon() {
  return (
    <Svg>
      <rect x="2" y="4" width="20" height="16" rx="2" />
      <path d="M2 9h20M2 15h20M7 9v-2m5 2v-2m5 2v-2M7 17v-2m5 2v-2m5 2v-2" />
    </Svg>
  );
}

export function HamburgerIcon() {
  return (
    <Svg>
      <path d="M4 7h16M4 12h16M4 17h16" />
    </Svg>
  );
}
