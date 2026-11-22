import '#index.css'

import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import { InboxSessionListItem } from '#inbox-session-list-item'
import type { Session } from '#types'

const session: Session = {
  id: 'sample-session',
  category: 'needs_action',
  isPaused: false,
  kind: 'Review',
  title: 'Review the sample service changes',
  preview: 'Review the changes before the next release.',
  elapsedSeconds: 120,
  unread: true,
  task: null,
  repo: 'example/service',
  project: 'service',
  parent: null,
  ancestorSessionIds: [],
  pullRequest: null,
}

let container: HTMLDivElement
let root: Root | null = null

function renderRow(defaultSwiped = false) {
  root = createRoot(container)
  act(() => {
    root?.render(
      <InboxSessionListItem
        session={session}
        selectedId={session.id}
        onSelect={() => undefined}
        isMobile
        onLater={() => undefined}
        onCloseSession={() => undefined}
        onOpenActions={() => undefined}
        defaultSwiped={defaultSwiped}
      />,
    )
  })
}

function snapshot() {
  const actions = container.querySelector('.inbox-row-swipe-actions')
  const actionLabels = Array.from(
    actions?.querySelectorAll('button') ?? [],
    (button) => button.getAttribute('aria-label'),
  )
  return {
    selected: container
      .querySelector('.session-row')
      ?.classList.contains('is-selected'),
    actionsContainerPresent: actions !== null,
    actionsVisible: actionLabels.length > 0,
    actionLabels,
    actionsHiddenFromAccessibility:
      actions?.getAttribute('aria-hidden') ?? null,
  }
}

function dispatchPointer(
  target: Element,
  type: 'pointerdown' | 'pointermove' | 'pointerup',
  clientX: number,
) {
  target.dispatchEvent(
    new PointerEvent(type, {
      bubbles: true,
      button: 0,
      clientX,
      clientY: 0,
      pointerId: 1,
      pointerType: 'touch',
      isPrimary: true,
    }),
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
  vi.unstubAllGlobals()
})

it('hides swipe actions from a selected phone row at rest', () => {
  renderRow()

  expect(snapshot()).toEqual({
    selected: true,
    actionsContainerPresent: true,
    actionsVisible: false,
    actionLabels: [],
    actionsHiddenFromAccessibility: 'true',
  })
})

it('reveals swipe actions while dragging and keeps them after release', () => {
  renderRow()
  const row = container.querySelector('.session-row')
  if (row === null) throw new Error('The selected session row is missing')

  const states = [snapshot()]
  act(() => {
    dispatchPointer(row, 'pointerdown', 100)
  })
  act(() => {
    dispatchPointer(row, 'pointermove', 30)
  })
  states.push(snapshot())
  act(() => {
    dispatchPointer(row, 'pointerup', 30)
  })
  states.push(snapshot())

  expect(states).toEqual([
    {
      selected: true,
      actionsContainerPresent: true,
      actionsVisible: false,
      actionLabels: [],
      actionsHiddenFromAccessibility: 'true',
    },
    {
      selected: true,
      actionsContainerPresent: true,
      actionsVisible: true,
      actionLabels: [
        'Later: Review the sample service changes',
        'Close: Review the sample service changes',
      ],
      actionsHiddenFromAccessibility: 'true',
    },
    {
      selected: true,
      actionsContainerPresent: true,
      actionsVisible: true,
      actionLabels: [
        'Later: Review the sample service changes',
        'Close: Review the sample service changes',
      ],
      actionsHiddenFromAccessibility: 'false',
    },
  ])
})
