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

it('formats separators for display while retaining the original permission command', () => {
  const originalCommand = `printf '%s\\n' "keep && quoted" && printf '%s\\n' ./sample/long/path | sed 's/path/file/'`
  const message: Extract<ChatItem, { type: 'permission' }> = {
    type: 'permission',
    requestId: 'permission-request-example',
    tool: 'Bash',
    description: '',
    command: originalCommand,
  }
  renderPermissionCard(message)

  const snapshot = () => ({
    displayedCommand: container.querySelector('.permission-command code')
      ?.textContent,
    messageCommand: message.command,
  })

  expect(snapshot()).toEqual({
    displayedCommand: `printf '%s\\n' "keep && quoted" &&\n  printf '%s\\n' ./sample/long/path |\n  sed 's/path/file/'`,
    messageCommand: originalCommand,
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

it('renders grouped thought activity inside a permission card', () => {
  const message: Extract<ChatItem, { type: 'permission' }> = {
    type: 'permission',
    requestId: 'permission-request-fictional',
    tool: 'Read',
    description: 'Read the fictional source file.',
    items: [
      { type: 'thinking', text: 'Review the fictional source request.' },
      {
        type: 'tool_use',
        tool: 'Read',
        target: 'src/fixture.rs',
        failed: false,
      },
    ],
  }
  renderPermissionCard(message)

  expect(container.innerHTML).toBe(
    '<article class="chat-permission-card"><h2>Permission required</h2><p class="permission-tool">Read</p><p>Read the fictional source file.</p><div class="permission-tool-activity"><details class="chat-tool-summary" data-has-operations="true"><summary><span class="chat-tool-summary-text">Thought ×1 · Read ×1</span></summary><ul class="chat-tool-operations"><li class="chat-tool-thought"><details><summary><span class="chat-tool-name">Thought</span><span class="chat-tool-thought-preview">Review the fictional source request.</span><span class="chat-tool-thought-length">36 chars</span></summary><p class="chat-tool-thought-full">Review the fictional source request.</p></details></li><li data-has-target="true" data-failed="false" data-has-status="false"><span class="chat-tool-name">Read</span><code class="chat-tool-target">src/fixture.rs</code></li></ul></details></div></article>',
  )
})
