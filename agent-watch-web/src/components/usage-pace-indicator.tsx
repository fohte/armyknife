import '#components/usage-pace-indicator.css'

import { getUsageLandingEstimate, type RateLimitWindow } from '#agent-usage'

const statusLabels = {
  'on-pace': 'On pace',
  warning: 'Approaching limit',
  danger: 'Likely to exceed limit',
} as const

const statusMarks = {
  'on-pace': '✓',
  warning: '⚠',
  danger: '🔥',
} as const

type UsagePaceIndicatorProps = {
  window: RateLimitWindow | null
  nowSeconds: number
}

export function UsagePaceIndicator({
  window,
  nowSeconds,
}: UsagePaceIndicatorProps) {
  if (window === null) return null
  const estimate = getUsageLandingEstimate(window, nowSeconds)
  if (estimate === null) return null
  const label = statusLabels[estimate.status]
  const description = `${label}: projected ${String(estimate.percentage)}% at window end`
  return (
    <span
      aria-label={description}
      className={`usage-pace-indicator is-${estimate.status}`}
      title={description}
    >
      {statusMarks[estimate.status]} {String(estimate.percentage)}%
    </span>
  )
}
