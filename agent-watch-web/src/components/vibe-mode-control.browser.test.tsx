import '#components/conversation-pane-view.css'
import '#components/conversation-pane.css'
import '#components/resizable-columns.css'

import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { userEvent } from 'vitest/browser'

import { SessionHeading } from '#components/session-heading'
import { VibeModeControl } from '#components/vibe-mode-control'
import type { VibeMode } from '#types'

let container: HTMLDivElement
let root: Root | null = null
const fetchMock = vi.fn()

function requireButton(label: string) {
  const button = Array.from(document.querySelectorAll('button')).find(
    (candidate) => candidate.textContent.includes(label),
  )
  if (button === undefined) throw new Error(`Expected ${label} button`)
  return button
}

function controlState() {
  return {
    requests: fetchMock.mock.calls,
    status: container.querySelector('button')?.getAttribute('aria-label'),
    panel: document.querySelector('.vibe-control-popover[data-open]') !== null,
  }
}

function errorState() {
  return {
    error: document.querySelector('[role="alert"]')?.textContent,
    panel: document.querySelector('.vibe-control-popover[data-open]') !== null,
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

  await act(async () => {
    await userEvent.click(requireButton(testCase.triggerLabel))
  })
  expect(document.querySelector('blockquote')?.textContent).toBe(
    testCase.message,
  )
  await act(async () => {
    await userEvent.click(requireButton(testCase.actionLabel))
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

  await act(async () => {
    await userEvent.click(requireButton('Vibe off'))
  })
  await act(async () => {
    await userEvent.click(requireButton('Turn on and send'))
    await finishRequest()
  })

  expect(errorState()).toEqual({
    error: 'Vibe mode is disabled by repository policy.',
    panel: true,
  })
})

it.each([
  {
    name: 'Escape',
    dismiss: () => userEvent.keyboard('{Escape}'),
  },
  {
    name: 'an outside click',
    dismiss: () => userEvent.click(document.body),
  },
])('closes the desktop popover on $name', async ({ dismiss }) => {
  renderControl()

  await act(async () => {
    await userEvent.click(requireButton('Vibe off'))
  })
  expect(document.querySelector('.vibe-control-popover')).not.toBeNull()

  await act(async () => {
    await dismiss()
  })

  expect(document.querySelector('.vibe-control-popover')).toBeNull()
})

it('closes the mobile sheet on Escape', async () => {
  const onOpenChange = vi.fn()
  root = createRoot(container)
  act(() => {
    root?.render(
      <VibeModeControl
        sessionId="fixture-session"
        repo="example/service"
        mode={{ mode: 'disabled' }}
        isMobile
        open
        onOpenChange={onOpenChange}
      />,
    )
  })

  await act(async () => {
    await userEvent.keyboard('{Escape}')
  })

  expect(onOpenChange.mock.calls).toEqual([[false]])
})

it('keeps the mobile sheet open when its contents are clicked', async () => {
  const onOpenChange = vi.fn()
  root = createRoot(container)
  act(() => {
    root?.render(
      <VibeModeControl
        sessionId="fixture-session"
        repo="example/service"
        mode={{ mode: 'disabled' }}
        isMobile
        open
        onOpenChange={onOpenChange}
      />,
    )
  })
  const content = document.querySelector('.vibe-panel-message')
  if (content === null) throw new Error('Expected the vibe panel content')

  await act(async () => {
    await userEvent.click(content)
  })

  const snapshot = () => ({
    panelOpen: document.querySelector('.vibe-panel-backdrop') !== null,
    openChanges: onOpenChange.mock.calls,
  })
  expect(snapshot()).toEqual({ panelOpen: true, openChanges: [] })
})

it.each([
  { name: 'short title', title: 'Fixture title' },
  {
    name: 'long title',
    title:
      'A fictional conversation title long enough to reach the right edge of the pane',
  },
])(
  'keeps the desktop popover inside its pane for a $name',
  async ({ title }) => {
    container.className = 'resizable-columns-pane'
    container.style.cssText =
      'box-sizing: border-box; height: 260px; overflow: hidden; position: relative; width: 520px;'
    root = createRoot(container)
    act(() => {
      root?.render(
        <div className="conversation-pane">
          <header className="conversation-header">
            <div className="conversation-header-copy">
              <div className="session-title-heading">
                <SessionHeading title={title} sessionId="fixture-session" />
                <VibeModeControl
                  sessionId="fixture-session"
                  repo="example/service"
                  mode={{ mode: 'disabled' }}
                  open
                />
                <span className="session-title-edit" />
              </div>
            </div>
            <div className="conversation-header-actions" />
          </header>
        </div>,
      )
    })

    await vi.waitFor(() => {
      const popover = document.querySelector<HTMLElement>(
        '.vibe-control-popover',
      )
      if (popover === null) throw new Error('Expected vibe mode popover')
      const paneRect = container.getBoundingClientRect()
      const popoverRect = popover.getBoundingClientRect()
      expect(
        popoverRect.top >= paneRect.top &&
          popoverRect.right <= paneRect.right &&
          popoverRect.bottom <= paneRect.bottom &&
          popoverRect.left >= paneRect.left,
      ).toEqual(true)
    })
  },
)

it('anchors the desktop popover when its trigger is hidden', async () => {
  container.className = 'resizable-columns-pane'
  container.style.cssText =
    'box-sizing: border-box; height: 260px; overflow: hidden; position: relative; width: 520px;'
  root = createRoot(container)
  act(() => {
    root?.render(
      <VibeModeControl
        sessionId="fixture-session"
        repo="example/service"
        mode={{ mode: 'disabled' }}
        showTrigger={false}
        open
      />,
    )
  })

  await vi.waitFor(() => {
    const popover = document.querySelector<HTMLElement>('.vibe-control-popover')
    if (popover === null) throw new Error('Expected vibe mode popover')
    const paneRect = container.getBoundingClientRect()
    const popoverRect = popover.getBoundingClientRect()
    expect(
      popoverRect.top >= paneRect.top &&
        popoverRect.right <= paneRect.right &&
        popoverRect.bottom <= paneRect.bottom &&
        popoverRect.left >= paneRect.left,
    ).toEqual(true)
  })
})
