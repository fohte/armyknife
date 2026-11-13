import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import { ConversationPaneHeader } from '#components/conversation-pane-header'
import type { Session, SessionActions } from '#types'

vi.mock('#components/copy-session-id-control', () => ({
  CopySessionIdControl: () => null,
}))

let container: HTMLDivElement
let root: Root | null = null

const session: Session = {
  id: 'fixture-session',
  engine: 'claude',
  category: 'needs_action',
  isPaused: false,
  kind: 'Message',
  title: 'A fixture title',
  preview: null,
  elapsedSeconds: 0,
  unread: false,
  task: null,
  repo: 'example/project',
  project: 'project',
  parent: null,
  ancestorSessionIds: [],
  pullRequest: null,
}

const actions: SessionActions = {
  message: true,
  answerQuestion: false,
  permission: { approve: false, reject: false },
  interrupt: false,
  openInTerminal: false,
}

function requireElement<T extends Element>(element: T | null): T {
  if (element === null) throw new Error('Expected element was not rendered')
  return element
}

function renderHeader(
  isMobile: boolean,
  onAction: (
    action: string,
    payload?: Record<string, unknown>,
  ) => Promise<boolean>,
) {
  act(() => {
    root?.render(
      <ConversationPaneHeader
        session={session}
        sessions={[]}
        actions={actions}
        isMobile={isMobile}
        statusPanelOpen={false}
        backgroundTasks={[]}
        openInTerminalStatus={null}
        isToday={false}
        onAction={onAction}
        onOpenInTerminal={() => undefined}
        onSelectSession={() => undefined}
        onToggleStatusPanel={() => undefined}
      />,
    )
  })
}

function setInputValue(input: HTMLInputElement, value: string) {
  const valueSetter = Reflect.getOwnPropertyDescriptor(
    input.constructor.prototype,
    'value',
  )?.set
  if (valueSetter !== undefined) Reflect.apply(valueSetter, input, [value])
  input.dispatchEvent(new Event('input', { bubbles: true }))
}

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

it.each([
  { layout: 'desktop', isMobile: false, title: 'Updated fixture title' },
  { layout: 'mobile', isMobile: true, title: '' },
])('$layout header saves an edited title', async ({ isMobile, title }) => {
  const onAction = vi.fn(
    (action: string, payload?: Record<string, unknown>): Promise<boolean> => {
      return Promise.resolve(
        action === 'title' && typeof payload?.['title'] === 'string',
      )
    },
  )
  renderHeader(isMobile, onAction)

  act(() => {
    requireElement(
      container.querySelector<HTMLButtonElement>(
        '[aria-label="Edit session title"]',
      ),
    ).click()
  })
  const input = requireElement(
    container.querySelector<HTMLInputElement>(
      'input[aria-label="Session title"]',
    ),
  )
  act(() => {
    setInputValue(input, title)
  })
  await act(async () => {
    requireElement(
      container.querySelector<HTMLButtonElement>('button[type="submit"]'),
    ).click()
    await Promise.resolve()
  })

  const snapshot = () => ({
    action: onAction.mock.calls,
    editorVisible:
      container.querySelector('input[aria-label="Session title"]') !== null,
  })
  expect(snapshot()).toEqual({
    action: [['title', { title }]],
    editorVisible: false,
  })
})
