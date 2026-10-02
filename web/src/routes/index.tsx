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
  Command,
  Wifi,
  Battery,
  Grid2X2,
  MessageSquare,
  Sparkles,
} from 'lucide-react'
import {
  useEffect,
  useRef,
  useState,
  type CSSProperties,
  type RefObject,
} from 'react'
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

function ScreenMenuBar({
  app,
  purple = false,
}: {
  app: string
  purple?: boolean
}) {
  return (
    <div className={`screen-toolbar os-bar ${purple ? 'os-bar-purple' : ''}`}>
      <div className="os-menu">
        <Command />
        <strong>{app}</strong>
        <span>File</span>
        <span>Edit</span>
        <span>View</span>
      </div>
      <div className="os-status">
        <Wifi />
        <Battery />
        <span>9:41</span>
      </div>
    </div>
  )
}

function ScreenTaskbar({ remote }: { remote: boolean }) {
  return (
    <div className="screen-taskbar">
      <div className="taskbar-apps">
        <Grid2X2 />
        <span className="taskbar-active">
          {remote ? <Laptop /> : <MessageSquare />}
        </span>
        <span className="taskbar-search">Search</span>
      </div>
      <div className="taskbar-status">
        <Wifi />
        <Battery />
        <span>9:41</span>
      </div>
    </div>
  )
}

function ShareCursor({ side }: { side: 'left' | 'right' }) {
  const ref = useRef<HTMLDivElement>(null)
  useEffect(() => {
    const cursor = ref.current!
    const screen = cursor.parentElement!
    const field = screen.querySelector<HTMLElement>(
      `.share-typing-${side} .share-typing-input`,
    )!
    const peer = screen
      .closest('.computers')!
      .querySelector<HTMLElement>(
        side === 'left' ? '.laptop .screen' : '.monitor .screen',
      )!
    const peerField = peer.querySelector<HTMLElement>(
      `.share-typing-${side === 'left' ? 'right' : 'left'} .share-typing-input`,
    )!
    let active = true
    const update = () => {
      if (!active) return
      const bounds = screen.getBoundingClientRect()
      const target = field.getBoundingClientRect()
      const peerBounds = peer.getBoundingClientRect()
      const peerTarget = peerField.getBoundingClientRect()
      const leftTarget = side === 'left' ? target : peerTarget
      const rightTarget = side === 'right' ? target : peerTarget
      const leftBounds = side === 'left' ? bounds : peerBounds
      const rightBounds = side === 'right' ? bounds : peerBounds
      const leftScreen = side === 'left' ? screen : peer
      const rightScreen = side === 'right' ? screen : peer
      const leftDistance =
        leftBounds.left +
        leftScreen.clientLeft +
        (leftBounds.width - leftScreen.clientLeft * 2) -
        (leftTarget.left + leftTarget.width * 0.9)
      const rightDistance =
        rightTarget.left +
        rightTarget.width * 0.35 -
        (rightBounds.left + rightScreen.clientLeft)
      // Treat the display edges as adjacent, with one slope across the handoff.
      const leftY = leftTarget.top + leftTarget.height / 2
      const rightY = rightTarget.top + rightTarget.height / 2
      const crossingY =
        leftY +
        ((rightY - leftY) * leftDistance) / (leftDistance + rightDistance)
      const tip = Number.parseFloat(
        getComputedStyle(cursor).getPropertyValue('--cursor-tip'),
      )
      cursor.style.setProperty(
        '--type-x',
        `${target.left + target.width * (side === 'left' ? 0.9 : 0.35) - bounds.left - screen.clientLeft - tip}px`,
      )
      cursor.style.setProperty(
        '--type-y',
        `${target.top + target.height / 2 - bounds.top - screen.clientTop - tip}px`,
      )
      cursor.style.setProperty(
        '--cross-y',
        `${crossingY - bounds.top - screen.clientTop - tip}px`,
      )
      const fraction = leftDistance / (leftDistance + rightDistance)
      const outbound = 0.24 + 0.18 * fraction
      const inbound = 0.72 + 0.24 * (1 - fraction)
      const atField = {
        left: cursor.style.getPropertyValue('--type-x'),
        top: cursor.style.getPropertyValue('--type-y'),
      }
      const atEdge = {
        left:
          side === 'left'
            ? `${bounds.width - screen.clientLeft * 2 - tip}px`
            : `${-tip}px`,
        top: cursor.style.getPropertyValue('--cross-y'),
      }
      for (const animation of cursor.getAnimations()) {
        const name = (animation as CSSAnimation).animationName
        const effect = animation.effect as KeyframeEffect
        if (name === `share-${side}-path`) {
          effect.setKeyframes(
            side === 'left'
              ? [
                  { ...atField, offset: 0 },
                  { ...atField, offset: 0.24 },
                  { ...atEdge, offset: outbound },
                  { ...atEdge, offset: inbound },
                  { ...atField, offset: 0.96 },
                  { ...atField, offset: 1 },
                ]
              : [
                  { ...atEdge, offset: 0 },
                  { ...atEdge, offset: outbound },
                  { ...atField, offset: 0.42 },
                  { ...atField, offset: 0.72 },
                  { ...atEdge, offset: inbound },
                  { ...atEdge, offset: 1 },
                ],
          )
        } else if (name === `share-${side}-visible`) {
          const initial = side === 'left' ? 1 : 0
          effect.setKeyframes([
            { opacity: initial, offset: 0 },
            { opacity: 1 - initial, offset: outbound },
            { opacity: initial, offset: inbound },
            { opacity: initial, offset: 1 },
          ])
        }
      }
    }
    const observer = new ResizeObserver(update)
    observer.observe(screen)
    observer.observe(field)
    observer.observe(peer)
    observer.observe(peerField)
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
    history:
      side === 'left'
        ? ['connected', 'ready to type']
        : ['Any ideas?', 'Let’s make something.'],
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
            'brainstorm',
            'name this idea',
            'next steps?',
            'make it simple',
            'a new angle',
            'one more idea',
            'help me focus',
            'what comes next',
          ]
    const replies = [
      'Let’s explore.',
      'How about Orbit?',
      'Start small.',
      'Less is more.',
      'Try this…',
      'Here’s a spark.',
      'One step at a time.',
      'Keep going.',
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
    let reply = ''
    let replyCount = 0
    let nextTokenAt = Infinity
    let phrase = ''
    let input = ''
    let history =
      side === 'left'
        ? ['connected', 'ready to type']
        : ['Any ideas?', 'Let’s make something.']
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
          history =
            side === 'left' ? [...history.slice(-1), phrase] : [phrase, '…']
          submitted = true
          nextInput = ''
          if (side === 'right') {
            reply = replies[phrases.indexOf(phrase)]
            replyCount = 0
            nextTokenAt = animation.currentTime + duration * 0.08
          }
          setTerminal({ history, input: '' })
        } else if (nextInput !== input) {
          setTerminal({ history, input: nextInput })
        }
        if (side === 'right' && animation.currentTime >= nextTokenAt) {
          while (
            replyCount < reply.length &&
            animation.currentTime >= nextTokenAt
          ) {
            replyCount = Math.min(
              reply.length,
              replyCount + 1 + Math.floor(Math.random() * 4),
            )
            nextTokenAt += 45 + Math.random() * 65
          }
          history = [history[0], reply.slice(0, replyCount)]
          if (replyCount === reply.length) nextTokenAt = Infinity
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
        setTerminal({
          history:
            side === 'left'
              ? ['connected', phrases[0]]
              : [phrases[0], replies[0]],
          input: '',
        })
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
    <div
      ref={ref}
      className={`share-typing share-typing-${side} ${side === 'right' ? 'share-chat' : ''}`}
    >
      {side === 'right' ? (
        <div className="share-chat-header">
          <Sparkles /> AI chat
        </div>
      ) : null}
      <div className="share-terminal-history">
        {terminal.history.map((line, index) => (
          <div
            key={index}
            className={
              side === 'right'
                ? `chat-message ${index === 0 ? 'chat-user' : 'chat-assistant'}`
                : undefined
            }
          >
            {side === 'left' ? '› ' : ''}
            {line}
          </div>
        ))}
      </div>
      <div className="share-typing-input">
        {side === 'left' ? <span className="accent">›</span> : null}
        <span
          className={`share-typed-text ${side === 'right' && !terminal.input ? 'chat-placeholder' : ''}`}
        >
          {terminal.input || (side === 'right' ? 'Ask anything…' : '')}
        </span>
        {side === 'right' ? <ArrowUpRight className="chat-send" /> : null}
      </div>
    </div>
  )
}

function WorkspaceCursor({ mode }: { mode: 'extend' | 'mirror' | 'remote' }) {
  const ref = useRef<HTMLDivElement>(null)
  useEffect(() => {
    const cursor = ref.current!
    const window = cursor.parentElement!
    const update = () => {
      const bounds = window.getBoundingClientRect()
      const scale =
        bounds.width / Number.parseFloat(getComputedStyle(window).width)
      const tip =
        (cursor.querySelector('svg')!.getBoundingClientRect().width * 3) /
        (24 * scale)
      for (const [index, name] of ['circle', 'square', 'toggle'].entries()) {
        const shape = window
          .querySelectorAll('.workspace-art i')
          [index].getBoundingClientRect()
        cursor.style.setProperty(
          `--mirror-${name}-x`,
          `${(shape.left + shape.width / 2 - bounds.left) / scale - window.clientLeft - tip}px`,
        )
        cursor.style.setProperty(
          `--mirror-${name}-y`,
          `${(shape.top + shape.height / 2 - bounds.top) / scale - window.clientTop - tip}px`,
        )
      }
    }
    const observer = new ResizeObserver(update)
    observer.observe(window)
    observer.observe(window.querySelector('.workspace-art')!)
    update()
    return () => observer.disconnect()
  }, [mode])
  return (
    <div
      ref={ref}
      className={
        mode === 'mirror'
          ? 'extend-cursor mirror-drag-cursor'
          : mode === 'extend'
            ? 'extend-cursor'
            : 'remote-cursor'
      }
    >
      <MousePointer2 fill="currentColor" />
    </div>
  )
}

function useExtendArtwork(root: RefObject<HTMLElement | null>, mode: Mode) {
  const [artwork, setArtwork] = useState<CSSProperties[]>([
    { background: '#c2f269', transform: 'scale(1)' },
    { background: '#779d55', transform: 'rotate(15deg)' },
    { background: 'transparent', transform: 'scale(1)' },
  ])
  useEffect(() => {
    if (mode !== 'extend') return
    let frame = 0
    let animation: Animation | undefined
    let handled = 0
    const choose = (options: string[], previous: unknown) => {
      const candidates = options.filter((value) => value !== previous)
      return candidates[Math.floor(Math.random() * candidates.length)]
    }
    const tick = () => {
      animation ??= root.current
        ?.querySelector('.workspace-right .extend-cursor')
        ?.getAnimations()
        .find((item) => (item as CSSAnimation).animationName === 'extend-grab')
      if (animation && typeof animation.currentTime === 'number') {
        const duration = Number(animation.effect!.getTiming().duration)
        const cycle = Math.floor(animation.currentTime / duration)
        const progress = (animation.currentTime % duration) / duration
        const count =
          cycle * 3 +
          [0.38, 0.56, 0.74].filter((time) => progress >= time).length
        // Skip old clicks after a long background interval.
        handled = Math.max(handled, count - 6)
        while (handled < count) {
          const index = handled++ % 3
          setArtwork((previous) =>
            previous.map((shape, i) =>
              i !== index
                ? shape
                : {
                    background: choose(
                      index === 2
                        ? [
                            'transparent',
                            '#c2f269',
                            '#b78bdd',
                            '#72c6b7',
                            '#efb57c',
                          ]
                        : ['#c2f269', '#b78bdd', '#72c6b7', '#efb57c'],
                      shape.background,
                    ),
                    transform: choose(
                      index === 1
                        ? [
                            'rotate(-24deg)',
                            'rotate(-12deg)',
                            'rotate(12deg)',
                            'rotate(24deg)',
                            'rotate(38deg)',
                          ]
                        : ['scale(0.88)', 'scale(1)', 'scale(1.12)'],
                      shape.transform,
                    ),
                  },
            ),
          )
        }
      }
      frame = requestAnimationFrame(tick)
    }
    frame = requestAnimationFrame(tick)
    return () => cancelAnimationFrame(frame)
  }, [root, mode])
  return artwork
}

function RemoteViewport() {
  const ref = useRef<HTMLDivElement>(null)
  useEffect(() => {
    const viewport = ref.current!
    const source = viewport
      .closest('.computers')!
      .querySelector<HTMLElement>('.monitor .screen')!
    const desktop = viewport.firstElementChild as HTMLElement
    const viewer = viewport.parentElement!
    const local = viewer.parentElement!
    const update = () => {
      const sourceStyle = getComputedStyle(source)
      const width = Number.parseFloat(sourceStyle.width) - source.clientLeft * 2
      const height =
        Number.parseFloat(sourceStyle.height) -
        Number.parseFloat(sourceStyle.borderTopWidth) -
        Number.parseFloat(sourceStyle.borderBottomWidth)
      const bar = viewer.querySelector<HTMLElement>('.remote-viewer-bar')!
      const availableHeight = local.clientHeight * 0.81 - bar.offsetHeight - 10
      viewer.style.width = `${Math.min(local.clientWidth * 0.82, (availableHeight * width) / height)}px`
      viewport.style.aspectRatio = `${width} / ${height}`
      desktop.style.width = `${width}px`
      desktop.style.height = `${height}px`
      desktop.style.transform = `scale(${viewport.getBoundingClientRect().width / width})`
    }
    const observer = new ResizeObserver(update)
    observer.observe(source)
    observer.observe(viewport)
    observer.observe(local)
    update()
    return () => observer.disconnect()
  }, [])
  return (
    <div ref={ref} className="remote-viewport">
      <div className="remote-desktop">
        <ScreenMenuBar app="Canvas" />
        <Workspace side="left" mode="remote" />
      </div>
    </div>
  )
}

function Workspace({
  side,
  mode,
  artwork,
}: {
  side: 'left' | 'right'
  mode: Mode
  artwork?: CSSProperties[]
}) {
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
          <RemoteViewport />
        </div>
      </div>
    )
  }
  return (
    <div className={`workspace-picture workspace-${side}`}>
      <div className="workspace-grid" />
      <div className="workspace-window">
        <div className="workspace-window-bar">
          ● ● ●{' '}
          <span>
            {mode === 'remote'
              ? 'REMOTE WORKSPACE'
              : mode === 'mirror'
                ? 'THE SAME WORKSPACE'
                : 'ROOM TO MAKE'}
          </span>
        </div>
        <div className="workspace-art">
          {[0, 1, 2].map((index) => (
            <i
              key={index}
              style={mode === 'extend' ? artwork?.[index] : undefined}
            />
          ))}
        </div>
        <span className="workspace-caption">Good things take space.</span>
        {mode === 'extend' || mode === 'mirror' || mode === 'remote' ? (
          <WorkspaceCursor mode={mode} />
        ) : null}
      </div>
    </div>
  )
}

function Desk() {
  const [mode, setMode] = useState<Mode>('extend')
  const ref = useRef<HTMLElement>(null)
  const artwork = useExtendArtwork(ref, mode)
  const [paused, setPaused] = useState(false)
  const current = modes.find((item) => item.id === mode)!
  const tabs = Ariakit.useTabStore({
    selectedId: mode,
    setSelectedId: (id) => setMode(id as Mode),
  })
  return (
    <section
      ref={ref}
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
              <ScreenMenuBar app={mode === 'share' ? 'Terminal' : 'Canvas'} />
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
                <Workspace side="left" mode={mode} artwork={artwork} />
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
              {mode === 'share' || mode === 'remote' ? (
                <ScreenTaskbar remote={mode === 'remote'} />
              ) : (
                <ScreenMenuBar app="Canvas" />
              )}
              {mode === 'share' ? (
                <div className="laptop-wallpaper">
                  <div className="wallpaper-orbit" />
                  <ShareTyping side="right" />
                </div>
              ) : (
                <Workspace side="right" mode={mode} artwork={artwork} />
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
