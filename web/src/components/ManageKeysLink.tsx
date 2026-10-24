import type { MouseEvent, ReactNode } from 'react'

export function ManageKeysLink({ children }: { children: ReactNode }) {
  function highlight(event: MouseEvent<HTMLAnchorElement>) {
    if (
      event.button !== 0 ||
      event.metaKey ||
      event.ctrlKey ||
      event.shiftKey ||
      event.altKey
    )
      return
    const heading = document.getElementById('credentials-title')
    const section = heading?.closest('section')
    if (!heading || !section) return
    heading.focus({ preventScroll: true })
    for (const animation of section.getAnimations())
      if (animation.id === 'credentials-highlight') animation.cancel()
    const reducedMotion = window.matchMedia(
      '(prefers-reduced-motion: reduce)',
    ).matches
    const color = 'color-mix(in srgb, var(--acid) 9%, transparent)'
    const visible = {
      borderRadius: '6px',
      backgroundColor: color,
      boxShadow: `0 0 0 12px ${color}`,
    }
    const hidden = {
      borderRadius: '6px',
      backgroundColor: 'transparent',
      boxShadow: '0 0 0 12px transparent',
    }
    const animation = section.animate(
      reducedMotion
        ? [visible, visible, hidden]
        : [hidden, { ...visible, offset: 0.2 }, visible, hidden],
      { duration: 1200, easing: reducedMotion ? 'steps(1, end)' : 'ease-out' },
    )
    animation.id = 'credentials-highlight'
  }
  return (
    <a className="text-button" href="#credentials-title" onClick={highlight}>
      {children}
    </a>
  )
}
