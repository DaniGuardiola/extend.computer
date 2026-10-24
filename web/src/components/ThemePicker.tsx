import { useEffect, useState } from 'react'

type Theme = 'system' | 'light' | 'dark'
const isTheme = (value: string | null): value is Theme =>
  value === 'system' || value === 'light' || value === 'dark'

export function ThemePicker() {
  const [theme, setTheme] = useState<Theme>('system')
  const [pointerFocus, setPointerFocus] = useState(false)
  useEffect(() => {
    const value = document.documentElement.dataset.theme ?? null
    if (isTheme(value)) setTheme(value)
  }, [])
  return (
    <label className="theme-picker inline-flex items-center gap-2 text-app-muted text-[11px]">
      <span className="theme-picker-label">Appearance</span>
      <select
        value={theme}
        data-pointer-focus={pointerFocus || undefined}
        onPointerDown={() => setPointerFocus(true)}
        onKeyDown={() => setPointerFocus(false)}
        onBlur={() => setPointerFocus(false)}
        onChange={(event) => {
          const value = event.target.value
          if (!isTheme(value)) return
          setTheme(value)
          document.documentElement.dataset.theme = value
          try {
            localStorage.setItem('extend-theme', value)
          } catch {}
        }}
      >
        <option value="system">System</option>
        <option value="light">Light</option>
        <option value="dark">Dark</option>
      </select>
    </label>
  )
}

export function ThemeFooter() {
  return (
    <footer className="theme-footer flex justify-center shrink-0">
      <ThemePicker />
    </footer>
  )
}

export function ThemeHeader() {
  return (
    <div className="theme-header absolute top-6 right-7">
      <ThemePicker />
    </div>
  )
}
