import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { page } from 'vitest/browser'

import { useEditorReviewDocument } from '#components/use-editor-review-document'
import type { Session } from '#types'

const session: Session = {
  id: 'fixture-session',
  category: 'needs_action',
  isPaused: false,
  kind: 'Review',
  title: 'Review the sample service',
  preview: null,
  elapsedSeconds: 0,
  unread: false,
  task: null,
  repo: 'sample-org/sample-repo',
  project: 'sample service',
  parent: null,
  ancestorSessionIds: [],
  pullRequest: null,
  reviewDocumentId: 'fixture-review-document',
  reviewDocumentCount: 2,
}

function requestUrl(input: RequestInfo | URL) {
  if (typeof input === 'string') return input
  if (input instanceof URL) return input.href
  return input.url
}

let container: HTMLDivElement
let root: Root | null = null

function EditorDocumentView({ session }: { session: Session }) {
  const document = useEditorReviewDocument(session)
  const editorState = document?.state ?? null
  return (
    <>
      <output>
        {editorState === null
          ? 'Loading editor document'
          : editorState.status === 'unavailable'
            ? 'Draft details are unavailable'
            : `${editorState.summary.kind} #${String(editorState.summary.target.issueNumber)}`}
      </output>
      {document?.summaries?.status === 'unavailable' ? (
        <button type="button" onClick={document.retrySummaries}>
          Retry draft list
        </button>
      ) : null}
      <output>{document?.summaries?.status ?? 'loading'}</output>
    </>
  )
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

it('loads the summary for the editor document from the session endpoint', async () => {
  const fetcher = vi.fn<typeof fetch>().mockResolvedValue(
    new Response(
      JSON.stringify([
        {
          id: 'fixture-review-document',
          kind: 'issue_comment',
          target: { issueNumber: 1234 },
          isEditor: true,
          canSave: true,
          canFinish: true,
        },
      ]),
      { status: 200 },
    ),
  )
  vi.stubGlobal('fetch', fetcher)
  root = createRoot(container)

  act(() => root?.render(<EditorDocumentView session={session} />))
  await act(async () => {
    for (let attempt = 0; attempt < 20; attempt += 1) {
      if (container.textContent === 'issue_comment #1234ready') return
      await new Promise((resolve) => window.setTimeout(resolve, 0))
    }
  })

  const snapshot = () => ({
    request: fetcher.mock.calls.map(([input]) => requestUrl(input)),
    document: container.textContent,
  })
  expect(snapshot()).toEqual({
    request: ['/api/sessions/fixture-session/review-documents'],
    document: 'issue_comment #1234ready',
  })
})

it('retries an unavailable draft list', async () => {
  const fetcher = vi
    .fn<typeof fetch>()
    .mockResolvedValueOnce(
      new Response('temporary server failure', { status: 500 }),
    )
    .mockResolvedValueOnce(
      new Response(
        JSON.stringify([
          {
            id: 'fixture-review-document',
            kind: 'issue_comment',
            target: { issueNumber: 1234 },
            isEditor: true,
            canSave: true,
            canFinish: true,
          },
        ]),
        { status: 200, headers: { 'Content-Type': 'application/json' } },
      ),
    )
  vi.stubGlobal('fetch', fetcher)
  root = createRoot(container)

  act(() => root?.render(<EditorDocumentView session={session} />))
  await act(async () => {
    for (let attempt = 0; attempt < 20; attempt += 1) {
      if (container.textContent === 'Draft details are unavailable') return
      if (container.querySelector('button') !== null) return
      await new Promise((resolve) => window.setTimeout(resolve, 0))
    }
  })

  await act(async () => {
    await page.getByRole('button', { name: 'Retry draft list' }).click()
    for (let attempt = 0; attempt < 20; attempt += 1) {
      if (container.textContent === 'issue_comment #1234ready') return
      await new Promise((resolve) => window.setTimeout(resolve, 0))
    }
  })

  const snapshot = () => ({
    request: fetcher.mock.calls.map(([input]) => requestUrl(input)),
    document: container.textContent,
  })
  expect(snapshot()).toEqual({
    request: [
      '/api/sessions/fixture-session/review-documents',
      '/api/sessions/fixture-session/review-documents',
    ],
    document: 'issue_comment #1234ready',
  })
})
