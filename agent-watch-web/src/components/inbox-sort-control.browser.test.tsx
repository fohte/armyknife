import { act, useState } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { userEvent } from 'vitest/browser'

import { InboxSortControl } from '#components/inbox-sort-control'
import type { InboxSort } from '#inbox-sort'

let container: HTMLDivElement
let root: Root | null = null

function SortControlHarness() {
  const [sort, setSort] = useState<InboxSort>('priority')
  const [open, setOpen] = useState(false)

  return (
    <InboxSortControl
      sort={sort}
      storageAvailable
      isMobile={false}
      open={open}
      onOpenChange={setOpen}
      onSortChange={setSort}
    />
  )
}

function renderSortControl() {
  root = createRoot(container)
  act(() => {
    root?.render(<SortControlHarness />)
  })
}

function sortTrigger() {
  const trigger = container.querySelector<HTMLButtonElement>(
    '.inbox-sort-trigger',
  )
  if (trigger === null) throw new Error('Expected sort trigger')
  return trigger
}

function menuOption(label: string) {
  const option = Array.from(
    container.querySelectorAll<HTMLButtonElement>('[role="menuitemradio"]'),
  ).find((candidate) => candidate.innerText.includes(label))
  if (option === undefined) throw new Error(`Expected ${label} sort option`)
  return option
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

it('opens on the selected option and moves focus with the arrow keys', async () => {
  renderSortControl()
  const trigger = sortTrigger()

  await act(async () => userEvent.click(trigger))
  await act(async () => userEvent.keyboard('{ArrowDown}'))

  const snapshot = () => ({
    expanded: trigger.getAttribute('aria-expanded'),
    focusedOption:
      container
        .querySelector<HTMLButtonElement>('[role="menuitemradio"]:focus')
        ?.innerText.replace(/\s+/g, ' ')
        .trim() ?? null,
    selectedOption: menuOption('Priority').getAttribute('aria-checked'),
  })
  expect(snapshot()).toEqual({
    expanded: 'true',
    focusedOption: 'Updated Newest first',
    selectedOption: 'true',
  })
})

it('returns focus to the trigger after choosing an option with Enter', async () => {
  renderSortControl()
  const trigger = sortTrigger()

  await act(async () => userEvent.click(trigger))
  await act(async () => userEvent.keyboard('{ArrowDown}{Enter}'))

  const snapshot = () => ({
    triggerLabel: trigger.getAttribute('aria-label'),
    expanded: trigger.getAttribute('aria-expanded'),
    focusReturned: document.activeElement === trigger,
    menuOpen: container.querySelector('.inbox-sort-menu') !== null,
  })
  expect(snapshot()).toEqual({
    triggerLabel: 'Sort Updated',
    expanded: 'false',
    focusReturned: true,
    menuOpen: false,
  })
})

it('returns focus to the trigger after Escape closes the menu', async () => {
  renderSortControl()
  const trigger = sortTrigger()

  await act(async () => userEvent.click(trigger))
  await act(async () => userEvent.keyboard('{Escape}'))

  const snapshot = () => ({
    expanded: trigger.getAttribute('aria-expanded'),
    menuOpen: container.querySelector('.inbox-sort-menu') !== null,
    focusReturned: document.activeElement === trigger,
  })
  expect(snapshot()).toEqual({
    expanded: 'false',
    menuOpen: false,
    focusReturned: true,
  })
})

it('opens the closed menu from the trigger with ArrowDown', async () => {
  renderSortControl()
  const trigger = sortTrigger()
  trigger.focus()

  await act(async () => userEvent.keyboard('{ArrowDown}'))

  const snapshot = () => ({
    expanded: trigger.getAttribute('aria-expanded'),
    menuOpen: container.querySelector('.inbox-sort-menu') !== null,
    focusedOption:
      container
        .querySelector<HTMLButtonElement>('[role="menuitemradio"]:focus')
        ?.innerText.replace(/\s+/g, ' ')
        .trim() ?? null,
    selectedOption: menuOption('Priority').getAttribute('aria-checked'),
  })
  expect(snapshot()).toEqual({
    expanded: 'true',
    menuOpen: true,
    focusedOption: 'Priority tq priority, then newest',
    selectedOption: 'true',
  })
})
