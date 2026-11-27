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
  tasks: [
    {
      id: 'sample-task',
      number: 27,
      title: 'Keep the task readable beside long names',
      priority: 1,
    },
    {
      id: 'sample-linked-task',
      number: 28,
      title: 'Document retry handling',
      priority: null,
    },
  ],
  repo: 'example-platform/notification-delivery-service',
  branch: 'feature/preserve-task-details-with-long-identifiers',
  project: 'sample project',
  parent: null,
  ancestorSessionIds: [],
  pullRequest: null,
}

function renderRow(
  density: 'full' | 'compact',
  viewSession: Session = session,
  isMobile = false,
) {
  root = createRoot(frame)
  act(() => {
    root?.render(
      <SessionRow
        session={viewSession}
        density={density}
        isMobile={isMobile}
      />,
    )
  })
}

async function resizeFrame(width: string) {
  await act(async () => {
    frame.style.width = width
    await new Promise<void>((resolve) => {
      requestAnimationFrame(() => {
        requestAnimationFrame(() => {
          resolve()
        })
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
    sessionTitleTooltip: container
      .querySelector('.session-title')
      ?.getAttribute('title'),
    task: taskTitle?.textContent,
    taskHasVisibleWidth: (taskTitle?.clientWidth ?? 0) > 0,
    taskTitleOverflowing:
      taskTitle !== null && taskTitle.scrollWidth > taskTitle.clientWidth,
    taskTooltip: container
      .querySelector('.session-task')
      ?.getAttribute('title'),
    repository: repository?.querySelector('.session-repo-label')?.textContent,
    repositoryTitle: repository?.title,
    repositoryNameOverflowing:
      repositoryName !== null &&
      repositoryName.scrollWidth > repositoryName.clientWidth,
    repositoryNameTextOverflow:
      repositoryName === null
        ? null
        : getComputedStyle(repositoryName).textOverflow,
    branch: branch?.textContent,
    branchTitle: branch?.title,
    branchSeparator:
      container.querySelector('.session-branch-separator')?.textContent ?? null,
    branchOverflowing:
      branch !== null && branch.scrollWidth > branch.clientWidth,
    branchTextOverflow:
      branch === null ? null : getComputedStyle(branch).textOverflow,
  }
}

async function resizeRowSnapshots(
  density: 'full' | 'compact',
  viewSession: Session = session,
  narrowWidth = '300px',
  isMobile = false,
) {
  renderRow(density, viewSession, isMobile)
  await resizeFrame('1100px')
  const wideSnapshot = snapshot()
  await resizeFrame(narrowWidth)
  const narrowSnapshot = snapshot()
  await resizeFrame('1100px')
  return [wideSnapshot, narrowSnapshot, snapshot()]
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
  expect(await resizeRowSnapshots('compact')).toEqual([
    {
      title: 'Keep the session title visible',
      sessionTitleTooltip: 'Keep the session title visible',
      task: 'Keep the task readable beside long names +1',
      taskHasVisibleWidth: true,
      taskTitleOverflowing: false,
      taskTooltip: 'P1 #27 Keep the task readable beside long names +1',
      repository: 'example-platform/notification-delivery-service',
      repositoryTitle: 'example-platform/notification-delivery-service',
      repositoryNameOverflowing: false,
      repositoryNameTextOverflow: 'ellipsis',
      branch: 'feature/preserve-task-details-with-long-identifiers',
      branchTitle: 'feature/preserve-task-details-with-long-identifiers',
      branchSeparator: '⑂',
      branchOverflowing: false,
      branchTextOverflow: 'ellipsis',
    },
    {
      title: 'Keep the session title visible',
      sessionTitleTooltip: 'Keep the session title visible',
      task: 'Keep the task readable beside long names +1',
      taskHasVisibleWidth: true,
      taskTitleOverflowing: true,
      taskTooltip: 'P1 #27 Keep the task readable beside long names +1',
      repository: '…/notification-delivery-service',
      repositoryTitle: 'example-platform/notification-delivery-service',
      repositoryNameOverflowing: false,
      repositoryNameTextOverflow: 'ellipsis',
      branch: 'feature/preserve-task-details-with-long-identifiers',
      branchTitle: 'feature/preserve-task-details-with-long-identifiers',
      branchSeparator: '⑂',
      branchOverflowing: true,
      branchTextOverflow: 'ellipsis',
    },
    {
      title: 'Keep the session title visible',
      sessionTitleTooltip: 'Keep the session title visible',
      task: 'Keep the task readable beside long names +1',
      taskHasVisibleWidth: true,
      taskTitleOverflowing: false,
      taskTooltip: 'P1 #27 Keep the task readable beside long names +1',
      repository: 'example-platform/notification-delivery-service',
      repositoryTitle: 'example-platform/notification-delivery-service',
      repositoryNameOverflowing: false,
      repositoryNameTextOverflow: 'ellipsis',
      branch: 'feature/preserve-task-details-with-long-identifiers',
      branchTitle: 'feature/preserve-task-details-with-long-identifiers',
      branchSeparator: '⑂',
      branchOverflowing: false,
      branchTextOverflow: 'ellipsis',
    },
  ])
})

const fullRowSession: Session = {
  ...session,
  task: {
    id: 'sample-task',
    number: 27,
    title:
      'Keep the task title readable while repository and branch names are long',
    priority: 1,
  },
  repo: 'sample-contributor-automation-team/notification-delivery-service',
}

const longFullRowTaskTitle = fullRowSession.task?.title ?? ''

const fullRowSnapshots = [
  {
    title: 'Keep the session title visible',
    sessionTitleTooltip: 'Keep the session title visible',
    task: longFullRowTaskTitle,
    taskHasVisibleWidth: true,
    taskTitleOverflowing: false,
    taskTooltip: `P1 #27 ${longFullRowTaskTitle}`,
    repository:
      'sample-contributor-automation-team/notification-delivery-service',
    repositoryTitle:
      'sample-contributor-automation-team/notification-delivery-service',
    repositoryNameOverflowing: false,
    repositoryNameTextOverflow: 'ellipsis',
    branch: undefined,
    branchTitle: undefined,
    branchSeparator: null,
    branchOverflowing: false,
    branchTextOverflow: null,
  },
  {
    title: 'Keep the session title visible',
    sessionTitleTooltip: 'Keep the session title visible',
    task: longFullRowTaskTitle,
    taskHasVisibleWidth: true,
    taskTitleOverflowing: true,
    taskTooltip: `P1 #27 ${longFullRowTaskTitle}`,
    repository: '…/notification-delivery-service',
    repositoryTitle:
      'sample-contributor-automation-team/notification-delivery-service',
    repositoryNameOverflowing: true,
    repositoryNameTextOverflow: 'ellipsis',
    branch: undefined,
    branchTitle: undefined,
    branchSeparator: null,
    branchOverflowing: false,
    branchTextOverflow: null,
  },
  {
    title: 'Keep the session title visible',
    sessionTitleTooltip: 'Keep the session title visible',
    task: longFullRowTaskTitle,
    taskHasVisibleWidth: true,
    taskTitleOverflowing: false,
    taskTooltip: `P1 #27 ${longFullRowTaskTitle}`,
    repository:
      'sample-contributor-automation-team/notification-delivery-service',
    repositoryTitle:
      'sample-contributor-automation-team/notification-delivery-service',
    repositoryNameOverflowing: false,
    repositoryNameTextOverflow: 'ellipsis',
    branch: undefined,
    branchTitle: undefined,
    branchSeparator: null,
    branchOverflowing: false,
    branchTextOverflow: null,
  },
]

it.each([
  { rowType: 'inbox', isMobile: false, width: '460px' },
  { rowType: 'iPhone inbox', isMobile: true, width: '390px' },
])(
  'keeps the task title readable in a narrow full $rowType row',
  async ({ isMobile, width }) => {
    expect(
      await resizeRowSnapshots('full', fullRowSession, width, isMobile),
    ).toEqual(fullRowSnapshots)
  },
)

it('truncates a long task title only after the repository reaches its minimum width', async () => {
  renderRow('full', fullRowSession)
  await resizeFrame('320px')

  const repository = container.querySelector<HTMLElement>('.session-repo')
  const repositoryName =
    container.querySelector<HTMLElement>('.session-repo-name')
  const repositoryOwner = container.querySelector<HTMLElement>(
    '.session-repo-owner',
  )
  const repositoryOwnerSeparator = container.querySelector<HTMLElement>(
    '.session-repo-owner-separator',
  )
  const taskTitle = container.querySelector<HTMLElement>('.session-task-title')
  const task = container.querySelector<HTMLElement>('.session-task')
  if (
    repository === null ||
    repositoryName === null ||
    repositoryOwner === null ||
    repositoryOwnerSeparator === null ||
    taskTitle === null ||
    task === null
  ) {
    throw new Error('The full row details were not rendered')
  }

  const repositoryMinimum = document.createElement('span')
  repositoryMinimum.style.position = 'fixed'
  repositoryMinimum.style.width = '10ch'
  repositoryMinimum.style.font = getComputedStyle(repository).font
  document.body.append(repositoryMinimum)
  const repositoryMinimumWidth = repositoryMinimum.getBoundingClientRect().width
  repositoryMinimum.remove()

  const shortenedRepositoryMinimum = document.createElement('span')
  shortenedRepositoryMinimum.style.position = 'fixed'
  shortenedRepositoryMinimum.style.width = '12ch'
  shortenedRepositoryMinimum.style.font = getComputedStyle(repository).font
  document.body.append(shortenedRepositoryMinimum)
  const shortenedRepositoryMinimumWidth =
    shortenedRepositoryMinimum.getBoundingClientRect().width
  shortenedRepositoryMinimum.remove()

  const rowState = () => ({
    taskTitleOverflowing: taskTitle.scrollWidth > taskTitle.clientWidth,
    taskTooltip: task.title,
    repositoryNameAtMinimumWidth:
      Math.abs(
        repositoryName.getBoundingClientRect().width - repositoryMinimumWidth,
      ) <= 1,
    ownerMarkerVisible: repositoryOwner.getBoundingClientRect().width > 0,
    ownerSeparatorVisible:
      repositoryOwnerSeparator.getBoundingClientRect().width > 0,
    repositoryAtShortenedMinimumWidth:
      Math.abs(
        repository.getBoundingClientRect().width -
          shortenedRepositoryMinimumWidth,
      ) <= 1,
  })

  expect(rowState()).toEqual({
    taskTitleOverflowing: true,
    taskTooltip: `P1 #27 ${longFullRowTaskTitle}`,
    repositoryNameAtMinimumWidth: true,
    ownerMarkerVisible: true,
    ownerSeparatorVisible: true,
    repositoryAtShortenedMinimumWidth: true,
  })
})

it('shortens a clipped owner within the one-pixel repository fit tolerance', async () => {
  const clippedOwnerSession: Session = {
    ...session,
    engine: null,
    task: null,
    tasks: [],
    repo: 'example-team/service',
  }
  renderRow('full', clippedOwnerSession)
  await resizeFrame('1100px')

  const repository = container.querySelector<HTMLElement>('.session-repo')
  const repositoryMeasure = container.querySelector<HTMLElement>(
    '.session-repo-measure',
  )
  const repositoryOwner = container.querySelector<HTMLElement>(
    '.session-repo-owner',
  )
  const repositoryOwnerSeparator = container.querySelector<HTMLElement>(
    '.session-repo-owner-separator',
  )
  const metaLine = repository?.parentElement ?? null
  if (
    repository === null ||
    repositoryMeasure === null ||
    repositoryOwner === null ||
    repositoryOwnerSeparator === null ||
    metaLine === null
  ) {
    throw new Error('The full row repository details were not rendered')
  }

  const metaLineStyle = getComputedStyle(metaLine)
  const metaSiblings = Array.from(metaLine.children).filter(
    (child): child is HTMLElement =>
      child instanceof HTMLElement && child !== repository,
  )
  const metaSiblingWidth = metaSiblings.reduce(
    (width, sibling) =>
      width + Math.max(sibling.clientWidth, sibling.scrollWidth),
    0,
  )
  const parentPadding =
    (Number.parseFloat(metaLineStyle.paddingLeft) || 0) +
    (Number.parseFloat(metaLineStyle.paddingRight) || 0)
  const columnGap = Number.parseFloat(metaLineStyle.columnGap) || 0
  const availableWidth = () =>
    metaLine.clientWidth -
    parentPadding -
    metaSiblingWidth -
    columnGap * metaSiblings.length
  const repositoryWidth = repositoryMeasure.getBoundingClientRect().width
  const targetAvailableWidth = Math.ceil(repositoryWidth) - 1
  const targetFrameWidth =
    Number.parseFloat(frame.style.width) +
    targetAvailableWidth -
    availableWidth()

  repositoryOwner.style.width = '1ch'
  const ownerWasClipped =
    repositoryOwner.scrollWidth > repositoryOwner.clientWidth
  await resizeFrame(`${String(targetFrameWidth)}px`)

  const rowSnapshot = () => ({
    ownerWasClipped,
    repositoryIsWithinToleranceButDoesNotFit:
      repositoryWidth > availableWidth() &&
      repositoryWidth <= availableWidth() + 1,
    repository: repository.querySelector('.session-repo-label')?.textContent,
    repositoryTitle: repository.title,
    ownerMarkerVisible:
      repositoryOwner.textContent === '…' &&
      repositoryOwner.getBoundingClientRect().width > 0,
    ownerSeparatorVisible:
      repositoryOwnerSeparator.getBoundingClientRect().width > 0,
  })
  expect(rowSnapshot()).toEqual({
    ownerWasClipped: true,
    repositoryIsWithinToleranceButDoesNotFit: true,
    repository: '…/service',
    repositoryTitle: 'example-team/service',
    ownerMarkerVisible: true,
    ownerSeparatorVisible: true,
  })
})

it('shortens the owner before a repository name ellipsizes near the compact-row threshold', async () => {
  renderRow('compact')
  await resizeFrame('1100px')

  const repositoryLine =
    container.querySelector<HTMLElement>('.session-repo-line')
  const repositoryMeasure = container.querySelector<HTMLElement>(
    '.session-repo-measure',
  )
  const branch = container.querySelector<HTMLElement>('.session-branch')
  const separator = container.querySelector<HTMLElement>(
    '.session-branch-separator',
  )
  if (
    repositoryLine === null ||
    repositoryMeasure === null ||
    branch === null ||
    separator === null
  ) {
    throw new Error('The compact row details were not rendered')
  }

  const lineStyle = getComputedStyle(repositoryLine)
  const desiredRepositoryWidth =
    repositoryMeasure.getBoundingClientRect().width - 5
  const reservedWidth =
    (Number.parseFloat(getComputedStyle(branch).minWidth) || 0) +
    separator.getBoundingClientRect().width +
    (Number.parseFloat(lineStyle.columnGap) || 0) * 2
  const desiredLineWidth =
    desiredRepositoryWidth +
    reservedWidth +
    (Number.parseFloat(lineStyle.paddingLeft) || 0) +
    (Number.parseFloat(lineStyle.paddingRight) || 0)
  const targetFrameWidth =
    Number.parseFloat(frame.style.width) +
    desiredLineWidth -
    repositoryLine.clientWidth

  await resizeFrame(`${String(targetFrameWidth)}px`)

  const repository = container.querySelector<HTMLElement>('.session-repo')
  const repositoryName =
    container.querySelector<HTMLElement>('.session-repo-name')
  const rowSnapshot = () => {
    if (repository === null || repositoryName === null) {
      throw new Error('The repository label was not rendered')
    }
    return {
      repository: repository.querySelector('.session-repo-label')?.textContent,
      repositoryTitle: repository.title,
      repositoryNameOverflowing:
        repositoryName.scrollWidth > repositoryName.clientWidth,
      branch: branch.textContent,
      branchTitle: branch.title,
    }
  }

  expect(rowSnapshot()).toEqual({
    repository: '…/notification-delivery-service',
    repositoryTitle: 'example-platform/notification-delivery-service',
    repositoryNameOverflowing: false,
    branch: 'feature/preserve-task-details-with-long-identifiers',
    branchTitle: 'feature/preserve-task-details-with-long-identifiers',
  })
})

it('does not reserve repository width for the blank-repository placeholder', async () => {
  renderRow('compact', {
    ...session,
    repo: ' ',
    branch: 'feature/repo-placeholder-space',
  })
  await resizeFrame('330px')

  const repository = container.querySelector<HTMLElement>('.session-repo')
  const repositoryLabel = container.querySelector<HTMLElement>(
    '.session-repo-label',
  )
  const repositoryName =
    container.querySelector<HTMLElement>('.session-repo-name')
  const branch = container.querySelector<HTMLElement>('.session-branch')
  const rowSnapshot = () => ({
    repository: repositoryLabel?.textContent,
    repositoryTitle: repository?.title,
    repositoryMinimumWidth:
      repository === null ? null : getComputedStyle(repository).minWidth,
    labelMinimumWidth:
      repositoryLabel === null
        ? null
        : getComputedStyle(repositoryLabel).minWidth,
    nameMinimumWidth:
      repositoryName === null
        ? null
        : getComputedStyle(repositoryName).minWidth,
    branch: branch?.textContent,
    branchTitle: branch?.title,
  })

  expect(rowSnapshot()).toEqual({
    repository: '—',
    repositoryTitle: '—',
    repositoryMinimumWidth: '0px',
    labelMinimumWidth: '0px',
    nameMinimumWidth: '0px',
    branch: 'feature/repo-placeholder-space',
    branchTitle: 'feature/repo-placeholder-space',
  })
})
