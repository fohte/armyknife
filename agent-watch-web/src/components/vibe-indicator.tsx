import '#components/vibe-indicator.css'

import { Zap } from 'lucide-react'

import type { VibeMode } from '#types'
import { vibeModeIsOn, vibeModeLabel } from '#vibe-mode'

type VibeIndicatorProps = {
  mode: VibeMode
  size?: number
}

export function VibeIndicator({ mode, size = 12 }: VibeIndicatorProps) {
  if (!vibeModeIsOn(mode)) return null

  const inherited = mode.mode === 'inherited'
  return (
    <span
      className={`vibe-indicator${inherited ? ' is-inherited' : ''}`}
      role="img"
      aria-label={vibeModeLabel(mode)}
      title={vibeModeLabel(mode)}
    >
      <Zap
        aria-hidden="true"
        size={size}
        fill={inherited ? 'none' : 'currentColor'}
        strokeWidth={inherited ? 1.6 : 2.2}
      />
    </span>
  )
}
