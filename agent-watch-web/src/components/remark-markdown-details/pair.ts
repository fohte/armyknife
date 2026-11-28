type PairUnit = { marker: { kind: string } } | { node: unknown }
type Pair = { close: number; summaryOpen: number; summaryClose: number }

export function pairDetails(units: readonly PairUnit[]) {
  type Frame = {
    open: number
    render: boolean
    summaryOpen?: number
    summaryClose?: number
  }
  const pairs = new Map<number, Pair>()
  const closes = new Map<number, number>()
  const stack: Frame[] = []
  const pending = new Set<Frame>()
  const invalidatePending = () => {
    pending.clear()
  }

  for (const [index, unit] of units.entries()) {
    if (!('marker' in unit)) {
      invalidatePending()
      continue
    }
    const { kind } = unit.marker
    if (kind === 'details-open') {
      invalidatePending()
      const frame = { open: index, render: true }
      stack.push(frame)
      pending.add(frame)
    } else if (kind === 'details-raw-open') {
      invalidatePending()
      stack.push({ open: index, render: false })
    } else if (kind === 'summary-open') {
      const frame = stack.at(-1)
      if (frame?.render === true && pending.delete(frame))
        frame.summaryOpen = index
      else invalidatePending()
    } else if (kind === 'summary-close') {
      const frame = stack.at(-1)
      if (
        frame?.render === true &&
        frame.summaryOpen !== undefined &&
        frame.summaryClose === undefined
      ) {
        frame.summaryClose = index
      } else invalidatePending()
    } else if (kind === 'details-close') {
      const frame = stack.pop()
      if (frame === undefined) continue
      pending.delete(frame)
      closes.set(frame.open, index)
      if (
        frame.render &&
        frame.summaryOpen !== undefined &&
        frame.summaryClose !== undefined
      ) {
        pairs.set(frame.open, {
          close: index,
          summaryOpen: frame.summaryOpen,
          summaryClose: frame.summaryClose,
        })
      }
    }
  }
  return { pairs, closes }
}
