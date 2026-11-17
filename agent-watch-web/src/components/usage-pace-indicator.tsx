import '#components/usage-pace-indicator.css'

import {
  getUsageLandingDescription,
  getUsageLandingEstimate,
  type RateLimitWindow,
} from '#agent-usage'

type UsagePaceIndicatorProps = {
  label: string
  usedPercentage: number
  window: RateLimitWindow | null
  nowSeconds: number
  decorative?: boolean
}

export function UsagePaceIndicator({
  label,
  usedPercentage,
  window,
  nowSeconds,
  decorative = false,
}: UsagePaceIndicatorProps) {
  if (window === null) return null
  const estimate = getUsageLandingEstimate(window, nowSeconds)
  if (estimate === null) return null
  const description = getUsageLandingDescription(
    label,
    usedPercentage,
    window,
    nowSeconds,
  )
  return (
    <span
      aria-hidden={decorative || undefined}
      aria-label={decorative ? undefined : (description ?? undefined)}
      className={`usage-pace-indicator is-${estimate.status}`}
      title={description ?? undefined}
    >
      {'→ '}
      {String(estimate.percentage)}%
    </span>
  )
}
