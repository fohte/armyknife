import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import {
  AsyncActionFeedbackProvider,
  type AsyncActionOutcome,
  useAsyncActionFeedback,
} from '#async-action-feedback'

let container: HTMLDivElement
let root: Root | null = null

function FeedbackHarness({ run }: { run: () => Promise<AsyncActionOutcome> }) {
  const feedback = useAsyncActionFeedback()
  return (
    <>
      <button
        type="button"
        onClick={() => {
          feedback.start({
            kind: 'merge',
            sessionId: 'sample-session',
            sessionTitle: 'Sample session',
            pullRequestNumber: 42,
            run,
          })
        }}
      >
        Start merge
      </button>
      <button
        type="button"
        onClick={() => {
          feedback.openActivity()
        }}
      >
        Show activity
      </button>
      <output>
        {JSON.stringify({
          actions: feedback.actions.map(({ status }) => status),
          toastIds: feedback.toastIds,
        })}
      </output>
    </>
  )
}

function textContent(selector: string) {
  return container.querySelector(selector)?.textContent ?? null
}

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  vi.setSystemTime(new Date(2026, 10, 20, 14, 13))
  container = document.createElement('div')
  document.body.append(container)
  root = createRoot(container)
})

afterEach(() => {
  if (root !== null) act(() => root?.unmount())
  root = null
  container.remove()
  vi.unstubAllGlobals()
  vi.useRealTimers()
})

it('records a merge while it runs, then keeps its result in Activity', async () => {
  let completeMerge: (() => void) | undefined
  const run = vi.fn(
    () =>
      new Promise<AsyncActionOutcome>((resolve) => {
        completeMerge = () => {
          resolve({ status: 'merged' })
        }
      }),
  )
  act(() => {
    root?.render(
      <AsyncActionFeedbackProvider>
        <FeedbackHarness run={run} />
      </AsyncActionFeedbackProvider>,
    )
  })

  act(() => {
    container.querySelector<HTMLButtonElement>('button')?.click()
  })
  const runningState = () => ({
    runCalls: run.mock.calls.length,
    state: textContent('output'),
    toast: (textContent('.async-action-toast-stack') ?? '').trim(),
  })
  expect(runningState()).toEqual({
    runCalls: 1,
    state: '{"actions":["merging"],"toastIds":[0]}',
    toast: 'Merging PR #42Sample session',
  })

  await act(async () => {
    completeMerge?.()
    await Promise.resolve()
  })
  act(() => {
    Array.from(container.querySelectorAll<HTMLButtonElement>('button'))
      .find((button) => button.textContent === 'Show activity')
      ?.click()
  })

  const resultState = () => ({
    state: textContent('output'),
    toast: (textContent('.async-action-toast-stack') ?? '').trim(),
    activity: (textContent('[role="dialog"]') ?? '')
      .replaceAll(/\s+/g, ' ')
      .trim(),
  })
  expect(resultState()).toEqual({
    state: '{"actions":["merged"],"toastIds":[0]}',
    toast: 'Merged PR #42Sample session',
    activity: 'ActivityClear finishedMerged PR #42Sample session14:13Close',
  })
})

it('keeps merge failures available for retry and opens their Activity details', async () => {
  const run = vi
    .fn<() => Promise<AsyncActionOutcome>>()
    .mockResolvedValueOnce({
      status: 'mergeFailed',
      error: 'The request timed out.',
    })
    .mockResolvedValueOnce({ status: 'merged' })
  act(() => {
    root?.render(
      <AsyncActionFeedbackProvider>
        <FeedbackHarness run={run} />
      </AsyncActionFeedbackProvider>,
    )
  })

  await act(async () => {
    container.querySelector<HTMLButtonElement>('button')?.click()
    await Promise.resolve()
  })
  const failureSnapshot = () => ({
    calls: run.mock.calls.length,
    alert: (textContent('[role="alert"]') ?? '').replaceAll(/\s+/g, ' ').trim(),
  })
  expect(failureSnapshot()).toEqual({
    calls: 1,
    alert:
      'Merge failed · PR #42Sample sessionThe request timed out.RetryDetails',
  })

  await act(async () => {
    Array.from(container.querySelectorAll<HTMLButtonElement>('button'))
      .find((button) => button.textContent === 'Details')
      ?.click()
    await Promise.resolve()
  })
  expect(
    (textContent('.async-action-activity-list') ?? '')
      .replaceAll(/\s+/g, ' ')
      .trim(),
  ).toEqual(
    'Merge failed · PR #42Sample sessionThe request timed out.Retry14:13',
  )

  await act(async () => {
    Array.from(
      container.querySelectorAll<HTMLButtonElement>(
        '.async-action-toast-stack button',
      ),
    )
      .find((button) => button.textContent === 'Retry')
      ?.click()
    await Promise.resolve()
  })
  const retrySnapshot = () => ({
    calls: run.mock.calls.length,
    result: (textContent('.async-action-toast-stack') ?? '').trim(),
  })
  expect(retrySnapshot()).toEqual({
    calls: 2,
    result: 'Merged PR #42Sample session',
  })
})
