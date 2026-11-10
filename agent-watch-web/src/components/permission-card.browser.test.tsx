import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import { PermissionCard } from '#components/permission-card'
import type { ChatItem, SessionActions } from '#types'

const message: Extract<ChatItem, { type: 'permission' }> = {
  type: 'permission',
  tool: 'Shell',
  description: 'Run the formatter on changed files.',
}

const actions: SessionActions = {
  message: true,
  answerQuestion: false,
  permission: { approve: true, reject: true },
  interrupt: true,
  openInTerminal: true,
}

let container: HTMLDivElement
let root: Root | null = null

function render(props: {
  actions: SessionActions | null
  actionsReady: boolean
  engine: 'Claude' | 'Codex'
  onAction: (action: string) => Promise<boolean>
}) {
  root ??= createRoot(container)
  act(() => {
    root?.render(<PermissionCard message={message} {...props} />)
  })
}

function snapshot(actionCalls: unknown[]) {
  const card = container.querySelector('.chat-permission-card')
  return {
    view:
      card === null
        ? null
        : {
            title: card.querySelector('h2')?.textContent,
            tool: card.querySelector('.permission-tool')?.textContent,
            description: card.querySelector('p:not(.permission-tool)')
              ?.textContent,
            buttons: Array.from(card.querySelectorAll('button'), (button) => ({
              text: button.textContent,
              disabled: button.disabled,
            })),
            terminalMessage: card.querySelector(
              '.conversation-terminal-fallback span',
            )?.textContent,
          },
    actionCalls,
  }
}

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

it('renders permission details and dispatches approval actions', () => {
  const onAction = vi.fn(() => Promise.resolve(true))
  render({ actions, actionsReady: true, engine: 'Claude', onAction })

  act(() => {
    for (const button of container.querySelectorAll('button')) button.click()
  })

  expect(snapshot(onAction.mock.calls)).toEqual({
    view: {
      title: 'Permission required',
      tool: 'Shell',
      description: 'Run the formatter on changed files.',
      buttons: [
        { text: 'Approve', disabled: false },
        { text: 'Reject', disabled: false },
      ],
      terminalMessage: undefined,
    },
    actionCalls: [['permission/approve'], ['permission/reject']],
  })
})

it('shows the terminal fallback for a Codex permission request', () => {
  const onAction = vi.fn(() => Promise.resolve(true))
  render({
    actions: {
      message: true,
      answerQuestion: false,
      interrupt: true,
      openInTerminal: true,
    },
    actionsReady: true,
    engine: 'Codex',
    onAction,
  })

  act(() => {
    container.querySelector<HTMLButtonElement>('button')?.click()
  })

  expect(snapshot(onAction.mock.calls)).toEqual({
    view: {
      title: 'Permission required',
      tool: 'Shell',
      description: 'Run the formatter on changed files.',
      buttons: [{ text: 'Open in terminal', disabled: false }],
      terminalMessage: 'Review this permission in terminal.',
    },
    actionCalls: [['focus']],
  })
})
