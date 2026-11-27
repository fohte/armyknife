import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import { SessionRow } from '#components/session-row'
import type { Session } from '#types'

let container: HTMLDivElement
let frame: HTMLDivElement
let root: Root | null = null

const session: Session = {
  id: 'sample-session',
  category: 'needs_action',
  engine: 'codex',
  workType: 'development',
  isPaused: false,
  kind: 'Question',
  title: 'Keep the session title visible',
  preview: null,
  elapsedSeconds: 120,
  unread: false,
  task: {
    id: 'sample-task',
    number: 27,
    title: 'Keep the task readable beside long names',
    priority: 1,
  },
  repo: 'example-platform/notification-delivery-service',
  branch: 'feature/preserve-task-details-with-long-identifiers',
  project: 'sample project',
  parent: null,
  ancestorSessionIds: [],
  pullRequest: null,
}

function renderRow(density: 'full' | 'compact') {
  root = createRoot(frame)
  act(() => {
    root?.render(<SessionRow session={session} density={density} />)
  })
}

async function resizeFrame(width: string) {
  await act(async () => {
    frame.style.width = width
    await new Promise<void>((resolve) => {
      requestAnimationFrame(() => {
        requestAnimationFrame(resolve)
      })
    })
  })
}

function snapshot() {
  const repository = container.querySelector<HTMLElement>('.session-repo')
  const repositoryName =
    container.querySelector<HTMLElement>('.session-repo-name')
  const branch = container.querySelector<HTMLElement>('.session-branch')
  const taskTitle = container.querySelector<HTMLElement>('.session-task-title')

  return {
    title: container.querySelector('.session-title')?.textContent,
    task: taskTitle?.textContent,
    taskHasVisibleWidth: (taskTitle?.clientWidth ?? 0) > 0,
    repository: repository?.querySelector('.session-repo-label')?.textContent,
    repositoryTitle: repository?.title,
    repositoryNameOverflowing:
      repositoryName !== null &&
      repositoryName.scrollWidth > repositoryName.clientWidth,
    branch: branch?.textContent,
    branchTitle: branch?.title,
    branchSeparator:
      container.querySelector('.session-branch-separator')?.textContent ?? null,
    branchOverflowing:
      branch !== null && branch.scrollWidth > branch.clientWidth,
  }
}

function rowStates(...states: ReturnType<typeof snapshot>[]) {
  return states
}

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  container = document.createElement('div')
  frame = document.createElement('div')
  frame.style.width = '1100px'
  container.append(frame)
  document.body.append(container)
})

afterEach(() => {
  if (root !== null) act(() => root?.unmount())
  root = null
  container.remove()
  vi.unstubAllGlobals()
})

it('shortens the repository owner before its name as the row narrows', async () => {
  renderRow('compact')
  await resizeFrame('1100px')

  const wideSnapshot = snapshot()

  await resizeFrame('300px')
  const narrowSnapshot = snapshot()

  expect(rowStates(wideSnapshot, narrowSnapshot)).toEqual([
    {
      title: 'Keep the session title visible',
      task: 'Keep the task readable beside long names',
      taskHasVisibleWidth: true,
      repository: 'example-platform/notification-delivery-service',
      repositoryTitle: 'example-platform/notification-delivery-service',
      repositoryNameOverflowing: false,
      branch: 'feature/preserve-task-details-with-long-identifiers',
      branchTitle: 'feature/preserve-task-details-with-long-identifiers',
      branchSeparator: '⑂',
      branchOverflowing: false,
    },
    {
      title: 'Keep the session title visible',
      task: 'Keep the task readable beside long names',
      taskHasVisibleWidth: true,
      repository: '…/notification-delivery-service',
      repositoryTitle: 'example-platform/notification-delivery-service',
      repositoryNameOverflowing: true,
      branch: 'feature/preserve-task-details-with-long-identifiers',
      branchTitle: 'feature/preserve-task-details-with-long-identifiers',
      branchSeparator: '⑂',
      branchOverflowing: true,
    },
  ])
})

it('preserves the task and repository name in a narrow full row', async () => {
  renderRow('full')
  await resizeFrame('1100px')

  const wideSnapshot = snapshot()

  await resizeFrame('300px')
  const narrowSnapshot = snapshot()

  expect(rowStates(wideSnapshot, narrowSnapshot)).toEqual([
    {
      title: 'Keep the session title visible',
      task: 'Keep the task readable beside long names',
      taskHasVisibleWidth: true,
      repository: 'example-platform/notification-delivery-service',
      repositoryTitle: 'example-platform/notification-delivery-service',
      repositoryNameOverflowing: false,
      branch: undefined,
      branchTitle: undefined,
      branchSeparator: null,
      branchOverflowing: false,
    },
    {
      title: 'Keep the session title visible',
      task: 'Keep the task readable beside long names',
      taskHasVisibleWidth: true,
      repository: '…/notification-delivery-service',
      repositoryTitle: 'example-platform/notification-delivery-service',
      repositoryNameOverflowing: true,
      branch: undefined,
      branchTitle: undefined,
      branchSeparator: null,
      branchOverflowing: false,
    },
  ])
})
