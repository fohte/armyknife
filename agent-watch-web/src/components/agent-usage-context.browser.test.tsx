import { act, useEffect } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, expect, it, vi } from 'vitest'

import type { AgentUsage } from '#agent-usage'
import {
  AgentUsageProvider,
  useAgentUsageContext,
} from '#components/agent-usage-context'

type UsageEvent =
  { type: 'usage'; value: AgentUsage } | { type: 'error' } | null

function UsagePublisher({ event }: { event: UsageEvent }) {
  const { onUsage, onUsageError } = useAgentUsageContext()

  useEffect(() => {
    if (event?.type === 'usage') onUsage(event.value)
    if (event?.type === 'error') onUsageError()
  }, [event, onUsage, onUsageError])

  return null
}

function UsageSnapshot() {
  const { error, usage } = useAgentUsageContext()
  return (
    <output data-testid="usage-state">
      {JSON.stringify({ usage, error })}
    </output>
  )
}

function usageResponse(model: string): AgentUsage {
  return {
    sessions: [
      {
        sessionId: 'sample-session',
        engine: 'claude',
        model,
        effort: 'medium',
        context: null,
        rateLimits: { fiveHour: null, sevenDay: null },
        version: null,
        observedAt: '2026-11-16T12:00:00Z',
      },
    ],
    quotas: {
      claude: null,
      codex: null,
    },
  }
}

let container: HTMLDivElement | null = null
let root: Root | null = null

afterEach(() => {
  if (root !== null) act(() => root?.unmount())
  root = null
  container?.remove()
  container = null
  vi.unstubAllGlobals()
  vi.restoreAllMocks()
})

it('retains the latest usage and clears its error after a new usage event', async () => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  container = document.createElement('div')
  document.body.append(container)
  root = createRoot(container)
  const firstUsage = usageResponse('fixture-model-one')
  const nextUsage = usageResponse('fixture-model-two')

  const render = async (event: UsageEvent) => {
    await act(async () => {
      root?.render(
        <AgentUsageProvider>
          <UsagePublisher event={event} />
          <UsageSnapshot />
        </AgentUsageProvider>,
      )
      await Promise.resolve()
    })
    const state: unknown = JSON.parse(
      container?.querySelector('[data-testid="usage-state"]')?.textContent ??
        'null',
    )
    return state
  }

  const states = Object.fromEntries([
    ['initial', await render(null)],
    ['firstUsage', await render({ type: 'usage', value: firstUsage })],
    ['staleAfterError', await render({ type: 'error' })],
    ['recovered', await render({ type: 'usage', value: nextUsage })],
  ])

  expect(states).toEqual({
    initial: { usage: null, error: false },
    firstUsage: { usage: firstUsage, error: false },
    staleAfterError: { usage: firstUsage, error: true },
    recovered: { usage: nextUsage, error: false },
  })
})
