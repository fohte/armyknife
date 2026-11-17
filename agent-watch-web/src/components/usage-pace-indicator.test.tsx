import { renderToStaticMarkup } from 'react-dom/server'
import { expect, it } from 'vitest'

import { UsagePaceIndicator } from '#components/usage-pace-indicator'

const nowSeconds = 1_800_000_000

it('omits the projection before five percent of the window has elapsed', () => {
  expect(
    renderToStaticMarkup(
      <UsagePaceIndicator
        label="7d"
        nowSeconds={nowSeconds}
        usedPercentage={3}
        window={{
          usedPercentage: 3,
          resetsAt: nowSeconds + 17_136,
          windowSeconds: 18_000,
        }}
      />,
    ),
  ).toEqual('')
})

it('shows the forecast with its accessible pace and reset description', () => {
  expect(
    renderToStaticMarkup(
      <UsagePaceIndicator
        label="7d"
        nowSeconds={nowSeconds}
        usedPercentage={62}
        window={{
          usedPercentage: 62,
          resetsAt: nowSeconds + 9_000,
          windowSeconds: 18_000,
        }}
      />,
    ),
  ).toEqual(
    '<span aria-label="7d: 62% used, projected 124% at window end, at or above quota pace, resets in 2h30m" class="usage-pace-indicator is-warning" title="7d: 62% used, projected 124% at window end, at or above quota pace, resets in 2h30m">→ 124%</span>',
  )
})

it('describes a forecast at or above 150 percent as well above quota pace', () => {
  expect(
    renderToStaticMarkup(
      <UsagePaceIndicator
        label="7d"
        nowSeconds={nowSeconds}
        usedPercentage={80}
        window={{
          usedPercentage: 80,
          resetsAt: nowSeconds + 9_000,
          windowSeconds: 18_000,
        }}
      />,
    ),
  ).toEqual(
    '<span aria-label="7d: 80% used, projected 160% at window end, well above quota pace, resets in 2h30m" class="usage-pace-indicator is-danger" title="7d: 80% used, projected 160% at window end, well above quota pace, resets in 2h30m">→ 160%</span>',
  )
})
