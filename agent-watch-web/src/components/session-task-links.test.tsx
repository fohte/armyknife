import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { userEvent } from 'vitest/browser'

import { SessionTaskLinks } from '#components/session-task-links'
import type { SessionTask } from '#types'

let container: HTMLDivElement
let root: Root | null = null

const primaryTask: SessionTask = {
  id: 'fixture-primary-task',
  number: 2048,
  title: 'Improve request reliability',
  priority: 1,
}

const secondaryTask: SessionTask = {
  id: 'fixture-secondary-task',
  number: 2046,
  title: 'Add request tracing',
  priority: 4,
}

const completedSecondaryTask: SessionTask = {
  id: 'fixture-completed-task',
  number: 2047,
  title: 'Document retry behavior',
  priority: null,
  isClosed: true,
}

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  container = document.createElement('div')
  container.style.setProperty('--aw-border-emphasis', '#a1a1aa')
  container.style.setProperty('--aw-text-body', '#d4d4d8')
  container.style.setProperty('--aw-text-muted', '#a1a1aa')
  container.style.setProperty(
    '--font-mono',
    "'JetBrains Mono Variable', monospace",
  )
  document.body.append(container)
  root = createRoot(container)
})

afterEach(() => {
  if (root !== null) act(() => root?.unmount())
  root = null
  container.remove()
  vi.unstubAllGlobals()
})

it.each([
  {
    description: 'a single linked task',
    tasks: [primaryTask],
    expectedCount: null,
  },
  {
    description: 'multiple linked tasks',
    tasks: [primaryTask, secondaryTask],
    expectedCount: '+1',
  },
  {
    description: 'a primary task and a completed secondary task',
    tasks: [primaryTask, completedSecondaryTask],
    expectedCount: null,
  },
])(
  'shows the primary task and count for $description',
  ({ tasks, expectedCount }) => {
    act(() => {
      root?.render(
        <SessionTaskLinks
          task={primaryTask}
          tasks={tasks}
          onOpenTasks={() => undefined}
        />,
      )
    })

    const snapshot = () => ({
      primary: container.querySelector('.session-task-line > a')?.textContent,
      count:
        container.querySelector('.session-task-links')?.textContent ?? null,
      countLabel: container
        .querySelector('.session-task-links')
        ?.getAttribute('aria-label'),
      popup: document.body.querySelector('.session-task-links-menu'),
    })
    expect(snapshot()).toEqual({
      primary: 'P1 #2048 Improve request reliability',
      count: expectedCount,
      countLabel: expectedCount === null ? undefined : 'Open 1 linked tq task',
      popup: null,
    })
  },
)

it('opens the Tasks panel when the linked-task count is clicked', () => {
  const onOpenTasks = vi.fn()
  act(() => {
    root?.render(
      <SessionTaskLinks
        task={primaryTask}
        tasks={[primaryTask, secondaryTask]}
        onOpenTasks={onOpenTasks}
      />,
    )
  })

  act(() => {
    container.querySelector<HTMLButtonElement>('.session-task-links')?.click()
  })

  expect(onOpenTasks.mock.calls).toEqual([[]])
})

it('shows the design styling and a visible keyboard focus outline', async () => {
  act(() => {
    root?.render(
      <SessionTaskLinks
        task={primaryTask}
        tasks={[primaryTask, secondaryTask]}
        onOpenTasks={() => undefined}
      />,
    )
  })

  const button = container.querySelector<HTMLButtonElement>(
    '.session-task-links',
  )
  if (button === null) throw new Error('Expected a linked-task count button')

  await userEvent.tab()
  await userEvent.tab()

  const snapshot = () => {
    const style = getComputedStyle(button)
    return {
      height: style.height,
      padding: style.padding,
      borderWidth: style.borderWidth,
      backgroundColor: style.backgroundColor,
      color: style.color,
      fontFamily: style.fontFamily,
      fontSize: style.fontSize,
      focused: document.activeElement === button,
      outlineColor: style.outlineColor,
      outlineStyle: style.outlineStyle,
      outlineWidth: style.outlineWidth,
      outlineOffset: style.outlineOffset,
    }
  }
  expect(snapshot()).toEqual({
    height: '20px',
    padding: '0px 6px',
    borderWidth: '0px',
    backgroundColor: 'rgba(0, 0, 0, 0)',
    color: 'rgb(161, 161, 170)',
    fontFamily: '"JetBrains Mono Variable", monospace',
    fontSize: '12px',
    focused: true,
    outlineColor: 'rgb(161, 161, 170)',
    outlineStyle: 'solid',
    outlineWidth: '2px',
    outlineOffset: '2px',
  })
})

it('changes the count color on hover without adding a border or background', async () => {
  act(() => {
    root?.render(
      <SessionTaskLinks
        task={primaryTask}
        tasks={[primaryTask, secondaryTask]}
        onOpenTasks={() => undefined}
      />,
    )
  })

  const button = container.querySelector<HTMLButtonElement>(
    '.session-task-links',
  )
  if (button === null) throw new Error('Expected a linked-task count button')

  await userEvent.hover(button)

  const snapshot = () => {
    const style = getComputedStyle(button)
    return {
      borderWidth: style.borderWidth,
      backgroundColor: style.backgroundColor,
      color: style.color,
    }
  }
  expect(snapshot()).toEqual({
    borderWidth: '0px',
    backgroundColor: 'rgba(0, 0, 0, 0)',
    color: 'rgb(212, 212, 216)',
  })
})
