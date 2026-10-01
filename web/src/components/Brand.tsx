import { Link } from '@tanstack/react-router'

export function Brand() {
  return (
    <Link to="/" className="brand" aria-label="extend.computer home">
      <svg
        width="28"
        height="24"
        viewBox="0 0 28 24"
        fill="none"
        aria-hidden="true"
      >
        <path
          d="M2 3h10v15H2zM16 3h10v15H16zM6 22h16M14 18v4"
          stroke="currentColor"
          strokeWidth="2"
          strokeLinejoin="round"
        />
      </svg>
      <span>
        extend<span className="brand-dot">.</span>computer
      </span>
    </Link>
  )
}
