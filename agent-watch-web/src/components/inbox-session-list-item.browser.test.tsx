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
    actionsClipPath:
      actions instanceof HTMLElement ? actions.style.clipPath : null,
    actionsTransitionProperty:
      actions === null
        ? null
        : window.getComputedStyle(actions).transitionProperty,
    actionsTransitionDuration:
      actions === null
        ? null
        : window.getComputedStyle(actions).transitionDuration,
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
    actionsClipPath: 'inset(0px 0px 0px 176px)',
    actionsTransitionProperty: 'clip-path',
    actionsTransitionDuration: '0.16s',
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
    dispatchPointer(row, 'pointermove', 40)
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
      actionsClipPath: 'inset(0px 0px 0px 176px)',
      actionsTransitionProperty: 'clip-path',
      actionsTransitionDuration: '0.16s',
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
      actionsClipPath: 'inset(0px 0px 0px 116px)',
      actionsTransitionProperty: 'clip-path',
      actionsTransitionDuration: '0.16s',
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
      actionsClipPath: 'inset(0px)',
      actionsTransitionProperty: 'clip-path',
      actionsTransitionDuration: '0.16s',
      actionsHiddenFromAccessibility: 'false',
    },
  ])
})

it.each([
  { label: 'Later', action: 'later' },
  { label: 'Close', action: 'close' },
])(
  'runs the $action callback after the row is fully swiped',
  ({ label, action }) => {
    const calls: string[] = []
    root = createRoot(container)
    act(() => {
      root?.render(
        <InboxSessionListItem
          session={session}
          selectedId={session.id}
          onSelect={() => undefined}
          isMobile
          onLater={() => {
            calls.push('later')
          }}
          onCloseSession={() => {
            calls.push('close')
          }}
          defaultSwiped
        />,
      )
    })
    const button = container.querySelector<HTMLButtonElement>(
      `.inbox-row-swipe-actions button[aria-label="${label}: ${session.title}"]`,
    )
    if (button === null) throw new Error(`The ${label} action is missing`)

    act(() => {
      button.click()
    })

    expect(calls).toEqual([action])
  },
)
