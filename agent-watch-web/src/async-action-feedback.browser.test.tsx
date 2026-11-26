import { act, useState } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { userEvent } from 'vitest/browser'

import {
  AsyncActionFeedbackProvider,
  type AsyncActionOutcome,
  useAsyncActionStarter,
  useOptionalAsyncActionFeedback,
} from '#async-action-feedback'

let container: HTMLDivElement
let root: Root | null = null

function FeedbackHarness({ run }: { run: () => Promise<AsyncActionOutcome> }) {
  const feedback = useOptionalAsyncActionFeedback()
  const startAsyncAction = useAsyncActionStarter(() => {})
  return (
    <>
      <button
        type="button"
        onClick={() => {
          startAsyncAction({
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
          feedback?.openActivity()
        }}
      >
        Show activity
      </button>
      <output>
        {JSON.stringify({
          actions: feedback?.actions.map(({ status }) => status),
          toastIds: feedback?.toastIds,
        })}
      </output>
    </>
  )
}

function MissingProviderHarness({
  run,
}: {
  run: () => Promise<AsyncActionOutcome>
}) {
  const [error, setError] = useState<string | null>(null)
  const startAsyncAction = useAsyncActionStarter((feedbackError) => {
    setError(feedbackError.message)
  })
  return (
    <>
      <button
        type="button"
        onClick={() => {
          startAsyncAction({
            kind: 'merge',
            sessionId: 'sample-session',
            sessionTitle: 'Sample session',
            run,
          })
        }}
      >
        Start action
      </button>
      <output>{error}</output>
    </>
  )
}

function RewindFeedbackHarness({
  run,
}: {
  run: () => Promise<AsyncActionOutcome>
}) {
  const feedback = useOptionalAsyncActionFeedback()
  return (
    <>
      <button
        type="button"
        onClick={() => {
          feedback?.start({
            kind: 'rewind',
            sessionId: 'example-session',
            sessionTitle: 'Review the example request',
            run,
          })
        }}
      >
        Start rewind
      </button>
      <output>
        {JSON.stringify(feedback?.actions.map(({ status }) => status))}
      </output>
    </>
  )
}

function CommandFeedbackHarness({
  run,
}: {
  run: () => Promise<AsyncActionOutcome>
}) {
  const feedback = useOptionalAsyncActionFeedback()
  const startAsyncAction = useAsyncActionStarter(() => undefined)
  return (
    <>
      <button
        type="button"
        onClick={() => {
          startAsyncAction({
            kind: 'command',
            sessionId: 'example-session',
            sessionTitle: 'Example session',
            command: "printf 'sample output'",
            run,
          })
        }}
      >
        Start command
      </button>
      <output>
        {JSON.stringify(feedback?.actions.map(({ status }) => status))}
      </output>
    </>
  )
}

function ReviewDocumentActionHarness({
  run,
  label,
}: {
  run: () => Promise<AsyncActionOutcome>
  label: string
}) {
  const feedback = useOptionalAsyncActionFeedback()
  return (
    <>
      <button
        type="button"
        onClick={() => {
          feedback?.start({
            kind: 'review_document',
            reviewDocumentAction: label,
            sessionId: 'example-session',
            sessionTitle: 'Review the example comment',
            run,
          })
        }}
      >
        {label}
      </button>
      <output>
        {JSON.stringify(feedback?.actions.map(({ status }) => status))}
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

it('runs the action and reports a provider error when used outside its provider', () => {
  const run = vi.fn(() => Promise.resolve({ status: 'merged' as const }))
  act(() => {
    root?.render(<MissingProviderHarness run={run} />)
  })
  act(() => {
    container.querySelector<HTMLButtonElement>('button')?.click()
  })

  const snapshot = () => ({
    providerError: textContent('output'),
    runCalls: run.mock.calls.length,
  })
  expect(snapshot()).toEqual({
    providerError:
      'useAsyncActionFeedback must be used inside AsyncActionFeedbackProvider',
    runCalls: 1,
  })
})

it('keeps a command in the toast while it runs and reports its success', async () => {
  let finishCommand: (outcome: AsyncActionOutcome) => void = () => undefined
  const run = vi.fn(
    () =>
      new Promise<AsyncActionOutcome>((resolve) => {
        finishCommand = resolve
      }),
  )
  act(() => {
    root?.render(
      <AsyncActionFeedbackProvider>
        <CommandFeedbackHarness run={run} />
      </AsyncActionFeedbackProvider>,
    )
  })

  const startButton = container.querySelector('button')
  if (!(startButton instanceof HTMLButtonElement)) {
    throw new Error('Expected the start command button')
  }
  await act(async () => {
    await userEvent.click(startButton)
  })
  const running = {
    status: textContent('output'),
    toast: textContent('.async-action-toast-stack'),
  }
  await act(async () => {
    finishCommand({ status: 'ranCommand' })
    await Promise.resolve()
  })

  const snapshot = () => ({
    running,
    completedStatus: textContent('output'),
    completedToast: textContent('.async-action-toast-stack'),
  })
  expect(snapshot()).toEqual({
    running: {
      status: '["runningCommand"]',
      toast: "Running commandprintf 'sample output'",
    },
    completedStatus: '["ranCommand"]',
    completedToast: "Ran commandprintf 'sample output'",
  })
})

it('records a merge while it runs, then keeps its result in Activity', async () => {
  vi.useFakeTimers()
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

  await act(async () => {
    await vi.advanceTimersByTimeAsync(4_999)
  })
  const beforeExpirySnapshot = () => ({
    toast: textContent('.async-action-toast-stack'),
    activity: textContent('.async-action-activity-list'),
  })
  expect(beforeExpirySnapshot()).toEqual({
    toast: 'Merged PR #42Sample session',
    activity: 'Merged PR #42Sample session14:13',
  })

  await act(async () => {
    await vi.advanceTimersByTimeAsync(1)
  })
  const expiredSnapshot = () => ({
    state: textContent('output'),
    toast:
      textContent('.async-action-toast-stack') === ''
        ? null
        : textContent('.async-action-toast-stack'),
    activity: textContent('.async-action-activity-list'),
  })
  expect(expiredSnapshot()).toEqual({
    state: '{"actions":["merged"],"toastIds":[]}',
    toast: null,
    activity: 'Merged PR #42Sample session14:13',
  })
})

it('shows rewind failure in the action toast and status', async () => {
  const run = vi.fn(() =>
    Promise.resolve({
      status: 'rewindFailed' as const,
      error: 'The example session could not be rewound.',
    }),
  )
  act(() => {
    root?.render(
      <AsyncActionFeedbackProvider>
        <RewindFeedbackHarness run={run} />
      </AsyncActionFeedbackProvider>,
    )
  })

  const startButton = container.querySelector('button')
  if (!(startButton instanceof HTMLButtonElement)) {
    throw new Error('Expected the start rewind button')
  }
  await act(async () => {
    await userEvent.click(startButton)
    await Promise.resolve()
  })

  const snapshot = () => ({
    status: textContent('output'),
    toast: textContent('.async-action-toast-stack'),
    failureRole: container
      .querySelector('.async-action-item')
      ?.getAttribute('role'),
  })
  expect(snapshot()).toEqual({
    status: '["rewindFailed"]',
    toast:
      'Rewind failedReview the example requestThe example session could not be rewound.',
    failureRole: 'alert',
  })
})

it('reports a review document action through failure, retry, and success', async () => {
  const completions: Array<(outcome: AsyncActionOutcome) => void> = []
  const run = vi.fn(
    () =>
      new Promise<AsyncActionOutcome>((resolve) => {
        completions.push(resolve)
      }),
  )
  act(() => {
    root?.render(
      <AsyncActionFeedbackProvider>
        <ReviewDocumentActionHarness run={run} label="Update PR #73" />
      </AsyncActionFeedbackProvider>,
    )
  })

  const startButton = container.querySelector('button')
  if (!(startButton instanceof HTMLButtonElement)) {
    throw new Error('Expected the review document action button')
  }
  await act(async () => {
    await userEvent.click(startButton)
  })
  const running = {
    status: textContent('output'),
    toast: textContent('.async-action-toast-stack'),
  }
  await act(async () => {
    completions[0]?.({
      status: 'reviewDocumentActionFailed',
      error: 'The example pull request could not be updated.',
    })
    await Promise.resolve()
  })
  const failed = {
    status: textContent('output'),
    toast: textContent('.async-action-toast-stack'),
    error: textContent('.async-action-error'),
    retry: container.querySelector('.async-action-controls button')
      ?.textContent,
  }
  const retryButton = Array.from(
    container.querySelectorAll<HTMLButtonElement>('button'),
  ).find((button) => button.textContent === 'Retry')
  if (retryButton === undefined) {
    throw new Error('Expected the failed action retry button')
  }
  await act(async () => {
    await userEvent.click(retryButton)
  })
  const retrying = {
    status: textContent('output'),
    toast: textContent('.async-action-toast-stack'),
  }
  await act(async () => {
    completions[1]?.({ status: 'reviewDocumentActionSucceeded' })
    await Promise.resolve()
  })

  const snapshot = () => ({
    running,
    failed,
    retrying,
    succeeded: textContent('output'),
    calls: run.mock.calls.length,
  })
  expect(snapshot()).toEqual({
    running: {
      status: '["reviewDocumentActionRunning"]',
      toast: 'Update PR #73 in progressReview the example comment',
    },
    failed: {
      status: '["reviewDocumentActionFailed"]',
      toast:
        'Update PR #73 failedReview the example commentThe example pull request could not be updated.Retry',
      error: 'The example pull request could not be updated.',
      retry: 'Retry',
    },
    retrying: {
      status: '["reviewDocumentActionRunning"]',
      toast: 'Update PR #73 in progressReview the example comment',
    },
    succeeded: '["reviewDocumentActionSucceeded"]',
    calls: 2,
  })
})

it('clears finished Activity entries while a pending action remains', async () => {
  const completions: Array<() => void> = []
  const run = vi.fn(
    () =>
      new Promise<AsyncActionOutcome>((resolve) => {
        completions.push(() => {
          resolve({ status: 'merged' })
        })
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
    container.querySelector<HTMLButtonElement>('button')?.click()
  })
  await act(async () => {
    completions[0]?.()
    await Promise.resolve()
  })
  act(() => {
    Array.from(container.querySelectorAll<HTMLButtonElement>('button'))
      .find((button) => button.textContent === 'Show activity')
      ?.click()
  })
  act(() => {
    Array.from(container.querySelectorAll<HTMLButtonElement>('button'))
      .find((button) => button.textContent === 'Clear finished')
      ?.click()
  })

  const clearSnapshot = () => ({
    state: textContent('output'),
    toast: textContent('.async-action-toast-stack'),
    activity: textContent('.async-action-activity-list'),
  })
  expect(clearSnapshot()).toEqual({
    state: '{"actions":["merging"],"toastIds":[1]}',
    toast: 'Merging PR #42Sample session',
    activity: 'Merging PR #42Sample session14:13',
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
