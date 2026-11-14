import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { userEvent } from 'vitest/browser'

import { MarkdownMessage } from '#components/markdown-message'
import {
  SessionOperationProvider,
  useSessionOperations,
} from '#session-operation-context'

let container: HTMLDivElement
let root: Root | null = null
let resolvePreview: ((response: Response) => void) | null = null

function ConversationHarness() {
  const { pendingOperations } = useSessionOperations()
  return (
    <>
      <output data-testid="pending-operations">
        {pendingOperations.map(({ status }) => status).join(',')}
      </output>
      <MarkdownMessage
        onCommandExecuted={() => undefined}
        sessionId="sample-session"
      >
        {'Run `! printf safe`.'}
      </MarkdownMessage>
    </>
  )
}

beforeEach(() => {
  resolvePreview = null
  vi.stubGlobal(
    'fetch',
    vi.fn((input: RequestInfo | URL) => {
      const url =
        typeof input === 'string'
          ? input
          : input instanceof URL
            ? input.href
            : input.url
      if (url.endsWith('/commands/preview')) {
        return new Promise<Response>((resolve) => {
          resolvePreview = resolve
        })
      }
      return Promise.resolve(
        new Response(
          JSON.stringify({ exitCode: 0, succeeded: true, timedOut: false }),
          { headers: { 'Content-Type': 'application/json' } },
        ),
      )
    }),
  )
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  container = document.createElement('div')
  document.body.append(container)
})

afterEach(() => {
  if (root !== null) act(() => root?.unmount())
  root = null
  container.remove()
  vi.unstubAllGlobals()
})

it('keeps the command dialog available when provider state rerenders its parent', async () => {
  root = createRoot(container)
  act(() => {
    root?.render(
      <SessionOperationProvider>
        <ConversationHarness />
      </SessionOperationProvider>,
    )
  })

  const runButton = container.querySelector<HTMLButtonElement>('button')
  if (runButton === null) throw new Error('Expected the command button')
  await act(async () => userEvent.click(runButton))

  const pending = container.querySelector('[data-testid="pending-operations"]')
  const completePreview = resolvePreview
  if (pending === null || completePreview === null) {
    throw new Error('Expected the provider to start the preview request')
  }
  const pendingDuringPreview = pending.textContent

  await act(async () => {
    completePreview(
      new Response(JSON.stringify({ cwd: '/workspace/example' }), {
        headers: { 'Content-Type': 'application/json' },
      }),
    )
    await new Promise((resolve) => window.setTimeout(resolve, 0))
  })

  const dialog = document.body.querySelector('[role="dialog"]')
  if (!(dialog instanceof HTMLElement)) {
    throw new Error('Expected the command confirmation dialog')
  }
  const dialogSnapshot = {
    command: dialog.querySelector('dd pre')?.textContent ?? null,
    cwd: dialog.querySelector('dd code')?.textContent ?? null,
    title: dialog.querySelector('h2')?.textContent ?? null,
  }
  const confirmButton = Array.from(dialog.querySelectorAll('button')).find(
    (button) => button.textContent === 'Run command',
  )
  if (!(confirmButton instanceof HTMLButtonElement)) {
    throw new Error('Expected the confirmation button')
  }

  await act(async () => userEvent.click(confirmButton))

  const getSnapshot = () => ({
    dialog: dialogSnapshot,
    pendingDuringPreview,
    pending: container.querySelector('[data-testid="pending-operations"]')
      ?.textContent,
    status: container.querySelector('[role="status"]')?.textContent,
  })
  expect(getSnapshot()).toEqual({
    dialog: {
      command: 'printf safe',
      cwd: '/workspace/example',
      title: 'Run agent command?',
    },
    pendingDuringPreview: 'sending',
    pending: '',
    status: 'Completed',
  })
})
