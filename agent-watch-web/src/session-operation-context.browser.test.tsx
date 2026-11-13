import { act, useMemo, useOptimistic, useState } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import {
  SessionOperationProvider,
  useSessionOperations,
} from '#session-operation-context'
import {
  applySessionOperation,
  type SessionOperation,
} from '#session-operations'
import type { Session } from '#types'

const session: Session = {
  id: 'optimistic-session',
  category: 'needs_action',
  isPaused: false,
  kind: null,
  title: 'Sample task',
  preview: null,
  elapsedSeconds: 0,
  unread: false,
  task: null,
  repo: 'sample/repository',
  project: 'Sample project',
  parent: null,
  ancestorSessionIds: [],
  pullRequest: null,
  isToday: false,
}

const operation: SessionOperation = {
  type: 'today',
  sessionId: session.id,
  isToday: true,
}

let container: HTMLDivElement
let root: Root | null = null
let resolvePost: ((response: Response) => void) | null = null
let resolveRefresh: (() => void) | null = null
let confirmServerState: (() => void) | null = null
let operationRequest: Promise<void> | null = null

function OperationHarness() {
  const [sessions, setSessions] = useState([session])
  const { pendingOperations, run, confirm } = useSessionOperations()
  const sessionsWithPendingOperations = useMemo(
    () =>
      pendingOperations.reduce(
        (current, { operation: pendingOperation }) =>
          applySessionOperation(current, pendingOperation),
        sessions,
      ),
    [pendingOperations, sessions],
  )
  const [optimisticSessions, dispatch] = useOptimistic(
    sessionsWithPendingOperations,
    applySessionOperation,
  )
  const [error, setError] = useState<string | null>(null)

  confirmServerState = () => {
    setSessions(applySessionOperation([session], operation))
    confirm(operation)
  }
  const submit = () => {
    operationRequest = run(operation, {
      optimisticDispatchers: [dispatch],
      refresh: () =>
        new Promise<void>((resolve) => {
          resolveRefresh = resolve
        }),
    }).then((result) => {
      result.match(
        () => {
          setError(null)
        },
        (requestError) => {
          setError(requestError.message)
        },
      )
    })
  }
  return (
    <>
      <span data-testid="starred">
        {String(optimisticSessions[0]?.isToday === true)}
      </span>
      <span data-testid="pending">
        {pendingOperations.map(({ status }) => status).join(',')}
      </span>
      <span data-testid="error">{error}</span>
      <button type="button" onClick={submit}>
        Star
      </button>
    </>
  )
}

function renderHarness() {
  root = createRoot(container)
  act(() => {
    root?.render(
      <SessionOperationProvider>
        <OperationHarness />
      </SessionOperationProvider>,
    )
  })
}

function snapshot() {
  return {
    starred: container.querySelector('[data-testid="starred"]')?.textContent,
    pending: container.querySelector('[data-testid="pending"]')?.textContent,
    error: container.querySelector('[data-testid="error"]')?.textContent,
  }
}

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  container = document.createElement('div')
  document.body.append(container)
  resolvePost = null
  resolveRefresh = null
  confirmServerState = null
  operationRequest = null
})

afterEach(() => {
  if (root !== null) act(() => root?.unmount())
  root = null
  container.remove()
  vi.unstubAllGlobals()
})

it('shows the optimistic state until the server confirms it', async () => {
  vi.stubGlobal(
    'fetch',
    vi.fn<typeof fetch>(
      () =>
        new Promise<Response>((resolve) => {
          resolvePost = resolve
        }),
    ),
  )
  renderHarness()
  const button = container.querySelector<HTMLButtonElement>('button')
  if (button === null) throw new Error('Expected the operation button')
  act(() => {
    button.click()
  })
  const beforeResponse = snapshot()

  const completePost = resolvePost
  if (completePost === null) throw new Error('Expected the POST request')
  await act(async () => {
    completePost(new Response(null, { status: 204 }))
    await operationRequest
  })
  const afterPost = snapshot()

  const finishRefresh = resolveRefresh
  if (finishRefresh === null)
    throw new Error('Expected the conversation refresh')
  await act(async () => {
    finishRefresh()
    await Promise.resolve()
  })
  const afterStaleRefresh = snapshot()

  const confirm = confirmServerState
  if (confirm === null) throw new Error('Expected a confirmation callback')
  act(confirm)
  await act(async () => {
    await Promise.resolve()
  })
  const afterConfirmation = snapshot()

  const completeSnapshot = () => ({
    beforeResponse,
    afterPost,
    afterStaleRefresh,
    afterConfirmation,
  })
  expect(completeSnapshot()).toEqual({
    beforeResponse: { starred: 'true', pending: 'sending', error: '' },
    afterPost: { starred: 'true', pending: 'sent', error: '' },
    afterStaleRefresh: { starred: 'true', pending: 'sent', error: '' },
    afterConfirmation: { starred: 'true', pending: '', error: '' },
  })
})

it('rolls back the optimistic state and shows an error when the POST fails', async () => {
  vi.stubGlobal(
    'fetch',
    vi.fn<typeof fetch>(
      () =>
        new Promise<Response>((resolve) => {
          resolvePost = resolve
        }),
    ),
  )
  renderHarness()
  const button = container.querySelector<HTMLButtonElement>('button')
  if (button === null) throw new Error('Expected the operation button')
  act(() => {
    button.click()
  })
  const beforeFailure = snapshot()

  const completePost = resolvePost
  if (completePost === null) throw new Error('Expected the POST request')
  await act(async () => {
    completePost(
      Response.json({ error: 'The operation was rejected.' }, { status: 500 }),
    )
    await operationRequest
  })

  const failureSnapshot = () => ({
    beforeFailure,
    afterFailure: snapshot(),
  })
  expect(failureSnapshot()).toEqual({
    beforeFailure: { starred: 'true', pending: 'sending', error: '' },
    afterFailure: {
      starred: 'false',
      pending: '',
      error: 'The operation was rejected.',
    },
  })
})
