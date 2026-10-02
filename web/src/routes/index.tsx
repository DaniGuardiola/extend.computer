import { createFileRoute, Link } from '@tanstack/react-router'
import {
  ArrowUpRight,
  ArrowRight,
  MousePointer2,
  Pause,
  Play,
  ShieldCheck,
  Radio,
  Laptop,
  Terminal,
} from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import * as Ariakit from '@ariakit/react'
import { Brand } from '../components/Brand'

export const Route = createFileRoute('/')({ component: Landing })

const modes = [
  {
    id: 'extend',
    label: 'Extend display',
    title: 'More room for your next idea.',
    description:
      'Turn another Mac into an extra display. Spread your windows out and make a little more room to think.',
    detail: 'ONE DESKTOP. MORE SPACE.',
  },
  {
    id: 'share',
    label: 'Share input',
    title: 'One keyboard. Every computer.',
    description:
      'Slide your cursor between computers. Your keyboard follows, while each Mac keeps its own apps and workspace.',
    detail: 'YOUR KEYBOARD + MOUSE',
  },
  {
    id: 'remote',
    label: 'Remote desktop',
    title: 'Your other Mac. Right here.',
    description:
      'Open another Mac’s desktop in a window on yours. Use its apps and files while keeping your own workspace.',
    detail: 'ANOTHER MAC. WITHIN REACH.',
  },
  {
    id: 'mirror',
    label: 'Mirror screen',
    title: 'Your screen. A different seat.',
    description:
      'Bring one Mac’s screen onto another. See the same workspace and control it from wherever you’re sitting.',
    detail: 'SAME SCREEN. NEW PERSPECTIVE.',
  },
] as const

type Mode = (typeof modes)[number]['id']

function ShareCursor({ side }: { side: 'left' | 'right' }) {
  const ref = useRef<HTMLDivElement>(null)
  useEffect(() => {
    const cursor = ref.current!
    const screen = cursor.parentElement!
    const field = screen.querySelector<HTMLElement>(
      `.share-typing-${side} .share-typing-input`,
    )!
    let active = true
    const update = () => {
      if (!active) return
      const bounds = screen.getBoundingClientRect()
      const target = field.getBoundingClientRect()
      const tip = Number.parseFloat(
        getComputedStyle(cursor).getPropertyValue('--cursor-tip'),
      )
      cursor.style.setProperty(
        '--type-x',
        `${target.left + target.width * 0.9 - bounds.left - screen.clientLeft - tip}px`,
      )
      cursor.style.setProperty(
        '--type-y',
        `${target.top + target.height / 2 - bounds.top - screen.clientTop - tip}px`,
      )
    }
    const observer = new ResizeObserver(update)
    observer.observe(screen)
    observer.observe(field)
    update()
    void document.fonts.ready.then(update)
    return () => {
      active = false
      observer.disconnect()
    }
  }, [side])
  return (
    <div ref={ref} className={`share-cursor share-cursor-${side}`}>
      <MousePointer2 className="visual-cursor" fill="currentColor" />
      <span>YOU</span>
    </div>
  )
}

function ShareTyping({ side }: { side: 'left' | 'right' }) {
  const ref = useRef<HTMLDivElement>(null)
  const [terminal, setTerminal] = useState({
    history: ['connected', 'ready to type'],
    input: '',
  })
  useEffect(() => {
    const phrases =
      side === 'left'
        ? [
            'hello, desktop',
            'one keyboard',
            'keep going',
            'less switching',
            'make some room',
            'back to work',
            'ideas welcome',
            'stay in flow',
          ]
        : [
            'hello, laptop',
            'same keyboard',
            'new workspace',
            'keep creating',
            'over here now',
            'pick up here',
            'more ideas',
            'one smooth hop',
          ]
    const recent: string[] = []
    let deck: string[] = []
    const nextPhrase = () => {
      if (!deck.length) {
        deck = [...phrases]
        for (let i = deck.length - 1; i > 0; i--) {
          const j = Math.floor(Math.random() * (i + 1))
          ;[deck[i], deck[j]] = [deck[j], deck[i]]
        }
      }
      const index = deck.findIndex((phrase) => !recent.includes(phrase))
      const [phrase] = deck.splice(index, 1)
      recent.push(phrase)
      if (recent.length > 3) recent.shift()
      return phrase
    }
    const motion = matchMedia('(prefers-reduced-motion: reduce)')
    let frame = 0
    let animation: Animation | undefined
    let cycle = -1
    let submitted = false
    let phrase = ''
    let input = ''
    let history = ['connected', 'ready to type']
    const tick = () => {
      animation ??= ref.current
        ?.closest('.screen')
        ?.querySelector(`.share-cursor-${side}`)
        ?.getAnimations()
        .find(
          (item) =>
            (item as CSSAnimation).animationName === `share-${side}-path`,
        )
      if (animation && typeof animation.currentTime === 'number') {
        const duration = Number(animation.effect!.getTiming().duration)
        const currentCycle = Math.floor(animation.currentTime / duration)
        const progress = (animation.currentTime % duration) / duration
        if (currentCycle !== cycle) {
          cycle = currentCycle
          phrase = nextPhrase()
          submitted = false
        }
        const start = side === 'left' ? 0.06 : 0.46
        const finish = side === 'left' ? 0.2 : 0.68
        const enter = side === 'left' ? 0.22 : 0.7
        const count = Math.max(
          0,
          Math.min(
            phrase.length,
            Math.floor(((progress - start) / (finish - start)) * phrase.length),
          ),
        )
        let nextInput = submitted ? '' : phrase.slice(0, count)
        if (!submitted && progress >= enter) {
          history = [...history.slice(-1), phrase]
          submitted = true
          nextInput = ''
          setTerminal({ history, input: '' })
        } else if (nextInput !== input) {
          setTerminal({ history, input: nextInput })
        }
        input = nextInput
      }
      frame = requestAnimationFrame(tick)
    }
    const start = () => {
      cancelAnimationFrame(frame)
      animation = undefined
      if (motion.matches) {
        setTerminal({ history: ['connected', phrases[0]], input: '' })
      } else {
        frame = requestAnimationFrame(tick)
      }
    }
    start()
    motion.addEventListener('change', start)
    return () => {
      cancelAnimationFrame(frame)
      motion.removeEventListener('change', start)
    }
  }, [side])
  return (
    <div ref={ref} className={`share-typing share-typing-${side}`}>
      <div className="share-terminal-history">
        {terminal.history.map((line, index) => (
          <div key={index}>› {line}</div>
        ))}
      </div>
      <div className="share-typing-input">
        <span className="accent">›</span>
        <span className="share-typed-text">{terminal.input}</span>
      </div>
    </div>
  )
}

function Workspace({ side, mode }: { side: 'left' | 'right'; mode: Mode }) {
  if (mode === 'remote' && side === 'right') {
    return (
      <div className="workspace-picture remote-local-workspace">
        <div className="remote-local-note">MY WORKSPACE</div>
        <div className="remote-viewer">
          <div className="remote-viewer-bar">
            <span>● ● ●</span>
            <span>DESKTOP MAC</span>
            <i className="status-dot" />
          </div>
          <div className="remote-viewport">
            <Workspace side="left" mode="remote" />
          </div>
        </div>
      </div>
    )
  }
  return (
    <div className={`workspace-picture workspace-${side}`}>
      <div className="workspace-grid" />
      {mode === 'remote' ? (
        <div className="remote-cursor">
          <MousePointer2 fill="currentColor" />
        </div>
      ) : null}
      <div className="workspace-window">
        <div className="workspace-window-bar">
          ● ● ●{' '}
          <span>
            {mode === 'remote'
              ? 'REMOTE WORKSPACE'
              : mode === 'mirror'
                ? 'THE SAME WORKSPACE'
                : side === 'left'
                  ? 'ROOM TO THINK'
                  : 'ROOM TO MAKE'}
          </span>
        </div>
        <div className="workspace-art">
          <i />
          <i />
          <i />
        </div>
        <span className="workspace-caption">Good things take space.</span>
        {mode === 'extend' || mode === 'mirror' ? (
          <div
            className={`extend-cursor ${mode === 'mirror' ? 'mirror-drag-cursor' : ''}`}
          >
            <MousePointer2 fill="currentColor" />
          </div>
        ) : null}
      </div>
    </div>
  )
}

function Desk() {
  const [mode, setMode] = useState<Mode>('extend')
  const [paused, setPaused] = useState(false)
  const current = modes.find((item) => item.id === mode)!
  const tabs = Ariakit.useTabStore({
    selectedId: mode,
    setSelectedId: (id) => setMode(id as Mode),
  })
  return (
    <section
      className={`desk-visual mode-${mode} ${paused ? 'demo-paused' : ''}`}
      aria-label="Explore the four modes"
    >
      <div className="desk-topline">
        <span>
          <i className="status-dot" /> TWO MACS. ONE FLOW.
        </span>
        <span>01 — 02</span>
      </div>
      <Ariakit.TabList
        store={tabs}
        className="mode-tabs"
        aria-label="Connection mode"
      >
        {modes.map((item) => (
          <Ariakit.Tab key={item.id} id={item.id} className="mode-tab">
            {item.label}
          </Ariakit.Tab>
        ))}
      </Ariakit.TabList>
      <Ariakit.TabPanel store={tabs} tabId={mode} className="mode-panel">
        <div className="computers" aria-hidden="true">
          <div className="computer monitor">
            <div className="screen">
              <div className="screen-toolbar">
                <span className="window-dots">● ● ●</span>
                <span>{mode === 'remote' ? 'REMOTE MAC' : 'DESKTOP'}</span>
              </div>
              {mode === 'share' ? (
                <div className="screen-content">
                  <div className="screen-greeting">
                    Make room
                    <br />
                    for more<span>.</span>
                  </div>
                  <div className="fake-window">
                    <span>~/good-things</span>
                    <ShareTyping side="left" />
                  </div>
                </div>
              ) : (
                <Workspace side="left" mode={mode} />
              )}
              {mode === 'share' ? <ShareCursor side="left" /> : null}
            </div>
            <div className="monitor-chin">
              <div />
            </div>
            <div className="monitor-stand" />
          </div>
          <div className="computer laptop">
            <div className="screen">
              <div className="screen-toolbar">
                <span className="window-dots">● ● ●</span>
                <span>
                  {mode === 'extend'
                    ? 'EXTRA DISPLAY'
                    : mode === 'remote'
                      ? 'THIS MAC'
                      : 'LAPTOP'}
                </span>
              </div>
              {mode === 'share' ? (
                <div className="laptop-wallpaper">
                  <span>
                    Keep
                    <br />
                    your flow<span className="accent">.</span>
                  </span>
                  <div className="wallpaper-orbit" />
                  <ShareTyping side="right" />
                </div>
              ) : (
                <Workspace side="right" mode={mode} />
              )}
              {mode === 'share' ? <ShareCursor side="right" /> : null}
            </div>
            <div className="laptop-base">
              <div />
            </div>
          </div>
        </div>
        <div className="mode-description">
          <h2>{current.title}</h2>
          <p>{current.description}</p>
        </div>
      </Ariakit.TabPanel>
      <div className="desk-bottomline">
        <span>{current.detail}</span>
        <button
          onClick={() => setPaused((value) => !value)}
          className="demo-control"
          aria-pressed={paused}
          aria-label={paused ? 'Play demo animation' : 'Pause demo animation'}
        >
          {paused ? <Play size={14} /> : <Pause size={14} />}
          {paused ? 'Play demo' : 'Pause demo'}
        </button>
      </div>
    </section>
  )
}

function Landing() {
  return (
    <div className="landing">
      <header className="site-header">
        <Brand />
        <nav aria-label="Main navigation">
          <a href="#how-it-works" className="nav-about">
            How it works
          </a>
          <Link to="/login">
            Sign in <ArrowUpRight size={16} />
          </Link>
        </nav>
      </header>
      <main id="main">
        <section className="hero">
          <div className="hero-copy">
            <div className="eyebrow">
              <span className="mini-pill">EARLY ACCESS</span>
              <span>MADE FOR MACOS</span>
            </div>
            <h1>
              One desk.
              <br />
              <span>Zero borders.</span>
            </h1>
            <p className="hero-description">
              Your screens, keyboard and mouse,
              <br className="desktop-break" /> across all your Macs. Keep going.
            </p>
            <div className="hero-actions">
              <Link to="/signup" className="button primary">
                Get started <ArrowUpRight size={20} />
              </Link>
              <a href="#how-it-works" className="source-link">
                See how it works <ArrowUpRight size={16} />
              </a>
            </div>
            <p className="hero-footnote">Free to use. Yours to host.</p>
          </div>
          <div className="hero-index" aria-hidden="true">
            <span>EXTEND YOUR FLOW</span>
            <span>NOT YOUR TO-DO LIST</span>
          </div>
        </section>
        <Desk />
        <section className="intro-section" id="how-it-works">
          <div>
            <p className="eyebrow">LESS SWITCHING. MORE DOING.</p>
            <h2>
              Your hands.
              <br />
              All your computers.
            </h2>
          </div>
          <p>
            Turn another Mac into an extra display.
            <br className="desktop-break" /> Share your keyboard. Mirror your
            screen.
            <br className="desktop-break" /> Control another desktop from yours.
            <br className="desktop-break" /> Choose how your computers work
            together.
          </p>
        </section>
        <section className="feature-grid" aria-label="Built for your desk">
          <article>
            <Radio size={24} />
            <span className="feature-number">01</span>
            <h3>Find your other Mac.</h3>
            <p>
              Pair nearby computers on your network. Local sharing works without
              an account.
            </p>
          </article>
          <article>
            <ShieldCheck size={24} />
            <span className="feature-number">02</span>
            <h3>Your connection. Private.</h3>
            <p>
              Encrypted connections and permission to connect. Your devices stay
              in your control.
            </p>
          </article>
          <article>
            <Terminal size={24} />
            <span className="feature-number">03</span>
            <h3>Make yourself at home.</h3>
            <p>
              Use the desktop app or the CLI. Run your own account server
              whenever you want.
            </p>
          </article>
        </section>
        <section className="account-strip">
          <div className="strip-icon">
            <Laptop size={30} />
          </div>
          <div>
            <p className="eyebrow">YOUR DESK, REMEMBERED</p>
            <h2>A home for your devices.</h2>
            <p>
              Sign in once. Find your computers in one place.
              <br />
              Your desk, ready wherever you are.
            </p>
          </div>
          <Link to="/signup" className="button light">
            Create an account <ArrowRight size={19} />
          </Link>
        </section>
      </main>
      <footer className="site-footer">
        <Brand />
        <span>Less friction. More flow.</span>
        <a href="#how-it-works">
          Self-hostable by design <ArrowUpRight size={15} />
        </a>
      </footer>
    </div>
  )
}
