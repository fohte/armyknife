import { act, Component, type ReactNode } from 'react'
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
  const feedbackResult = useAsyncActionFeedback()
  const feedback = feedbackResult.match(
    (value) => value,
    () => null,
  )
  return (
    <>
      <button
        type="button"
        onClick={() => {
          feedback?.start({
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
        {feedbackResult.match(
          ({ actions, toastIds }) =>
            JSON.stringify({
              actions: actions.map(({ status }) => status),
              toastIds,
            }),
          (error) => `provider error: ${error.message}`,
        )}
      </output>
    </>
  )
}

function MissingProviderHarness() {
  const feedback = useAsyncActionFeedback()
  return (
    <output>
      {feedback.match(
        () => 'provider available',
        (error) => `provider error: ${error.message}`,
      )}
    </output>
  )
}

class TestErrorBoundary extends Component<
  { children: ReactNode },
  { message: string | null }
> {
  override state: { message: string | null } = { message: null }

  static getDerivedStateFromError(error: unknown) {
    return {
      message: error instanceof Error ? error.message : String(error),
    }
  }

  override render() {
    if (this.state.message !== null) {
      return <output>thrown error: {this.state.message}</output>
    }
    return this.props.children
  }
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

it('returns a failure result when used outside its provider', () => {
  act(() => {
    root?.render(
      <TestErrorBoundary>
        <MissingProviderHarness />
      </TestErrorBoundary>,
    )
  })

  expect(textContent('output')).toEqual(
    'provider error: useAsyncActionFeedback must be used inside AsyncActionFeedbackProvider',
  )
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
