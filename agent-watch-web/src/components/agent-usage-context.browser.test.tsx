import { err, ok } from 'neverthrow'
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import type { AgentUsage } from '#agent-usage'
import {
  AgentUsageProvider,
  useAgentUsageContext,
} from '#components/agent-usage-context'
import { QuotaPanel } from '#components/quota-panel'

const { requestAgentUsageMock } = vi.hoisted(() => ({
  requestAgentUsageMock: vi.fn(),
}))

vi.mock('#api-requests', () => ({
  requestJson: requestAgentUsageMock,
}))

let container: HTMLDivElement
let root: Root | null
const visibilityStateDescriptor = Object.getOwnPropertyDescriptor(
  document,
  'visibilityState',
)

function setVisibilityState(visibilityState: DocumentVisibilityState) {
  Object.defineProperty(document, 'visibilityState', {
    configurable: true,
    value: visibilityState,
  })
  document.dispatchEvent(new Event('visibilitychange'))
}

function UsageSnapshot() {
  const { error, usage } = useAgentUsageContext()
  return (
    <>
      <span data-error={String(error)} data-session-model="">
        {usage?.sessions[0]?.model ?? 'loading'}
      </span>
      <QuotaPanel
        error={error}
        nowSeconds={1_794_832_000}
        quotas={usage?.quotas ?? null}
      />
    </>
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
      claude: {
        engine: 'claude',
        rateLimits: {
          fiveHour: {
            usedPercentage: 22,
            resetsAt: 1_794_841_200,
            windowSeconds: 18_000,
          },
          sevenDay: null,
        },
        observedAt: '2026-11-16T12:00:00Z',
      },
      codex: null,
    },
  }
}

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  vi.useFakeTimers()
  requestAgentUsageMock.mockReset()
  container = document.createElement('div')
  document.body.append(container)
  root = createRoot(container)
})

afterEach(() => {
  if (root !== null) act(() => root?.unmount())
  root = null
  container.remove()
  vi.useRealTimers()
  if (visibilityStateDescriptor === undefined) {
    Reflect.deleteProperty(document, 'visibilityState')
  } else {
    Object.defineProperty(
      document,
      'visibilityState',
      visibilityStateDescriptor,
    )
  }
  vi.unstubAllGlobals()
  vi.restoreAllMocks()
})

it('pauses usage refresh while hidden and refreshes when visible again', async () => {
  requestAgentUsageMock.mockReturnValue(ok(usageResponse('fixture-model')))
  setVisibilityState('hidden')

  await act(async () => {
    root?.render(
      <AgentUsageProvider>
        <UsageSnapshot />
      </AgentUsageProvider>,
    )
    await Promise.resolve()
  })
  const requestCounts = [requestAgentUsageMock.mock.calls.length]

  await act(async () => {
    setVisibilityState('visible')
    await Promise.resolve()
  })
  requestCounts.push(requestAgentUsageMock.mock.calls.length)

  await act(async () => {
    setVisibilityState('hidden')
    await vi.advanceTimersByTimeAsync(120_000)
  })
  requestCounts.push(requestAgentUsageMock.mock.calls.length)

  await act(async () => {
    setVisibilityState('visible')
    await Promise.resolve()
  })
  requestCounts.push(requestAgentUsageMock.mock.calls.length)

  expect(requestCounts).toEqual([0, 1, 1, 2])
})

it('shows stale usage after a failed refresh and clears the status on recovery', async () => {
  requestAgentUsageMock
    .mockReturnValueOnce(ok(usageResponse('fixture-model-one')))
    .mockReturnValueOnce(err(new Error('fixture refresh failure')))
    .mockReturnValueOnce(ok(usageResponse('fixture-model-two')))

  await act(async () => {
    root?.render(
      <AgentUsageProvider>
        <UsageSnapshot />
      </AgentUsageProvider>,
    )
    await Promise.resolve()
  })
  const snapshot = () => ({
    model: container.querySelector('[data-session-model]')?.textContent ?? null,
    error:
      container
        .querySelector('[data-session-model]')
        ?.getAttribute('data-error') ?? null,
    quotaStatus:
      container.querySelector('.quota-panel-error')?.textContent ?? null,
    requests: requestAgentUsageMock.mock.calls.length,
  })
  const states = [snapshot()]

  await act(async () => {
    await vi.advanceTimersByTimeAsync(60_000)
  })
  states.push(snapshot())

  await act(async () => {
    await vi.advanceTimersByTimeAsync(60_000)
  })
  states.push(snapshot())

  expect(states).toEqual([
    {
      model: 'fixture-model-one',
      error: 'false',
      quotaStatus: null,
      requests: 1,
    },
    {
      model: 'fixture-model-one',
      error: 'true',
      quotaStatus: 'Refresh failed; showing stale usage',
      requests: 2,
    },
    {
      model: 'fixture-model-two',
      error: 'false',
      quotaStatus: null,
      requests: 3,
    },
  ])
})
