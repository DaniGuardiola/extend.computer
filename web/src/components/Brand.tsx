import { Link } from '@tanstack/react-router'

export function Brand() {
  return (
    <Link to="/" className="brand" aria-label="extend.computer home">
      <svg
        width="34"
        height="34"
        viewBox="110 110 292 330"
        fill="none"
        aria-hidden="true"
      >
        <path
          fill="currentColor"
          d="M138 137h236v126H214v-34h126v-58H172v139h178v34H138V137Zm101 207h34v42h61v34H178v-34h61v-42Z"
        />
      </svg>
      <span>
        extend<span className="brand-dot">.</span>computer
      </span>
    </Link>
  )
}
