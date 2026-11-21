import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, expect, it, vi } from 'vitest'

import { UpdateNotice } from '#components/update-notice'

let container: HTMLDivElement
let root: Root | null = null

afterEach(() => {
  if (root !== null) {
    act(() => {
      root?.unmount()
    })
  }
  root = null
  container.remove()
  vi.unstubAllGlobals()
  vi.restoreAllMocks()
})

it('calls the reload action only when Reload is clicked', () => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  container = document.createElement('div')
  document.body.append(container)
  const onReload = vi.fn()
  const onDismiss = vi.fn()
  root = createRoot(container)
  act(() => {
    root?.render(
      <UpdateNotice
        isMobile={false}
        onDismiss={onDismiss}
        onReload={onReload}
        version="fixture-version-next"
      />,
    )
  })

  act(() => {
    container.querySelector<HTMLButtonElement>('.update-notice-reload')?.click()
  })

  const snapshot = () => ({
    reloadCalls: onReload.mock.calls.length,
    dismissCalls: onDismiss.mock.calls.length,
  })
  expect(snapshot()).toEqual({ reloadCalls: 1, dismissCalls: 0 })
})
