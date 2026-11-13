import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import { PermissionCard } from '#components/permission-card'
import type { ChatItem } from '#types'

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

it('highlights a multiline Bash permission without overflowing its card', async () => {
  const message: Extract<ChatItem, { type: 'permission' }> = {
    type: 'permission',
    requestId: 'permission-request-example',
    tool: 'Bash',
    description: '',
    command,
  }
  root = createRoot(container)
  act(() => {
    root?.render(
      <PermissionCard
        message={message}
        actions={null}
        actionsReady
        pendingAction={null}
        isCodex={false}
        onAction={() => undefined}
        onOpenInTerminal={() => undefined}
      />,
    )
  })

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
