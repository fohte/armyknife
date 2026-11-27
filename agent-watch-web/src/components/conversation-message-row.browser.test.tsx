import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import { ConversationMessageRow } from '#components/conversation-message-row'

let container: HTMLDivElement
let root: Root | null = null

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

it('omits an empty standalone thinking summary', () => {
  act(() => {
    root?.render(
      <ConversationMessageRow
        actions={null}
        actionsReady={false}
        isMobile
        message={{ type: 'thinking', text: '  ' }}
        pendingAction={null}
        openInTerminalStatus={null}
        onAction={() => Promise.resolve(false)}
        onOpenInTerminal={() => undefined}
        onSelectSession={() => undefined}
        sessionEngine="codex"
        sessions={[]}
      />,
    )
  })

  expect(container.innerHTML).toEqual('')
})

it('shows unavailable for an unmapped local image in an assistant reply', () => {
  act(() => {
    root?.render(
      <ConversationMessageRow
        actions={null}
        actionsReady={false}
        isMobile={false}
        message={{
          type: 'assistant',
          text: '![fixture][sample-image]\n\n[sample-image]: </tmp/fixture.png>',
        }}
        pendingAction={null}
        openInTerminalStatus={null}
        onAction={() => Promise.resolve(false)}
        onOpenInTerminal={() => undefined}
        onSelectSession={() => undefined}
        sessionEngine="codex"
        sessions={[]}
      />,
    )
  })

  expect(container.innerHTML).toBe(
    '<article class="chat-assistant-message"><div class="markdown-message"><p><span class="markdown-image-unavailable">表示できません: /tmp/fixture.png</span></p></div></article>',
  )
})
