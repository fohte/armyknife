import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import {
  type PendingPermissionActionState,
  PermissionCard,
} from '#components/permission-card'
import type { ChatItem, SessionActions } from '#types'

let container: HTMLDivElement
let root: Root | null = null

const command = `printf '%s\\n' 'Preparing preview'
cat <<'EOF'
preview content
EOF`

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  container = document.createElement('div')
  container.style.width = '360px'
  document.body.append(container)
})

afterEach(() => {
  if (root !== null) act(() => root?.unmount())
  root = null
  container.remove()
  vi.unstubAllGlobals()
})

function renderPermissionCard(
  message: Extract<ChatItem, { type: 'permission' }>,
  options: {
    actions?: SessionActions | null
    isCodex?: boolean
    pendingAction?: PendingPermissionActionState
  } = {},
) {
  const { actions = null, isCodex = false, pendingAction = null } = options
  root = createRoot(container)
  act(() => {
    root?.render(
      <PermissionCard
        message={message}
        actions={actions}
        actionsReady
        pendingAction={pendingAction}
        isCodex={isCodex}
        onAction={() => undefined}
        onOpenInTerminal={() => undefined}
      />,
    )
  })
}

it('shows web approval and rejection for a Codex permission with capabilities', () => {
  const message: Extract<ChatItem, { type: 'permission' }> = {
    type: 'permission',
    requestId: 'permission-request-example',
    tool: 'Bash',
    description: '',
    command,
  }
  const actions: SessionActions = {
    message: true,
    answerQuestion: false,
    permission: { approve: true, reject: true },
    interrupt: false,
    openInTerminal: true,
    rewind: false,
  }
  renderPermissionCard(message, { actions, isCodex: true })

  const renderedState = () => ({
    buttons: Array.from(container.querySelectorAll('button')).map(
      (button) => button.getAttribute('aria-label') ?? button.textContent,
    ),
    terminalFallback: container.querySelector(
      '.conversation-terminal-fallback',
    ),
  })

  expect(renderedState()).toEqual({
    buttons: ['Approve', 'Reject'],
    terminalFallback: null,
  })
})

it('keeps the terminal fallback for a Codex permission without web capabilities', () => {
  const message: Extract<ChatItem, { type: 'permission' }> = {
    type: 'permission',
    requestId: 'permission-request-example',
    tool: 'Bash',
    description: '',
    command,
  }
  const actions: SessionActions = {
    message: true,
    answerQuestion: false,
    interrupt: false,
    openInTerminal: true,
    rewind: false,
  }
  renderPermissionCard(message, { actions, isCodex: true })

  const renderedState = () => ({
    buttons: Array.from(container.querySelectorAll('button')).map(
      (button) => button.getAttribute('aria-label') ?? button.textContent,
    ),
    fallback: container.querySelector('.conversation-terminal-fallback')
      ?.textContent,
  })

  expect(renderedState()).toEqual({
    buttons: ['Open in terminal'],
    fallback: 'Review this permission in terminal.',
  })
})

it('says Codex is waiting after a web approval is sent', () => {
  const message: Extract<ChatItem, { type: 'permission' }> = {
    type: 'permission',
    requestId: 'permission-request-example',
    tool: 'Bash',
    description: '',
    command,
  }
  const actions: SessionActions = {
    message: true,
    answerQuestion: false,
    permission: { approve: true, reject: true },
    interrupt: false,
    openInTerminal: true,
    rewind: false,
  }
  renderPermissionCard(message, {
    actions,
    isCodex: true,
    pendingAction: {
      action: 'approve',
      status: 'sent',
      requestId: 'permission-request-example',
    },
  })

  expect(
    container.querySelector('.permission-action-status')?.textContent,
  ).toBe('Approval sent. Waiting for Codex…')
})

it('highlights a multiline Bash permission without overflowing its card', async () => {
  const message: Extract<ChatItem, { type: 'permission' }> = {
    type: 'permission',
    requestId: 'permission-request-example',
    tool: 'Bash',
    description: '',
    command,
  }
  renderPermissionCard(message)

  const snapshot = () => {
    const card = container.querySelector('article')
    const code = container.querySelector('.permission-command code')
    return {
      code: code?.textContent ?? null,
      hasColoredToken:
        code !== null && code.querySelector('span[style]') !== null,
      cardOverflows: card === null ? null : card.scrollWidth > card.clientWidth,
      description: container.querySelector('article > p:not(.permission-tool)'),
    }
  }

  expect(snapshot()).toEqual({
    code: command,
    hasColoredToken: false,
    cardOverflows: false,
    description: null,
  })

  await act(async () => {
    const { highlightCode } = await import('./syntax-highlighter')
    await highlightCode(command, 'bash')
  })

  expect(snapshot()).toEqual({
    code: command,
    hasColoredToken: true,
    cardOverflows: false,
    description: null,
  })
})

it('shows the description for a non-Bash permission', () => {
  const message: Extract<ChatItem, { type: 'permission' }> = {
    type: 'permission',
    requestId: 'permission-request-example',
    tool: 'Read',
    description: 'The assistant wants to inspect a sample file.',
  }
  renderPermissionCard(message)

  const snapshot = () => ({
    tool: container.querySelector('.permission-tool')?.textContent ?? null,
    description:
      container.querySelector('article > p:not(.permission-tool)')
        ?.textContent ?? null,
    commandBlock: container.querySelector('.permission-command'),
  })

  expect(snapshot()).toEqual({
    tool: 'Read',
    description: 'The assistant wants to inspect a sample file.',
    commandBlock: null,
  })
})
