export type LatencyRun = {
  product: string
  scene: string
  repeat: number
  median: number
  p95: number
}

export function median(values: number[]) {
  if (!values.length) return null
  const sorted = [...values].sort((a, b) => a - b)
  const middle = Math.floor(sorted.length / 2)
  return sorted.length % 2
    ? sorted[middle]!
    : (sorted[middle - 1]! + sorted[middle]!) / 2
}

// Each matched scene/repeat and each latency statistic has equal weight.
// This is a relative latency index, never an overall product score.
export function latencyIndices(runs: LatencyRun[], products: string[]) {
  if (products.length < 2 || new Set(products).size !== products.length)
    return null
  const keys = [...new Set(runs.map((r) => `${r.scene}:${r.repeat}`))]
  if (!keys.length) return null
  const ratios: Record<string, number[]> = Object.fromEntries(
    products.map((p) => [p, []]),
  )
  for (const key of keys) {
    const matched = products.map((product) =>
      runs.filter(
        (r) => r.product === product && `${r.scene}:${r.repeat}` === key,
      ),
    )
    if (matched.some((rows) => rows.length !== 1)) return null
    const rows = matched.map((rows) => rows[0]!)
    for (const metric of ['median', 'p95'] as const) {
      if (rows.some((r) => !Number.isFinite(r[metric]) || r[metric] <= 0))
        return null
      const best = Math.min(...rows.map((r) => r[metric]))
      rows.forEach((r) => ratios[r.product]!.push(best / r[metric]))
    }
  }
  return Object.fromEntries(
    products.map((p) => [
      p,
      100 *
        Math.exp(
          ratios[p]!.reduce((sum, r) => sum + Math.log(r), 0) /
            ratios[p]!.length,
        ),
    ]),
  )
}

export function completeScore(
  categories: { score: number | null; weight: number }[],
) {
  if (
    !categories.length ||
    categories.some(
      (c) =>
        c.score === null ||
        !Number.isFinite(c.score) ||
        c.score < 0 ||
        c.score > 100 ||
        !Number.isFinite(c.weight) ||
        c.weight <= 0,
    )
  )
    return null
  const total = categories.reduce((sum, c) => sum + c.weight, 0)
  return categories.reduce((sum, c) => sum + (c.score! * c.weight) / total, 0)
}
