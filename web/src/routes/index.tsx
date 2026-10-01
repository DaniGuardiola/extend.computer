import { createFileRoute, Link } from '@tanstack/react-router'
import {
  ArrowUpRight,
  ArrowRight,
  MousePointer2,
  MoveRight,
  ShieldCheck,
  Radio,
  Laptop,
  Terminal,
} from 'lucide-react'
import { useState } from 'react'
import { Brand } from '../components/Brand'

export const Route = createFileRoute('/')({ component: Landing })

function Desk() {
  const [side, setSide] = useState<'left' | 'right'>('right')
  return (
    <div className="desk-visual">
      <div className="desk-topline">
        <span>
          <i className="status-dot" /> TWO MACS. ONE FLOW.
        </span>
        <span>01 — 02</span>
      </div>
      <div className="computers" aria-hidden="true">
        <div
          className={`computer monitor ${side === 'left' ? 'active-screen' : ''}`}
        >
          <div className="screen">
            <div className="screen-toolbar">
              <span className="window-dots">● ● ●</span>
              <span>DESKTOP</span>
            </div>
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
            {side === 'left' ? (
              <MousePointer2 className="visual-cursor" fill="currentColor" />
            ) : null}
          </div>
          <div className="monitor-chin">
            <div />
          </div>
          <div className="monitor-stand" />
        </div>
        <div
          className={`computer laptop ${side === 'right' ? 'active-screen' : ''}`}
        >
          <div className="screen">
            <div className="screen-toolbar">
              <span className="window-dots">● ● ●</span>
              <span>LAPTOP</span>
            </div>
            <div className="laptop-wallpaper">
              <span>
                Keep
                <br />
                your flow<span className="accent">.</span>
              </span>
              <div className="wallpaper-orbit" />
            </div>
            {side === 'right' ? (
              <MousePointer2 className="visual-cursor" fill="currentColor" />
            ) : null}
          </div>
          <div className="laptop-base">
            <div />
          </div>
        </div>
      </div>
      <div className="desk-bottomline">
        <span>YOUR KEYBOARD + MOUSE</span>
        <button
          onClick={() => setSide(side === 'left' ? 'right' : 'left')}
          className="demo-control"
          aria-label={`Move demo cursor to ${side === 'left' ? 'laptop' : 'desktop'}`}
        >
          Try the handoff <MoveRight size={17} />
        </button>
      </div>
    </div>
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
              Your keyboard and mouse,
              <br className="desktop-break" /> across all your Macs. Keep going.
            </p>
            <div className="hero-actions">
              <Link to="/signup" className="button primary">
                Get started <ArrowUpRight size={20} />
              </Link>
              <a
                href="https://github.com/DaniGuardiola/extend.computer"
                className="source-link"
                target="_blank"
                rel="noreferrer"
              >
                View source <ArrowUpRight size={16} />
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
            Move your cursor to the edge of one screen.
            <br className="desktop-break" /> Pick up right where you left off on
            the next.
            <br className="desktop-break" /> Your keyboard follows. That’s the
            whole idea.
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
              Create your account and manage your device list.
              <br />
              Desktop account sign-in is coming next.
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
        <a
          href="https://github.com/DaniGuardiola/extend.computer"
          target="_blank"
          rel="noreferrer"
        >
          Built in the open <ArrowUpRight size={15} />
        </a>
      </footer>
    </div>
  )
}
