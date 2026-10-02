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
import { useState } from 'react'
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
    id: 'mirror',
    label: 'Mirror screen',
    title: 'Your screen. A different seat.',
    description:
      'Bring one Mac’s screen onto another. See the same workspace and control it from wherever you’re sitting.',
    detail: 'SAME SCREEN. NEW PERSPECTIVE.',
  },
  {
    id: 'remote',
    label: 'Remote desktop',
    title: 'Your other Mac. Right here.',
    description:
      'View and control another Mac’s desktop from this one. Reach its apps and files without changing seats.',
    detail: 'ANOTHER MAC. WITHIN REACH.',
  },
] as const

type Mode = (typeof modes)[number]['id']

function Workspace({ side, mode }: { side: 'left' | 'right'; mode: Mode }) {
  return (
    <div className={`workspace-picture workspace-${side}`}>
      <div className="workspace-grid" />
      {mode === 'mirror' || (mode === 'remote' && side === 'right') ? (
        <div className="mirror-cursor">
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
                <span>DESKTOP</span>
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
                    <p>
                      <b>→</b> a little more space
                      <br />
                      <b>→</b> a lot less friction
                      <br />
                      <b>✓</b> keep going
                    </p>
                  </div>
                </div>
              ) : (
                <Workspace side="left" mode={mode} />
              )}
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
                      ? 'REMOTE DESKTOP'
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
                </div>
              ) : (
                <Workspace side="right" mode={mode} />
              )}
            </div>
            <div className="laptop-base">
              <div />
            </div>
          </div>
          <div className="cursor-track" key={mode}>
            <MousePointer2 className="visual-cursor" fill="currentColor" />
            <span>YOU</span>
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
