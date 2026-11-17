import { renderToStaticMarkup } from 'react-dom/server'
import { expect, it } from 'vitest'

import type { SessionUsage } from '#agent-usage'
import { SessionUsageRow } from '#components/session-usage-row'

it('announces current quota usage when the forecast is not available', () => {
  const nowSeconds = 1_800_000_000
  const usage: SessionUsage = {
    sessionId: 'sample-session',
    engine: 'codex',
    model: null,
    effort: null,
    context: null,
    rateLimits: {
      fiveHour: {
        usedPercentage: 3,
        resetsAt: nowSeconds + 17_136,
        windowSeconds: 18_000,
      },
      sevenDay: null,
    },
    version: null,
    observedAt: '2030-01-15T12:00:00Z',
  }

  expect(
    renderToStaticMarkup(
      <SessionUsageRow nowSeconds={nowSeconds} usage={usage} />,
    ),
  ).toEqual(
    '<div aria-label="Session usage" class="session-usage-row" data-mobile="false" role="group"><span class="session-usage-engine" title="Codex">codex</span><span aria-hidden="true" class="session-usage-separator">|</span><span aria-label="5h: 3% used" class="session-usage-rate" role="group" title="5h: 3% used"><span aria-hidden="true" class="session-usage-rate-current">5h 3%</span></span></div>',
  )
})
