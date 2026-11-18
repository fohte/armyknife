import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import { VibeModeControl } from '#components/vibe-mode-control'

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

function renderControl() {
  root = createRoot(container)
  act(() => {
    root?.render(
      <VibeModeControl
        sessionId="fixture-session"
        repo="example/service"
        mode={{ mode: 'disabled' }}
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

it('enables vibe mode and sends the previewed state change', async () => {
  fetchMock.mockResolvedValue(
    new Response(
      JSON.stringify({ ok: true, queuedPromptId: 'queue-fixture' }),
      {
        status: 200,
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

  expect(controlState()).toEqual({
    requests: [
      [
        '/api/sessions/fixture-session/vibe',
        {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ enabled: true }),
        },
      ],
    ],
    status: 'Vibe on',
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
