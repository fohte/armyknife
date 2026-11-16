import { ok } from 'neverthrow'
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import type { AgentUsage } from '#agent-usage'
import {
  AgentUsageProvider,
  useAgentUsageContext,
} from '#components/agent-usage-context'

const { requestAgentUsageMock } = vi.hoisted(() => ({
  requestAgentUsageMock: vi.fn(),
}))

vi.mock('#agent-usage', () => ({
  requestAgentUsage: requestAgentUsageMock,
}))

let container: HTMLDivElement
let root: Root | null

function UsageSnapshot() {
  const { usage } = useAgentUsageContext()
  return <span>{usage?.sessions[0]?.model ?? 'loading'}</span>
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
    quotas: { claude: null, codex: null },
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
  vi.unstubAllGlobals()
  vi.restoreAllMocks()
})

it('refreshes usage every thirty seconds', async () => {
  requestAgentUsageMock
    .mockReturnValueOnce(ok(usageResponse('fixture-model-one')))
    .mockReturnValueOnce(ok(usageResponse('fixture-model-two')))

  await act(async () => {
    root?.render(
      <AgentUsageProvider>
        <UsageSnapshot />
      </AgentUsageProvider>,
    )
    await Promise.resolve()
  })
  expect(container.textContent).toEqual('fixture-model-one')

  await act(async () => {
    await vi.advanceTimersByTimeAsync(30_000)
  })

  const snapshot = () => ({
    model: container.textContent,
    requests: requestAgentUsageMock.mock.calls.length,
  })
  expect(snapshot()).toEqual({ model: 'fixture-model-two', requests: 2 })
})
