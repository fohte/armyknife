import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import { VibeModeControl } from '#components/vibe-mode-control'
import type { VibeMode } from '#types'

let container: HTMLDivElement
let root: Root | null = null
const fetchMock = vi.fn()

function requireButton(label: string) {
  const button = Array.from(container.querySelectorAll('button')).find(
    (candidate) => candidate.textContent.includes(label),
  )
  if (button === undefined) throw new Error(`Expected ${label} button`)
  return button
}

function controlState() {
  return {
    requests: fetchMock.mock.calls,
    status: container.querySelector('button')?.getAttribute('aria-label'),
    panel: container.querySelector('[role="dialog"]') !== null,
  }
}

function errorState() {
  return {
    error: container.querySelector('[role="alert"]')?.textContent,
    panel: container.querySelector('[role="dialog"]') !== null,
  }
}

function renderControl(mode: VibeMode = { mode: 'disabled' }) {
  root = createRoot(container)
  act(() => {
    root?.render(
      <VibeModeControl
        sessionId="fixture-session"
        repo="example/service"
        mode={mode}
      />,
    )
  })
}

async function finishRequest() {
  await new Promise((resolve) => setTimeout(resolve, 0))
}

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  vi.stubGlobal('fetch', fetchMock)
  container = document.createElement('div')
  document.body.append(container)
  fetchMock.mockReset()
})

afterEach(() => {
  if (root !== null) act(() => root?.unmount())
  root = null
  container.remove()
  vi.unstubAllGlobals()
})

const modeChangeCases: {
  name: string
  mode: VibeMode
  enabled: boolean
  resultingMode: VibeMode
  triggerLabel: string
  actionLabel: string
  expectedLabel: string
  message: string
}[] = [
  {
    name: 'enables disabled mode',
    mode: { mode: 'disabled' },
    enabled: true,
    resultingMode: { mode: 'enabled' },
    triggerLabel: 'Vibe off',
    actionLabel: 'Turn on and send',
    expectedLabel: 'Vibe on',
    message: 'watch-web からユーザーが vibe mode を on にしました。',
  },
  {
    name: 'keeps inherited mode active when clearing the local setting',
    mode: { mode: 'enabled' },
    enabled: false,
    resultingMode: { mode: 'inherited', ancestorSessionId: 'fixture-parent' },
    triggerLabel: 'Vibe on',
    actionLabel: 'Turn off and send',
    expectedLabel: 'Vibe on · inherited',
    message:
      'watch-web からユーザーが vibe mode を off にしました。祖先セッションから引き継いでいる場合、vibe mode は引き続き有効です。',
  },
  {
    name: 'enables inherited mode explicitly for this session',
    mode: { mode: 'inherited', ancestorSessionId: 'fixture-parent' },
    enabled: true,
    resultingMode: { mode: 'enabled' },
    triggerLabel: 'Vibe on · inherited',
    actionLabel: 'Turn on here and send',
    expectedLabel: 'Vibe on',
    message: 'watch-web からユーザーが vibe mode を on にしました。',
  },
]

it.each(modeChangeCases)('$name', async (testCase) => {
  fetchMock.mockResolvedValue(
    new Response(
      JSON.stringify({
        ok: true,
        queuedPromptId: 'queue-fixture',
        vibeMode: testCase.resultingMode,
      }),
      {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      },
    ),
  )
  renderControl(testCase.mode)

  act(() => {
    requireButton(testCase.triggerLabel).click()
  })
  expect(container.querySelector('blockquote')?.textContent).toBe(
    testCase.message,
  )
  await act(async () => {
    requireButton(testCase.actionLabel).click()
    await finishRequest()
  })

  expect(controlState()).toEqual({
    requests: [
      [
        '/api/sessions/fixture-session/vibe',
        {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ enabled: testCase.enabled }),
        },
      ],
    ],
    status: testCase.expectedLabel,
    panel: false,
  })
})

it('shows the API reason when the server rejects a change', async () => {
  fetchMock.mockResolvedValue(
    new Response(
      JSON.stringify({ error: 'Vibe mode is disabled by repository policy.' }),
      {
        status: 409,
        headers: { 'Content-Type': 'application/json' },
      },
    ),
  )
  renderControl()

  act(() => {
    requireButton('Vibe off').click()
  })
  await act(async () => {
    requireButton('Turn on and send').click()
    await finishRequest()
  })

  expect(errorState()).toEqual({
    error: 'Vibe mode is disabled by repository policy.',
    panel: true,
  })
})
