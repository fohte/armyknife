import { act, type ComponentProps } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { userEvent } from 'vitest/browser'

import { CommandPalette } from '#components/command-palette'
import type { Session } from '#types'

let container: HTMLDivElement
let root: Root | null = null

const session: Session = {
  id: 'session-alerts',
  category: 'needs_action',
  isPaused: false,
  kind: 'Question',
  title: 'Review notification settings',
  preview: null,
  elapsedSeconds: 300,
  unread: true,
  task: {
    id: 'task-alerts',
    number: 42,
    title: 'Improve notifications',
    priority: 1,
  },
  repo: 'example/web-app',
  project: 'notification refresh',
  parent: null,
  ancestorSessionIds: [],
  pullRequest: null,
}

function renderPalette(
  props: Partial<ComponentProps<typeof CommandPalette>> = {},
) {
  root = createRoot(container)
  act(() => {
    root?.render(
      <CommandPalette
        sessions={[session]}
        recentSessionIds={[]}
        inboxFilterQuery=""
        onClose={() => undefined}
        onOpenSession={() => undefined}
        onOpenProject={() => undefined}
        onRunCommand={() => undefined}
        {...props}
      />,
    )
  })
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
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

it('opens the project for the active result with a prefix and Enter', async () => {
  const onOpenProject = vi.fn<(name: string) => void>()
  renderPalette({ onOpenProject })
  const input = container.querySelector<HTMLInputElement>(
    '[aria-label="Jump to a session, tq task or project, or type > for commands"]',
  )
  if (input === null) throw new Error('Palette input was not rendered')

  await act(async () => {
    await userEvent.type(input, '#notification')
    await userEvent.keyboard('{Enter}')
  })

  expect(onOpenProject.mock.calls).toEqual([['notification refresh']])
})

it('opens a matching project from the project scope with Enter', async () => {
  const onOpenProject = vi.fn<(name: string) => void>()
  renderPalette({ onOpenProject })
  const input = container.querySelector<HTMLInputElement>(
    '[aria-label="Jump to a session, tq task or project, or type > for commands"]',
  )
  if (input === null) throw new Error('Palette input was not rendered')

  await act(async () => {
    await userEvent.type(input, '@notification')
    await userEvent.keyboard('{Enter}')
  })

  expect(onOpenProject.mock.calls).toEqual([['notification refresh']])
})

it('runs the selected command with Ctrl-n and Enter', async () => {
  const onRunCommand =
    vi.fn<NonNullable<ComponentProps<typeof CommandPalette>['onRunCommand']>>()
  renderPalette({ initialQuery: '>Switch to', onRunCommand })
  const input = container.querySelector<HTMLInputElement>(
    '[aria-label="Jump to a session, tq task or project, or type > for commands"]',
  )
  if (input === null) throw new Error('Palette input was not rendered')

  await act(async () => {
    await userEvent.keyboard('{Control>}n{/Control}')
    await userEvent.keyboard('{Enter}')
  })

  expect(onRunCommand.mock.calls).toEqual([
    [
      {
        type: 'command',
        id: 'switch-inbox',
        label: 'Switch to Inbox',
        description: 'Show the session inbox',
      },
    ],
  ])
})
