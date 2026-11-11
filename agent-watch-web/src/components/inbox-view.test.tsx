import { act, type ComponentProps } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import { InboxView } from '#components/inbox-view'
import type { Session } from '#types'

let container: HTMLDivElement
let root: Root | null = null

const sessions: Session[] = [
  {
    id: 'sample-needs-action',
    category: 'needs_action',
    isPaused: false,
    kind: 'Question',
    title: 'Check the retry policy',
    preview: null,
    elapsedSeconds: 10,
    unread: true,
    task: null,
    repo: 'sample-org/service',
    project: 'web-app',
    parent: null,
    ancestorSessionIds: [],
    pullRequest: null,
  },
  {
    id: 'sample-later',
    category: 'later',
    isPaused: false,
    kind: 'Review',
    title: 'Review the changes',
    preview: null,
    elapsedSeconds: 20,
    unread: false,
    task: null,
    repo: 'sample-org/service',
    project: 'web-app',
    parent: null,
    ancestorSessionIds: [],
    pullRequest: null,
  },
  {
    id: 'sample-progress',
    category: 'in_progress',
    isPaused: false,
    kind: null,
    title: 'Add request metrics',
    preview: null,
    elapsedSeconds: 30,
    unread: false,
    task: null,
    repo: 'sample-org/metrics',
    project: 'web-app',
    parent: null,
    ancestorSessionIds: [],
    pullRequest: null,
  },
  {
    id: 'sample-done',
    category: 'done',
    isPaused: false,
    kind: null,
    title: 'Document request limits',
    preview: null,
    elapsedSeconds: 40,
    unread: false,
    task: null,
    repo: 'sample-org/service',
    project: 'web-app',
    parent: null,
    ancestorSessionIds: [],
    pullRequest: null,
  },
  {
    id: 'sample-other-project',
    category: 'needs_action',
    isPaused: false,
    kind: 'Error',
    title: 'Inspect worker output',
    preview: null,
    elapsedSeconds: 50,
    unread: true,
    task: null,
    repo: 'sample-org/worker',
    project: 'worker',
    parent: null,
    ancestorSessionIds: [],
    pullRequest: null,
  },
]

function renderInbox(props: Partial<ComponentProps<typeof InboxView>> = {}) {
  root = createRoot(container)
  act(() => {
    root?.render(
      <InboxView
        sessions={sessions}
        selectedId={null}
        loaded
        error={null}
        showAllInProgress
        onShowMore={() => undefined}
        onSelect={() => undefined}
        laterExpanded
        onToggleLater={() => undefined}
        onLater={() => undefined}
        onToggleToday={() => undefined}
        todaySessions={[]}
        onSelectToday={() => undefined}
        isMobile={false}
        inProgressExpanded
        onToggleInProgress={() => undefined}
        doneExpanded
        onToggleDone={() => undefined}
        {...props}
      />,
    )
  })
}

function sectionSnapshot() {
  return {
    headings: Array.from(container.querySelectorAll('.section-heading')).map(
      (heading) => heading.textContent,
    ),
    laterCount: container.querySelector('.later-section-count')?.textContent,
    visibleSessionIds: Array.from(
      container.querySelectorAll('.session-title'),
    ).map((title) => title.textContent),
    resultCount: container.querySelector('.inbox-filter-result-count')
      ?.textContent,
  }
}

function appliedFilterSnapshot() {
  return {
    dialogOpen: container.querySelector('[role="dialog"]') !== null,
    chips: container.querySelector('.inbox-filter-chips')?.textContent,
    needsActionCount: container.querySelector(
      '.needs-action-section .section-count',
    )?.textContent,
    visibleTitles: Array.from(container.querySelectorAll('.session-title')).map(
      (title) => title.textContent,
    ),
  }
}

function submitFilterQuery(query: string) {
  const input = container.querySelector<HTMLInputElement>(
    '[aria-label="Inbox filter query"]',
  )
  if (input === null) return
  const valueSetter = Reflect.getOwnPropertyDescriptor(
    HTMLInputElement.prototype,
    'value',
  )?.set
  if (valueSetter === undefined) return
  Reflect.apply(valueSetter, input, [query])
  input.dispatchEvent(new Event('input', { bubbles: true }))
  container
    .querySelector('form.inbox-filter-search-row')
    ?.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }))
}

function chipSnapshot() {
  return {
    chips: container.querySelector('.inbox-filter-chips')?.textContent,
    visibleTitles: Array.from(container.querySelectorAll('.session-title')).map(
      (title) => title.textContent,
    ),
  }
}

function dialogSnapshot() {
  return { isOpen: container.querySelector('[role="dialog"]') !== null }
}

function mobileRowActions() {
  return Array.from(
    container.querySelectorAll('.session-row-actions button'),
  ).map((button) => button.getAttribute('aria-label'))
}

function clickFilterOption(value: string) {
  const option = Array.from(
    container.querySelectorAll<HTMLLabelElement>('.inbox-filter-option'),
  ).find((label) => label.textContent.startsWith(value))
  option?.querySelector('input')?.click()
}

function slashInputSnapshot(event: KeyboardEvent) {
  return {
    defaultPrevented: event.defaultPrevented,
    ...dialogSnapshot(),
  }
}

function checkboxSnapshot() {
  return {
    query: container.querySelector<HTMLInputElement>(
      '[aria-label="Inbox filter query"]',
    )?.value,
    reviewChecked: Array.from(
      container.querySelectorAll<HTMLLabelElement>('.inbox-filter-option'),
    )
      .find((label) => label.textContent.startsWith('Review'))
      ?.querySelector<HTMLInputElement>('input')?.checked,
    ...dialogSnapshot(),
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

it('filters each inbox section and updates the visible counts', () => {
  renderInbox({ filterQuery: 'project:web-app' })

  expect(sectionSnapshot()).toEqual({
    headings: ['Needs action1', 'In progress1', 'Done1'],
    laterCount: '1',
    visibleSessionIds: [
      'Check the retry policy',
      'Review the changes',
      'Add request metrics',
      'Document request limits',
    ],
    resultCount: '1 of 2',
  })
})

it('opens the filter dialog from its button', () => {
  renderInbox()

  act(() => {
    container
      .querySelector<HTMLButtonElement>('.inbox-filter-open')
      ?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
  })

  expect(dialogSnapshot()).toEqual({ isOpen: true })
})

it('hides row actions from sessions on an iPhone', () => {
  renderInbox({ isMobile: true })

  expect(mobileRowActions()).toEqual([])
})

it('opens the filter dialog with slash', () => {
  renderInbox()

  act(() => {
    window.dispatchEvent(
      new KeyboardEvent('keydown', {
        key: '/',
        bubbles: true,
        cancelable: true,
      }),
    )
  })

  expect(dialogSnapshot()).toEqual({ isOpen: true })
})

it('leaves slash unhandled when the filter query input has focus', () => {
  renderInbox()
  act(() => {
    window.dispatchEvent(
      new KeyboardEvent('keydown', {
        key: '/',
        bubbles: true,
        cancelable: true,
      }),
    )
  })
  const input = container.querySelector<HTMLInputElement>(
    '[aria-label="Inbox filter query"]',
  )
  const slashEvent = new KeyboardEvent('keydown', {
    key: '/',
    bubbles: true,
    cancelable: true,
  })

  act(() => {
    input?.dispatchEvent(slashEvent)
  })

  expect(slashInputSnapshot(slashEvent)).toEqual({
    defaultPrevented: false,
    isOpen: true,
  })
})

it('closes the filter dialog with Escape', () => {
  renderInbox()
  act(() => {
    window.dispatchEvent(
      new KeyboardEvent('keydown', {
        key: '/',
        bubbles: true,
        cancelable: true,
      }),
    )
  })
  act(() => {
    container.querySelector('[aria-label="Inbox filter query"]')?.dispatchEvent(
      new KeyboardEvent('keydown', {
        key: 'Escape',
        bubbles: true,
        cancelable: true,
      }),
    )
  })

  expect(dialogSnapshot()).toEqual({ isOpen: false })
})

it('applies the query when the dialog form is submitted', () => {
  renderInbox()
  act(() => {
    container
      .querySelector<HTMLButtonElement>('.inbox-filter-open')
      ?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
  })
  act(() => {
    submitFilterQuery('project:worker')
  })

  expect(appliedFilterSnapshot()).toEqual({
    dialogOpen: false,
    chips: 'Project worker×',
    needsActionCount: '1',
    visibleTitles: ['Inspect worker output'],
  })
})

it('removes only the filter chip that was cleared', () => {
  renderInbox()
  act(() => {
    container
      .querySelector<HTMLButtonElement>('.inbox-filter-open')
      ?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
  })
  act(() => {
    submitFilterQuery('project:web-app kind:review')
  })
  act(() => {
    container
      .querySelector<HTMLButtonElement>('[aria-label="Remove Project filter"]')
      ?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
  })

  expect(chipSnapshot()).toEqual({
    chips: 'Kind review×',
    visibleTitles: ['Review the changes'],
  })
})

it('toggles dialog options while retaining other filter fields', () => {
  renderInbox()
  act(() => {
    container
      .querySelector<HTMLButtonElement>('.inbox-filter-open')
      ?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
  })
  act(() => {
    clickFilterOption('web-app')
  })
  act(() => {
    Array.from(
      container.querySelectorAll<HTMLButtonElement>('.inbox-filter-field'),
    )
      .find((button) => button.textContent.includes('Kind'))
      ?.click()
  })
  act(() => {
    clickFilterOption('Review')
  })
  act(() => {
    clickFilterOption('Review')
  })

  expect(checkboxSnapshot()).toEqual({
    query: 'project:web-app',
    reviewChecked: false,
    isOpen: true,
  })
})
