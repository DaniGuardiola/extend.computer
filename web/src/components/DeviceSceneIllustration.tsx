export function DeviceSceneIllustration({
  scene = 'lost',
}: {
  scene?: 'lost' | 'error'
}) {
  const isError = scene === 'error'
  return (
    <svg
      className="not-found-illustration"
      width="450"
      height="210"
      viewBox="0 0 450 210"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <defs>
        <linearGradient
          id="missing-page-background"
          x1="0"
          y1="0"
          x2="1"
          y2="1"
        >
          <stop stopColor="#34433c" />
          <stop offset="1" stopColor="#222d28" />
        </linearGradient>
        <clipPath id="missing-page-laptop">
          <rect x="24" y="30" width="218" height="138" rx="3" />
        </clipPath>
        <clipPath id="missing-page-tablet">
          <rect x="296" y="20" width="112" height="148" rx="5" />
        </clipPath>
        <g id="missing-page-media">
          <image
            className="not-found-motion"
            href={
              isError
                ? '/media/this-is-fine.gif'
                : '/media/confused-travolta.gif'
            }
            x={isError ? 34 : 50}
            y={isError ? -20 : 18}
            width={isError ? 398 : 420}
            height={isError ? 225 : 210}
          />
          <image
            className="not-found-still"
            href={
              isError
                ? '/media/this-is-fine-still.png'
                : '/media/confused-travolta-still.png'
            }
            x={isError ? 34 : 18}
            y={isError ? -20 : 26}
            width={isError ? 398 : 420}
            height={isError ? 225 : 210}
          />
        </g>
      </defs>
      <rect x="15" y="20" width="236" height="159" rx="9" fill="var(--panel)" />
      <rect
        x="24"
        y="30"
        width="218"
        height="138"
        rx="3"
        fill="url(#missing-page-background)"
        stroke="var(--line)"
      />
      <path
        d="M4 179h258v2c0 5-8 8-16 8H20c-8 0-16-3-16-8z"
        fill="var(--panel)"
      />
      <path d="M111 179h44l-3 3h-38z" fill="var(--line)" stroke="none" />
      <rect
        x="286"
        y="8"
        width="132"
        height="172"
        rx="13"
        fill="var(--panel)"
      />
      <rect
        x="296"
        y="20"
        width="112"
        height="148"
        rx="5"
        fill="url(#missing-page-background)"
        stroke="var(--line)"
      />
      <circle cx="352" cy="14" r="1.4" fill="currentColor" stroke="none" />
      <g clipPath="url(#missing-page-laptop)" stroke="none">
        <use href="#missing-page-media" />
      </g>
      <g clipPath="url(#missing-page-tablet)" stroke="none">
        <use href="#missing-page-media" transform="translate(54 0)" />
      </g>
    </svg>
  )
}
