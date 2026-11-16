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
  density: 'full' | 'compact',
  viewSession: Session = session,
  isMobile = false,
) {
  if (root !== null) act(() => root?.unmount())
  root = createRoot(container)
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

function snapshot() {
  const row = container.querySelector<HTMLElement>('[data-session-row-content]')
  const role = container.querySelector<HTMLElement>('.session-role-icon')
  const task = container.querySelector<HTMLElement>('.session-task')

  return {
    density:
      row?.classList.contains('density-full') === true ? 'full' : 'compact',
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

  expect(snapshot()).toEqual({
    density: 'full',
    mobile: false,
    title: 'Review the retry policy',
    age: '2m',
    preview: 'Should retries stop after three attempts?',
    role: 'development',
    engine: 'codex',
    task: 'P2 #17 Improve request reliability +1',
    repositoryLines: ['sample/service'],
  })
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
    task: 'P2 #17 Improve request reliability +1',
    repositoryLines: ['sample/service'],
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
