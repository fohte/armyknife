import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import { ResizableColumns } from '#components/resizable-columns'

let container: HTMLDivElement
let root: Root | null = null

function renderColumns(
  storageKey: string | null = 'test:columns',
  containerWidth = 900,
) {
  root = createRoot(container)
  container.style.width = `${String(containerWidth)}px`
  act(() => {
    root?.render(
      <ResizableColumns
        first={<div>First pane</div>}
        second={<div>Second pane</div>}
        storageKey={storageKey}
        firstSize={400}
        firstMinSize={240}
        secondMinSize={320}
        label="Resize first and second panes"
      />,
    )
  })
}

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  window.localStorage.clear()
  container = document.createElement('div')
  container.style.width = '900px'
  container.style.height = '500px'
  document.body.append(container)
})

afterEach(() => {
  if (root !== null) act(() => root?.unmount())
  root = null
  container.remove()
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

it('restores the stored width and saves keyboard resizing', () => {
  window.localStorage.setItem('test:columns', '440')
  renderColumns()

  const separator =
    container.querySelector<HTMLDivElement>('[role="separator"]')
  act(() => {
    separator?.dispatchEvent(
      new KeyboardEvent('keydown', {
        key: 'ArrowRight',
        bubbles: true,
        cancelable: true,
      }),
    )
  })

  const snapshot = () => ({
    width: container.querySelector<HTMLDivElement>(
      '.resizable-columns-first-pane',
    )?.style.flexBasis,
    value: separator?.getAttribute('aria-valuenow'),
    saved: window.localStorage.getItem('test:columns'),
  })

  expect(snapshot()).toEqual({
    width: '450px',
    value: '450',
    saved: '450',
  })
})

it('stores the width after pointer dragging', () => {
  renderColumns('test:pointer')
  const separator =
    container.querySelector<HTMLDivElement>('[role="separator"]')
  if (separator === null) throw new Error('Expected a resize separator')
  vi.spyOn(separator, 'setPointerCapture').mockImplementation(() => undefined)

  act(() => {
    separator.dispatchEvent(
      new PointerEvent('pointerdown', {
        pointerId: 1,
        clientX: 100,
        bubbles: true,
        cancelable: true,
      }),
    )
    separator.dispatchEvent(
      new PointerEvent('pointermove', {
        pointerId: 1,
        clientX: 150,
        bubbles: true,
      }),
    )
  })

  const snapshot = () => ({
    width: container.querySelector<HTMLDivElement>(
      '.resizable-columns-first-pane',
    )?.style.flexBasis,
    value: separator.getAttribute('aria-valuenow'),
    saved: window.localStorage.getItem('test:pointer'),
  })

  expect(snapshot()).toEqual({
    width: '450px',
    value: '450',
    saved: '450',
  })
})

it('keeps a user-selected width when a narrow window clamps it', () => {
  renderColumns('test:columns', 1000)
  const separator =
    container.querySelector<HTMLDivElement>('[role="separator"]')
  if (separator === null) throw new Error('Expected a resize separator')
  vi.spyOn(separator, 'setPointerCapture').mockImplementation(() => undefined)

  act(() => {
    separator.dispatchEvent(
      new PointerEvent('pointerdown', {
        pointerId: 1,
        clientX: 100,
        bubbles: true,
        cancelable: true,
      }),
    )
    separator.dispatchEvent(
      new PointerEvent('pointermove', {
        pointerId: 1,
        clientX: 300,
        bubbles: true,
      }),
    )
  })

  const snapshot = () => {
    const currentSeparator =
      container.querySelector<HTMLDivElement>('[role="separator"]')
    return {
      width: container.querySelector<HTMLDivElement>(
        '.resizable-columns-first-pane',
      )?.style.flexBasis,
      value: currentSeparator?.getAttribute('aria-valuenow'),
      maximum: currentSeparator?.getAttribute('aria-valuemax'),
      saved: window.localStorage.getItem('test:columns'),
    }
  }

  const snapshots = [snapshot()]

  act(() => {
    root?.unmount()
  })
  root = null
  renderColumns('test:columns', 800)

  snapshots.push(snapshot())

  act(() => {
    root?.unmount()
  })
  root = null
  renderColumns('test:columns', 1000)

  snapshots.push(snapshot())

  expect(snapshots).toEqual([
    { width: '600px', value: '600', maximum: '672', saved: '600' },
    { width: '472px', value: '472', maximum: '472', saved: '600' },
    { width: '600px', value: '600', maximum: '672', saved: '600' },
  ])
})

it('keeps resizing when browser storage is unavailable', () => {
  vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
    throw new DOMException('Storage is unavailable')
  })
  vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
    throw new DOMException('Storage is unavailable')
  })
  renderColumns()

  const separator =
    container.querySelector<HTMLDivElement>('[role="separator"]')
  act(() => {
    separator?.dispatchEvent(
      new KeyboardEvent('keydown', {
        key: 'ArrowRight',
        bubbles: true,
        cancelable: true,
      }),
    )
  })

  const snapshot = () => ({
    width: container.querySelector<HTMLDivElement>(
      '.resizable-columns-first-pane',
    )?.style.flexBasis,
    value: separator?.getAttribute('aria-valuenow'),
  })

  expect(snapshot()).toEqual({
    width: '410px',
    value: '410',
  })
})
