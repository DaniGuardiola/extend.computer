import { createFileRoute, Link } from '@tanstack/react-router'
import { useState } from 'react'
import {
  ArrowUpRight,
  ArrowDown,
  Download,
  FlaskConical,
  Check,
  CircleDashed,
  ChevronDown,
} from 'lucide-react'
import { Brand } from '../components/Brand'
import data from '../lib/benchmarks-data.json'
import { latencyIndices, median } from '../lib/benchmark-scoring'
import styles from '../benchmarks.css?url'

export const Route = createFileRoute('/benchmarks')({
  head: () => ({
    meta: [
      { title: 'Benchmarks — extend.computer' },
      {
        name: 'description',
        content:
          'Measured desktop display performance, methodology, test context and comparison limits.',
      },
    ],
    links: [{ rel: 'stylesheet', href: styles }],
  }),
  component: Benchmarks,
})

const products = data.products
const number = (value: number | null, digits = 1) =>
  value === null ? '—' : value.toFixed(digits)
const productName = (id: string) =>
  products.find((p) => p.id === id)?.name ?? id
const scenes = ['all', 'motion', 'scroll'] as const
const gaps = [
  [
    'Cursor response',
    'Matched 10-second cursor-transition and continuous-motion checks passed. Repeated trials remain.',
  ],
  [
    'Input-to-photon',
    'Synthetic input is covered. Physical hardware input is not yet measured.',
  ],
  [
    'Recovery & reliability',
    'Both products resumed after a 2-second sender pause. Network loss, sleep/wake and long sessions remain.',
  ],
  [
    'Energy & efficiency',
    'Process telemetry exists; matched energy evidence incomplete.',
  ],
  [
    'Settings coverage',
    'Resolution, frame-rate and quality presets need a complete matched matrix.',
  ],
  [
    'Window & display behavior',
    'Fullscreen, scaling and interaction outcomes tested in part.',
  ],
]

function Benchmarks() {
  const [scene, setScene] = useState<(typeof scenes)[number]>('all')
  const rows = data.latency.filter((r) => scene === 'all' || r.scene === scene)
  const indices = latencyIndices(
    rows,
    products.map((p) => p.id),
  )
  const aggregate = products.map((p) => {
    const runs = rows.filter((r) => r.product === p.id)
    return {
      ...p,
      median: median(runs.map((r) => r.median)),
      p95: median(runs.map((r) => r.p95)),
      samples: runs.reduce((sum, r) => sum + r.samples, 0),
      runs: runs.length,
    }
  })
  const download = () => {
    const url = URL.createObjectURL(
      new Blob(
        [
          JSON.stringify(
            {
              ...data,
              scoring: {
                version: 1,
                selectedScene: scene,
                latencyIndex: indices,
                overallScore: null,
              },
            },
            null,
            2,
          ),
        ],
        { type: 'application/json' },
      ),
    )
    const link = document.createElement('a')
    link.href = url
    link.download = `display-benchmarks-${data.date}.json`
    link.click()
    URL.revokeObjectURL(url)
  }
  return (
    <div className="benchmark-page">
      <header className="site-header">
        <Brand />
        <nav aria-label="Main navigation">
          <Link to="/">Overview</Link>
          <Link to="/login">
            Sign in <ArrowUpRight size={16} />
          </Link>
        </nav>
      </header>
      <main id="main" className="bench-shell">
        <section className="bench-hero">
          <div className="bench-eyebrow">
            <FlaskConical size={15} /> THE DISPLAY LAB{' '}
            <span>Snapshot · {data.date}</span>
          </div>
          <h1>
            Less guesswork.
            <br />
            <span>More measurement.</span>
          </h1>
          <div className="bench-intro">
            <p>
              A good extra display should feel like part of your desk. Here’s
              what we’ve measured, how we measured it, and what still needs
              work.
            </p>
            <button className="bench-download" onClick={download}>
              <Download size={16} /> Download data
            </button>
          </div>
          <nav className="bench-anchor-nav" aria-label="Benchmark sections">
            <a href="#results">
              Results <ArrowDown size={13} />
            </a>
            <a href="#quality">Image quality</a>
            <a href="#coverage">Coverage</a>
            <a href="#method">Method & scoring</a>
          </nav>
        </section>
        <div className="bench-context-strip">
          <span>
            <i /> Direct LAN · Wi-Fi
          </span>
          <span>2× Retina · 1728 × 1085</span>
          <span>Same two Macs</span>
          <span>3 repeats per scene</span>
        </div>
        <section id="results" className="bench-section">
          <div className="bench-section-head">
            <div>
              <p className="bench-eyebrow">01 / RESPONSIVENESS</p>
              <h2>From one screen to the other.</h2>
              <p>Physical screen-transition delay. Lower is better.</p>
            </div>
            <div
              className="bench-segment"
              role="group"
              aria-label="Filter scenes"
            >
              {scenes.map((s) => (
                <button
                  key={s}
                  aria-pressed={scene === s}
                  onClick={() => setScene(s)}
                >
                  {s === 'all'
                    ? 'All scenes'
                    : s === 'motion'
                      ? 'Motion'
                      : 'Scroll'}
                </button>
              ))}
            </div>
          </div>
          <div className="bench-product-grid">
            {aggregate.map((p) => (
              <article key={p.id} className={`bench-product ${p.id}`}>
                <div className="bench-product-top">
                  <h3>{p.name}</h3>
                  <span className="bench-tag">
                    {p.id === 'extend' ? 'Development build' : 'Reference app'}
                  </span>
                </div>
                <p className="bench-build">{p.build}</p>
                <div className="bench-big-number">
                  {number(p.median)}
                  <span>ms</span>
                </div>
                <p className="bench-number-label">Median of run medians</p>
                <div className="bench-product-foot">
                  <div>
                    <strong>{number(p.p95)} ms</strong>
                    <span>Median of run p95s</span>
                  </div>
                  <div>
                    <strong>{p.samples.toLocaleString()}</strong>
                    <span>Readable transitions · {p.runs} runs</span>
                  </div>
                </div>
              </article>
            ))}
          </div>
          <div className="bench-comparison">
            <div className="bench-chart-header">
              <h3>Typical delay & slower moments</h3>
              <span>
                Median <i className="bench-legend-solid" /> p95{' '}
                <i className="bench-legend-tail" />
              </span>
            </div>
            <div className="bench-axis">
              <span>0</span>
              <span>25</span>
              <span>50</span>
              <span>75</span>
              <span>100 ms</span>
            </div>
            {aggregate.map((p) => (
              <div className={`bench-bar-row ${p.id}`} key={p.id}>
                <span>{p.name}</span>
                <div className="bench-bar-track">
                  <div
                    className="bench-bar-tail"
                    style={{ width: `${p.p95 ?? 0}%` }}
                  />
                  <div
                    className="bench-bar"
                    style={{ width: `${p.median ?? 0}%` }}
                  />
                  <span
                    className="bench-bar-value"
                    style={{ left: `min(${(p.p95 ?? 0) + 2}%, 90%)` }}
                  >
                    {number(p.median)} / {number(p.p95)}
                  </span>
                </div>
              </div>
            ))}
            <p className="bench-note">
              p95 marks the slower end: 95% of readable transitions in a run
              were at or below that delay. Bars use medians across runs, not
              pooled events.
            </p>
          </div>
          <details className="bench-details">
            <summary>
              Every run, without averaging away the variation{' '}
              <ChevronDown size={16} />
            </summary>
            <div className="bench-table-scroll">
              <table>
                <caption className="sr-only">
                  Latency and visible updates for each selected diagnostic run
                </caption>
                <thead>
                  <tr>
                    <th>Product</th>
                    <th>Scene / repeat</th>
                    <th>Median</th>
                    <th>p95</th>
                    <th>Transitions</th>
                    <th>Visible updates ≥</th>
                  </tr>
                </thead>
                <tbody>
                  {rows.map((r) => (
                    <tr key={`${r.product}-${r.scene}-${r.repeat}`}>
                      <td>{productName(r.product)}</td>
                      <td>
                        {r.scene} / {r.repeat}
                      </td>
                      <td>{number(r.median)} ms</td>
                      <td>{number(r.p95)} ms</td>
                      <td>{r.samples}</td>
                      <td>{number(r.visibleFpsLowerBound)} / s</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </details>
          <div className="bench-score-panel">
            <div>
              <p className="bench-eyebrow">CATEGORY INDEX / PROVISIONAL</p>
              <h3>Responsiveness, on a common scale.</h3>
              <p>
                Best per measurement = 100; blended indices can be lower for
                every product. Equal weight for typical delay, p95, and every
                matched scene/repeat. This covers screen-transition latency
                only.
              </p>
            </div>
            <div className="bench-index-grid">
              {products.map((p) => (
                <div className={p.id} key={p.id}>
                  <span>{p.name}</span>
                  <strong>
                    {number(indices?.[p.id] ?? null, 0)}
                    <small>/100</small>
                  </strong>
                </div>
              ))}
            </div>
          </div>
          <p className="bench-caveat">
            <CircleDashed size={17} /> Both apps requested 60 FPS. Reference
            settings were observed before and after capture; its bitrate remains
            unavailable. This index covers latency only, not an overall ranking.
          </p>
        </section>
        <section id="quality" className="bench-section">
          <div className="bench-section-head">
            <div>
              <p className="bench-eyebrow">02 / IMAGE & INTERACTION</p>
              <h2>Clarity is more than a frame rate.</h2>
              <p>
                Calibrated synthetic compositor captures. Three samples per
                region.
              </p>
            </div>
          </div>
          <div className="bench-quality-intro">
            <div>
              <h3>Text is close. Color is mixed.</h3>
              <p>
                Both preserve text structure well in this fixture. Colored edges
                vary by region. Near-perfect structural scores do not mean
                identical pixels or equal physical display quality.
              </p>
            </div>
            <div className="bench-small-stat">
              <span>Text structural similarity</span>
              <strong>0.9995–1.0000</strong>
              <span>Both products · SSIM, higher is better</span>
            </div>
          </div>
          <div className="bench-table-scroll">
            <table>
              <thead>
                <tr>
                  <th>Test region</th>
                  <th>extend · SSIM</th>
                  <th>Duet · SSIM</th>
                  <th>extend · PSNR</th>
                  <th>Duet · PSNR</th>
                </tr>
              </thead>
              <tbody>
                {data.quality.map((q) => (
                  <tr key={q.region}>
                    <td>{q.region.replaceAll('_', ' ')}</td>
                    <td>{number(q.extend.luma_ssim_box11, 5)}</td>
                    <td>{number(q.duet.luma_ssim_box11, 5)}</td>
                    <td>{number(q.extend.rgb_psnr_db)} dB</td>
                    <td>{number(q.duet.rgb_psnr_db)} dB</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          <p className="bench-note">
            SSIM compares luminance structure. PSNR compares RGB pixel error;
            higher is better. Text scores cluster near 1; colored-edge errors
            vary substantially. These metrics alone do not establish a
            perceptual winner.
          </p>
          <div className="bench-interaction-grid">
            {data.interactions.map((p) => (
              <article key={p.product}>
                <h3>{productName(p.product)}</h3>
                <strong>
                  {p.regions.filter((r) => r.accepted).length}/
                  {p.regions.length}
                  <span> final visual regions passed</span>
                </strong>
                <div>
                  {p.regions.map((r) => (
                    <span key={r.region}>
                      <Check size={13} />
                      {r.region}
                    </span>
                  ))}
                </div>
              </article>
            ))}
          </div>
          <p className="bench-note">
            Earlier accepted checks verify final button, drag, text replacement
            and scroll outcomes. They have not been repeated for these latest
            builds, and do not measure input-to-photon or transition smoothness.
          </p>
        </section>
        <section id="coverage" className="bench-section">
          <div className="bench-section-head">
            <div>
              <p className="bench-eyebrow">03 / THE WHOLE EXPERIENCE</p>
              <h2>No overall winner. Yet.</h2>
              <p>A fast scene isn’t enough to rate a whole product.</p>
            </div>
            <span className="bench-tag">Overall score withheld</span>
          </div>
          <div className="bench-gap-grid">
            {gaps.map(([title, description]) => (
              <article key={title}>
                <CircleDashed size={18} />
                <div>
                  <h3>{title}</h3>
                  <p>{description}</p>
                </div>
              </article>
            ))}
          </div>
          <p className="bench-note">
            Missing measurements stay missing. They never become zero, a pass,
            or a reweighted score that hides incomplete coverage.
          </p>
        </section>
        <section id="method" className="bench-section">
          <div className="bench-section-head">
            <div>
              <p className="bench-eyebrow">04 / REPRODUCIBILITY</p>
              <h2>The conditions matter.</h2>
              <p>Results belong to this setup and these builds.</p>
            </div>
          </div>
          <dl className="bench-context-grid">
            {Object.entries(data.context)
              .filter(([key]) => key !== 'workloadSha256')
              .map(([key, value]) => (
                <div key={key}>
                  <dt>{key.replace(/([A-Z])/g, ' $1')}</dt>
                  <dd>{value}</dd>
                </div>
              ))}
          </dl>
          <div className="bench-method-grid">
            <article>
              <h3>One camera. One clock.</h3>
              <p>
                A generated counter appears on both screens. We compare its
                first readable appearance in the same high-speed video. That
                includes capture, encoding, transport, rendering, and both
                physical displays.
              </p>
              <p>
                Camera sampling and rolling shutter introduce uncertainty.
                Unreadable transitions are excluded; coverage stays part of the
                evidence.
              </p>
            </article>
            <article>
              <h3>Scoring that earns its number.</h3>
              <p>
                The latency index is the geometric mean of best/observed ratios
                for median and p95 in each matched scene/repeat, multiplied by
                100. Adding products can change this relative index.
              </p>
              <p>
                Future overall scores require every category and comparison gate
                to pass. Versioned weights will be declared before measurement;
                no overall score is calculated for this snapshot.
              </p>
            </article>
          </div>
          <details className="bench-details">
            <summary>
              Measurement limits & provenance <ChevronDown size={16} />
            </summary>
            <ul className="bench-limits">
              {data.limits.map((limit) => (
                <li key={limit}>{limit}</li>
              ))}
            </ul>
            <div className="bench-provenance">
              {data.evidence.map((e) => (
                <div key={e.id}>
                  <span>{e.id} · report SHA-256</span>
                  <code>{e.sha256}</code>
                </div>
              ))}
              <div>
                <span>Shared workload SHA-256</span>
                <code>{data.context.workloadSha256}</code>
              </div>
            </div>
          </details>
        </section>
        <footer className="bench-footer">
          <Brand />
          <span>Measured openly. Improved continuously.</span>
          <a href="#main">Back to top ↑</a>
        </footer>
      </main>
    </div>
  )
}
