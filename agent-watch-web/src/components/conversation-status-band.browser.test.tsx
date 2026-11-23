import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { userEvent } from 'vitest/browser'

import { ConversationStatusBand } from '#components/conversation-status-band'

let container: HTMLDivElement
let root: Root | null = null

const backgroundTasks = [
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
] satisfies Parameters<typeof ConversationStatusBand>[0]['backgroundTasks']

const backgroundTaskDetails = [
  'Rebuild the sample index · 2m 8s',
  'Check the sample bundle · 1m 2s',
]

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
        backgroundTasks={backgroundTasks}
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
    details: backgroundTaskDetails,
    chevron: '▾',
  })
})

it.each([
  {
    name: 'collapsed',
    state: { kind: 'working', detail: null } as const,
    defaultExpanded: false,
    chevron: '›',
  },
  {
    name: 'expanded',
    state: { kind: 'working', detail: null } as const,
    defaultExpanded: true,
    chevron: '▾',
  },
  {
    name: 'card shortcut',
    state: { kind: 'question', detail: null } as const,
    defaultExpanded: false,
    chevron: '↑',
  },
])(
  'aligns the empty-detail $name status row at both edges',
  ({ state, defaultExpanded, chevron }) => {
    container.style.width = '760px'
    root = createRoot(container)
    act(() => {
      root?.render(
        <ConversationStatusBand
          state={state}
          elapsedSeconds={6}
          activeTool={undefined}
          backgroundTasks={[]}
          defaultExpanded={defaultExpanded}
        />,
      )
    })

    const toggle = container.querySelector<HTMLButtonElement>(
      '.conversation-status-band-toggle',
    )
    const dot = container.querySelector<HTMLElement>(
      '.conversation-status-band-dot',
    )
    const label = container.querySelector<HTMLElement>(
      '.conversation-status-band-label',
    )
    const arrow = container.querySelector<HTMLElement>(
      '.conversation-status-band-chevron',
    )
    if (toggle === null || dot === null || label === null || arrow === null) {
      throw new Error('The status row was not rendered')
    }

    const getAlignment = () => {
      const toggleRect = toggle.getBoundingClientRect()
      const dotRect = dot.getBoundingClientRect()
      const labelRect = label.getBoundingClientRect()
      const arrowRect = arrow.getBoundingClientRect()
      const style = getComputedStyle(toggle)

      return {
        dotStartsAtContentEdge:
          Math.abs(
            dotRect.left -
              toggleRect.left -
              Number.parseFloat(style.paddingLeft),
          ) < 1,
        labelFollowsDot: labelRect.left >= dotRect.right,
        chevronAtContentEnd:
          Math.abs(
            toggleRect.right -
              Number.parseFloat(style.paddingRight) -
              arrowRect.right,
          ) < 1,
        chevron: arrow.textContent,
      }
    }
    expect(getAlignment()).toEqual({
      dotStartsAtContentEdge: true,
      labelFollowsDot: true,
      chevronAtContentEnd: true,
      chevron,
    })
  },
)

it.each([
  {
    kind: 'working' as const,
    state: { kind: 'working', detail: null } as const,
    activeTool: {
      tool: 'exec',
      target: 'cargo test --package fictional -- --show-output',
    },
    leadingDetail: 'Bash  cargo test --package fictional -- --show-output',
  },
  {
    kind: 'compacting' as const,
    state: { kind: 'compacting', detail: null } as const,
    activeTool: undefined,
    leadingDetail: '古い会話を要約しています',
  },
])(
  'shows all background tasks after expanding a $kind status row',
  async ({ state, activeTool, leadingDetail }) => {
    root = createRoot(container)
    act(() => {
      root?.render(
        <ConversationStatusBand
          state={state}
          elapsedSeconds={92}
          activeTool={activeTool}
          backgroundTasks={backgroundTasks}
        />,
      )
    })

    const toggle = container.querySelector<HTMLButtonElement>(
      '.conversation-status-band-toggle',
    )
    if (toggle === null) throw new Error('The status row was not rendered')
    await act(async () => userEvent.click(toggle))

    const snapshot = () => ({
      expanded: toggle.getAttribute('aria-expanded'),
      details: Array.from(
        container.querySelectorAll('.conversation-status-band-expanded p'),
        (detail) => detail.textContent,
      ),
    })
    expect(snapshot()).toEqual({
      expanded: 'true',
      details: [leadingDetail, ...backgroundTaskDetails],
    })
  },
)

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
  'keeps background tasks available from the $kind card shortcut',
  async ({ kind, detail }) => {
    const onReturnToCard = vi.fn()
    root = createRoot(container)
    act(() => {
      root?.render(
        <ConversationStatusBand
          state={{ kind, detail }}
          elapsedSeconds={0}
          activeTool={undefined}
          backgroundTasks={backgroundTasks}
          isMobile
          onReturnToCard={onReturnToCard}
        />,
      )
    })

    const toggle = container.querySelector<HTMLButtonElement>(
      '.conversation-status-band-toggle',
    )
    const summary = container.querySelector<HTMLElement>(
      '.conversation-status-band-tasks summary',
    )
    if (toggle === null || summary === null) {
      throw new Error('The status row or task disclosure was not rendered')
    }
    await act(async () => userEvent.click(toggle))
    await act(async () => userEvent.click(summary))

    const disclosure = container.querySelector<HTMLDetailsElement>(
      '.conversation-status-band-tasks',
    )
    const band = container.querySelector<HTMLElement>(
      '.conversation-status-band',
    )
    const snapshot = () => ({
      statusLabel: toggle.querySelector('.conversation-status-band-label')
        ?.textContent,
      arrow: toggle.querySelector('.conversation-status-band-chevron')
        ?.textContent,
      returnCalls: onReturnToCard.mock.calls,
      taskDisclosureOpen: disclosure?.open,
      tasks: Array.from(
        container.querySelectorAll('.conversation-status-band-tasks li'),
        (item) => item.textContent,
      ),
      bandHeight: Math.round(band?.getBoundingClientRect().height ?? 0),
    })
    expect(snapshot()).toEqual({
      statusLabel: kind === 'question' ? 'Question' : 'Permission',
      arrow: '↑',
      returnCalls: [[]],
      taskDisclosureOpen: true,
      tasks: ['Rebuild the sample index2m 8s', 'Check the sample bundle1m 2s'],
      bandHeight: 44,
    })
  },
)

it('opens the draft review panel from the review band', async () => {
  const onOpenDrafts = vi.fn()
  root = createRoot(container)
  act(() => {
    root?.render(
      <ConversationStatusBand
        state={{
          kind: 'draft-review',
          detail: '2 drafts · Issue comment #1234 is in the editor',
        }}
        elapsedSeconds={0}
        activeTool={undefined}
        backgroundTasks={[]}
        onOpenDrafts={onOpenDrafts}
      />,
    )
  })

  const button = container.querySelector<HTMLButtonElement>(
    '.conversation-status-band-review',
  )
  if (button === null)
    throw new Error('The draft review button was not rendered')
  await act(async () => userEvent.click(button))

  const snapshot = () => ({
    text: button.textContent,
    calls: onOpenDrafts.mock.calls.length,
  })
  expect(snapshot()).toEqual({ text: 'Open drafts →', calls: 1 })
})

it('shows the full active-tool command after the compact row is pressed', async () => {
  const activeToolDetail =
    'Bash  cargo test --package fictional -- --show-output'
  root = createRoot(container)
  act(() => {
    root?.render(
      <ConversationStatusBand
        state={{ kind: 'working', detail: null }}
        elapsedSeconds={92}
        activeTool={{
          tool: 'exec',
          target: 'cargo test --package fictional -- --show-output',
        }}
        backgroundTasks={[]}
      />,
    )
  })

  const toggle = container.querySelector<HTMLButtonElement>(
    '.conversation-status-band-toggle',
  )
  if (toggle === null) throw new Error('The status row was not rendered')
  await act(async () => userEvent.click(toggle))

  const snapshot = () => ({
    expanded: toggle.getAttribute('aria-expanded'),
    detail: container.querySelector('.conversation-status-band-expanded p')
      ?.textContent,
  })
  expect(snapshot()).toEqual({ expanded: 'true', detail: activeToolDetail })
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
