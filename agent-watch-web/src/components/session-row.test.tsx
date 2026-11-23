import '#components/session-tree-panel.css'

import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import { SessionRow } from '#components/session-row'
import type { Session } from '#types'

let container: HTMLDivElement
let root: Root | null = null

const session: Session = {
  id: 'sample-session',
  engine: 'codex',
  workType: 'development',
  category: 'needs_action',
  isPaused: false,
  kind: 'Question',
  title: 'Review the retry policy',
  preview: 'Should retries stop after three attempts?',
  elapsedSeconds: 125,
  unread: true,
  task: {
    id: 'sample-task',
    number: 17,
    title: 'Improve request reliability',
    priority: 2,
  },
  tasks: [
    {
      id: 'sample-task',
      number: 17,
      title: 'Improve request reliability',
      priority: 2,
    },
    {
      id: 'sample-child-task',
      number: 18,
      title: 'Document the retry policy',
      priority: null,
    },
  ],
  repo: 'sample/service',
  project: 'sample project',
  parent: null,
  ancestorSessionIds: [],
  pullRequest: null,
}

function renderRow(
  density: 'full' | 'compact' | 'compact-phone',
  viewSession: Session = session,
  isMobile = false,
  inTree = false,
) {
  if (root !== null) act(() => root?.unmount())
  root = createRoot(container)
  act(() => {
    root?.render(
      inTree ? (
        <div className="session-tree-row">
          <SessionRow
            session={viewSession}
            density={density}
            isMobile={isMobile}
          />
        </div>
      ) : (
        <SessionRow
          session={viewSession}
          density={density}
          isMobile={isMobile}
        />
      ),
    )
  })
}

function horizontalPaddingSnapshot() {
  return Array.from(
    container.querySelectorAll<HTMLElement>('[data-session-row-content]'),
    (row) => {
      const { paddingLeft, paddingRight } = getComputedStyle(row)
      return { paddingLeft, paddingRight }
    },
  )
}

function snapshot() {
  const row = container.querySelector<HTMLElement>('[data-session-row-content]')
  const role = container.querySelector<HTMLElement>('.session-role-icon')
  const task = container.querySelector<HTMLElement>('.session-task')

  return {
    density:
      row?.classList.contains('density-full') === true
        ? 'full'
        : row?.classList.contains('density-compact-phone') === true
          ? 'compact-phone'
          : 'compact',
    mobile: row?.classList.contains('is-mobile') ?? false,
    title: container.querySelector('.session-title')?.textContent,
    age: container.querySelector('.session-age')?.textContent,
    preview: container.querySelector('.session-preview')?.textContent ?? null,
    role: role?.getAttribute('aria-label'),
    engine: container.querySelector('.session-engine')?.textContent ?? null,
    task: task?.textContent,
    repositoryLines: Array.from(
      container.querySelectorAll('.session-repo'),
      (repo) => repo.textContent,
    ),
  }
}

function expectedFullRow(preview: string | null) {
  return {
    density: 'full',
    mobile: false,
    title: 'Review the retry policy',
    age: '2m',
    preview,
    role: 'development',
    engine: 'codex',
    task: 'P2 #17 Improve request reliability',
    repositoryLines: ['sample/service'],
  }
}

function vibeIndicatorState() {
  const indicator = container.querySelector(
    '.session-title-line .vibe-indicator',
  )
  return {
    label: indicator?.getAttribute('aria-label'),
    fill: indicator?.querySelector('svg')?.getAttribute('fill'),
  }
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
  vi.unstubAllGlobals()
})

it('shows the full Inbox row with role, engine, task and repository', () => {
  renderRow('full')

  expect(snapshot()).toEqual(
    expectedFullRow('Should retries stop after three attempts?'),
  )
})

it.each([
  {
    name: 'compact list rows retain horizontal padding',
    density: 'compact' as const,
    inTree: false,
    expected: [{ paddingLeft: '12px', paddingRight: '12px' }],
  },
  {
    name: 'desktop tree rows remain flush with their caret',
    density: 'compact' as const,
    inTree: true,
    expected: [{ paddingLeft: '0px', paddingRight: '0px' }],
  },
  {
    name: 'phone tree rows keep their existing horizontal spacing',
    density: 'compact-phone' as const,
    inTree: true,
    expected: [{ paddingLeft: '0px', paddingRight: '0px' }],
  },
])(
  'keeps the intended horizontal padding: $name',
  ({ density, inTree, expected }) => {
    renderRow(density, session, false, inTree)

    expect(horizontalPaddingSnapshot()).toEqual(expected)
  },
)

it.each([
  {
    name: 'shows the reason while waiting for a Crit review',
    isWaitingForCritReview: true,
    kind: 'Review',
    waitingReason: 'Review the retry policy changes',
    preview: 'Review the retry policy changes',
  },
  {
    name: 'keeps the existing preview when the Crit wait has no reason',
    isWaitingForCritReview: true,
    kind: 'Review',
    waitingReason: null,
    preview: 'Should retries stop after three attempts?',
  },
  {
    name: 'keeps the existing preview when a reason is not for a Crit wait',
    isWaitingForCritReview: false,
    kind: 'Review',
    waitingReason: 'Review the retry policy changes',
    preview: 'Should retries stop after three attempts?',
  },
  {
    name: 'keeps the existing preview when the Crit wait reason is blank',
    isWaitingForCritReview: true,
    kind: 'Review',
    waitingReason: '  ',
    preview: 'Should retries stop after three attempts?',
  },
  {
    name: 'keeps the Todo preview while a Crit review waits',
    isWaitingForCritReview: true,
    kind: 'Todo',
    waitingReason: 'Review the retry policy changes',
    preview: 'Should retries stop after three attempts?',
  },
  {
    name: 'keeps the Proposal preview while a Crit review waits',
    isWaitingForCritReview: true,
    kind: 'Proposal',
    waitingReason: 'Review the retry policy changes',
    preview: 'Should retries stop after three attempts?',
  },
])('$name', ({ isWaitingForCritReview, kind, waitingReason, preview }) => {
  renderRow('full', {
    ...session,
    isWaitingForCritReview,
    kind,
    waitingReason,
  })

  expect(snapshot()).toEqual(expectedFullRow(preview))
})

it('keeps the Crit review reason ahead of a folded-to-do preview', () => {
  renderRow('full', {
    ...session,
    isWaitingForCritReview: true,
    kind: 'Review',
    waitingReason: 'Awaiting reviewer feedback',
    todoLaterCount: 1,
    todoLaterTitle: 'Check the alert sound',
  })

  expect(snapshot()).toEqual(expectedFullRow('Awaiting reviewer feedback'))
})

it('shows compact rows without a preview and reserves the missing role slot', () => {
  renderRow('compact', { ...session, workType: null })

  expect(snapshot()).toEqual({
    density: 'compact',
    mobile: false,
    title: 'Review the retry policy',
    age: '2m',
    preview: null,
    role: 'No role',
    engine: 'codex',
    task: 'P2 #17 Improve request reliability +1',
    repositoryLines: ['sample/service'],
  })
})

it('shows compact phone rows with the same three detail lines', () => {
  renderRow('compact-phone', { ...session, workType: null })

  expect(snapshot()).toEqual({
    density: 'compact-phone',
    mobile: false,
    title: 'Review the retry policy',
    age: '2m',
    preview: null,
    role: 'No role',
    engine: 'codex',
    task: 'P2 #17 Improve request reliability +1',
    repositoryLines: ['sample/service'],
  })
})

it('keeps the full layout for mobile Inbox rows', () => {
  renderRow('full', session, true)

  expect(snapshot()).toEqual({
    density: 'full',
    mobile: true,
    title: 'Review the retry policy',
    age: '2m',
    preview: 'Should retries stop after three attempts?',
    role: 'development',
    engine: 'codex',
    task: 'P2 #17 Improve request reliability',
    repositoryLines: ['sample/service'],
  })
})

it('shows unreviewed decisions in the Done preview', () => {
  renderRow('full', {
    ...session,
    category: 'done',
    decisionCount: 3,
    unreviewedDecisionCount: 2,
  })

  const decisionPreviewSnapshot = () => ({
    preview: container.querySelector('.session-preview')?.textContent,
    previewClassName: container.querySelector('.session-preview')?.className,
    decisionCount:
      container.querySelector('.session-decision-count')?.textContent ?? null,
  })

  expect(decisionPreviewSnapshot()).toEqual({
    preview: '◇ 2 decisions to review',
    previewClassName: 'session-preview is-decision-preview',
    decisionCount: null,
  })
})

it.each([
  ['single unreviewed decision', 'done', 0, 1, '◇ 1 decision to review'],
  ['multiple decisions', 'needs_action', 2, 0, '◇ 2 decisions'],
] as const)(
  'shows the %s count with the shared label',
  (_label, category, decisionCount, unreviewedDecisionCount, expectedLabel) => {
    renderRow('compact', {
      ...session,
      category,
      decisionCount,
      unreviewedDecisionCount,
    })

    expect(
      container.querySelector('.session-decision-count')?.textContent,
    ).toEqual(expectedLabel)
  },
)

it.each([
  [
    'question',
    'Question',
    'lucide lucide-message-circle-question-mark session-row-kind-icon session-row-kind-icon--question',
  ],
  [
    'error',
    'Error',
    'lucide lucide-triangle-alert session-row-kind-icon session-row-kind-icon--error',
  ],
  [
    'stopped',
    'Stopped',
    'lucide lucide-circle-pause session-row-kind-icon session-row-kind-icon--stopped',
  ],
  [
    'unknown',
    'SampleKind',
    'lucide lucide-circle-question-mark session-row-kind-icon session-row-kind-icon--samplekind',
  ],
] as const)('uses the %s kind icon', (_name, kind, expectedClass) => {
  renderRow('compact', { ...session, kind })

  expect(
    container.querySelector('.session-row-kind-icon')?.getAttribute('class'),
  ).toEqual(expectedClass)
})

it.each([
  {
    name: 'enabled',
    mode: { mode: 'enabled' } as const,
    fill: 'currentColor',
  },
  {
    name: 'inherited',
    mode: { mode: 'inherited', ancestorSessionId: 'fixture-parent' } as const,
    fill: 'none',
  },
])('shows the $name vibe state beside the session title', ({ mode, fill }) => {
  renderRow('full', { ...session, vibeMode: mode })
  expect(vibeIndicatorState()).toEqual({
    label: mode.mode === 'enabled' ? 'Vibe on' : 'Vibe on · inherited',
    fill,
  })
})
