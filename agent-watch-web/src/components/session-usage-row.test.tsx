import { renderToStaticMarkup } from 'react-dom/server'
import { expect, it } from 'vitest'

import type { SessionUsage } from '#agent-usage'
import { SessionUsageRow } from '#components/session-usage-row'

it.each([
  { severity: 'normal', usedPercentage: 3 },
  { severity: 'warning', usedPercentage: 82 },
  { severity: 'danger', usedPercentage: 97 },
])(
  'announces $severity quota usage when a forecast is unavailable',
  ({ severity, usedPercentage }) => {
    const nowSeconds = 1_800_000_000
    const usage: SessionUsage = {
      sessionId: 'sample-session',
      engine: 'codex',
      model: null,
      effort: null,
      context: null,
      rateLimits: {
        fiveHour: {
          usedPercentage,
          resetsAt: null,
          windowSeconds: 18_000,
        },
        sevenDay: null,
      },
      version: null,
      observedAt: '2030-01-15T12:00:00Z',
    }

    const expectedMarkup = `<div aria-label="Session usage" class="session-usage-row" data-mobile="false" role="group"><span class="session-usage-engine" title="Codex">codex</span><span aria-hidden="true" class="session-usage-separator">|</span><span aria-label="5h: ${String(usedPercentage)}% used" class="session-usage-rate session-usage-rate-five-hour" role="group" title="5h: ${String(usedPercentage)}% used"><span aria-hidden="true" class="session-usage-rate-current is-${severity}"><span class="session-usage-rate-label">5h</span><span class="session-usage-rate-percentage">${String(usedPercentage)}%</span></span></span></div>`

    expect(
      renderToStaticMarkup(
        <SessionUsageRow nowSeconds={nowSeconds} usage={usage} />,
      ),
    ).toEqual(expectedMarkup)
  },
)
