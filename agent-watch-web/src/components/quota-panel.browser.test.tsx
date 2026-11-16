import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import type { EngineQuota } from '#agent-usage'
import { QuotaPanel } from '#components/quota-panel'

let container: HTMLDivElement
let root: Root | null

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  container = document.createElement('div')
  document.body.append(container)
  root = createRoot(container)
})

afterEach(() => {
  if (root !== null) act(() => root?.unmount())
  root = null
  container.remove()
  vi.unstubAllGlobals()
})

function visibleQuotaState(element: HTMLElement) {
  const windows = element.querySelectorAll('.quota-window')
  const expiredWindow = windows.item(0)
  const currentWindow = windows.item(1)
  return [
    expiredWindow.querySelector('.quota-window-percentage')?.textContent ??
      null,
    expiredWindow
      .querySelector('[role="meter"]')
      ?.getAttribute('aria-valuetext') ?? null,
    expiredWindow.querySelector('.quota-window-reset')?.textContent ?? null,
    expiredWindow.querySelector('.quota-meter-fill') === null,
    expiredWindow.querySelector('.quota-meter-pace') === null,
    currentWindow.querySelector('.quota-window-percentage')?.textContent ??
      null,
    currentWindow
      .querySelector('[role="meter"]')
      ?.getAttribute('aria-valuetext') ?? null,
  ]
}

it('hides an expired window while preserving the current window', () => {
  const nowSeconds = 1_800_000_000
  const claudeQuota: EngineQuota = {
    engine: 'claude',
    rateLimits: {
      fiveHour: {
        usedPercentage: 99,
        resetsAt: nowSeconds - 1,
        windowSeconds: 18_000,
      },
      sevenDay: {
        usedPercentage: 52,
        resetsAt: nowSeconds + 36_000,
        windowSeconds: 604_800,
      },
    },
    observedAt: '2030-01-15T12:00:00Z',
  }

  act(() => {
    root?.render(
      <QuotaPanel
        nowSeconds={nowSeconds}
        quotas={{ claude: claudeQuota, codex: null }}
      />,
    )
  })

  expect(visibleQuotaState(container)).toEqual([
    '--',
    'Unavailable',
    ' --',
    true,
    true,
    '52%',
    '52% used',
  ])
})
