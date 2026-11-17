import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { userEvent } from 'vitest/browser'

import { ConversationStatusBand } from '#components/conversation-status-band'

let container: HTMLDivElement
let root: Root | null = null

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  container = document.createElement('div')
  document.body.append(container)
})

afterEach(() => {
  if (root !== null) act(() => root?.unmount())
  root = null
  container.remove()
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

it('expands to every background task when the compact row is pressed', async () => {
  root = createRoot(container)
  act(() => {
    root?.render(
      <ConversationStatusBand
        state={{ kind: 'background-tasks', detail: 'Rebuild the sample index' }}
        elapsedSeconds={92}
        activeTool={undefined}
        backgroundTasks={[
          {
            id: 'fixture-task-one',
            description: 'Rebuild the sample index',
            elapsedSeconds: 128,
            kind: 'background_task',
          },
          {
            id: 'fixture-task-two',
            description: 'Check the sample bundle',
            elapsedSeconds: 62,
            kind: 'background_task',
          },
        ]}
      />,
    )
  })

  const toggle = container.querySelector<HTMLButtonElement>(
    '.conversation-status-band-toggle',
  )
  if (toggle === null) throw new Error('The status row was not rendered')
  const snapshot = () => ({
    expanded: toggle.getAttribute('aria-expanded'),
    details: Array.from(
      container.querySelectorAll('.conversation-status-band-expanded p'),
      (detail) => detail.textContent,
    ),
    chevron: container.querySelector('.conversation-status-band-chevron')
      ?.textContent,
  })

  expect(snapshot()).toEqual({
    expanded: 'false',
    details: [],
    chevron: '›',
  })
  await act(async () => userEvent.click(toggle))
  expect(snapshot()).toEqual({
    expanded: 'true',
    details: [
      'Rebuild the sample index · 2m 8s',
      'Check the sample bundle · 1m 2s',
    ],
    chevron: '▾',
  })
})

it.each([
  {
    kind: 'question' as const,
    detail: 'Which cache window should be used?',
  },
  {
    kind: 'permission' as const,
    detail: 'Bash  cargo run --package fictional',
  },
])(
  'returns from the $kind row to its conversation card',
  async ({ kind, detail }) => {
    const onReturnToCard = vi.fn()
    root = createRoot(container)
    act(() => {
      root?.render(
        <ConversationStatusBand
          state={{ kind, detail }}
          elapsedSeconds={0}
          activeTool={undefined}
          backgroundTasks={[]}
          onReturnToCard={onReturnToCard}
        />,
      )
    })

    const toggle = container.querySelector<HTMLButtonElement>(
      '.conversation-status-band-toggle',
    )
    if (toggle === null) throw new Error('The status row was not rendered')
    await act(async () => userEvent.click(toggle))

    const snapshot = () => ({
      label: toggle.querySelector('.conversation-status-band-label')
        ?.textContent,
      detail: toggle.querySelector('.conversation-status-band-inline-detail')
        ?.textContent,
      arrow: toggle.querySelector('.conversation-status-band-chevron')
        ?.textContent,
      expanded: toggle.getAttribute('aria-expanded'),
      returnCalls: onReturnToCard.mock.calls,
    })
    expect(snapshot()).toEqual({
      label: kind === 'question' ? 'Question' : 'Permission',
      detail,
      arrow: '↑',
      expanded: null,
      returnCalls: [[]],
    })
  },
)
