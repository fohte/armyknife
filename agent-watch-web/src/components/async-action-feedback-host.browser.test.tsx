import { act, useState } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import type { AsyncAction } from '#async-action-feedback'
import { AsyncActionFeedbackHost } from '#components/async-action-feedback-host'

const failedMerge: AsyncAction = {
  id: 1,
  sessionId: 'example-session',
  sessionTitle: 'Example session',
  kind: 'merge',
  status: 'mergeFailed',
  pullRequestNumber: 42,
  pullRequestOwner: 'example-org',
  pullRequestRepo: 'example-repo',
  error: 'The pull request head changed.',
  createdAt: 1,
}

function ActivityRetryHarness({ calls }: { calls: string[] }) {
  const [activityOpen, setActivityOpen] = useState(true)
  return (
    <AsyncActionFeedbackHost
      actions={[failedMerge]}
      toastIds={[]}
      activityOpen={activityOpen}
      selectedActionId={failedMerge.id}
      retry={() => {
        calls.push('retry')
      }}
      dismissToast={() => undefined}
      clearFinished={() => undefined}
      openActivity={() => undefined}
      closeActivity={() => {
        calls.push('close')
        setActivityOpen(false)
      }}
      closeWithWorktree={() => undefined}
      openInTerminal={() => undefined}
    />
  )
}

let root: Root | null = null
let container: HTMLDivElement

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
})

afterEach(() => {
  act(() => {
    root?.unmount()
  })
  root = null
  document.body.innerHTML = ''
})

it('closes Activity before retrying a failed action', () => {
  const calls: string[] = []
  container = document.createElement('div')
  document.body.append(container)
  root = createRoot(container)
  act(() => {
    root?.render(<ActivityRetryHarness calls={calls} />)
  })

  act(() => {
    container
      .querySelector<HTMLButtonElement>('.async-action-activity-list button')
      ?.click()
  })

  const snapshot = () => ({
    calls,
    activityOpen: container.querySelector('[aria-label="Activity"]') !== null,
  })
  expect(snapshot()).toEqual({
    calls: ['close', 'retry'],
    activityOpen: false,
  })
})
