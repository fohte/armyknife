import '#index.css'

import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { page } from 'vitest/browser'

let container: HTMLDivElement
let root: Root | null = null

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

it('sets mobile text controls to 16px and keeps desktop sizing unchanged', async () => {
  root = createRoot(container)
  act(() => {
    root?.render(
      <>
        <style>{'.test-control { font-size: 12px; }'}</style>
        <input className="test-control" />
        <input className="test-control" type="text" />
        <input className="test-control" type="search" />
        <input className="test-control" type="email" />
        <input className="test-control" type="url" />
        <input className="test-control" type="tel" />
        <input className="test-control" type="password" />
        <input className="test-control" type="number" />
        <input className="test-control" type="date" />
        <textarea className="test-control" />
        <select className="test-control">
          <option>Option</option>
        </select>
        <input className="test-control" type="checkbox" />
        <input className="test-control" type="radio" />
        <input className="test-control" type="button" />
        <input className="test-control" type="submit" />
        <input className="test-control" type="reset" />
        <input className="test-control" type="image" />
        <input className="test-control" type="range" />
        <input className="test-control" type="color" />
        <input className="test-control" type="file" />
        <input className="test-control" type="hidden" />
      </>,
    )
  })

  await page.viewport(390, 844)
  const mobileSizes = [...container.querySelectorAll('.test-control')].map(
    (control) => getComputedStyle(control).fontSize,
  )
  await page.viewport(1280, 800)
  const desktopSizes = [...container.querySelectorAll('.test-control')].map(
    (control) => getComputedStyle(control).fontSize,
  )

  expect([mobileSizes, desktopSizes]).toMatchInlineSnapshot(`
    [
      [
        "16px",
        "16px",
        "16px",
        "16px",
        "16px",
        "16px",
        "16px",
        "16px",
        "16px",
        "16px",
        "16px",
        "12px",
        "12px",
        "12px",
        "12px",
        "12px",
        "12px",
        "12px",
        "12px",
        "12px",
        "12px",
      ],
      [
        "12px",
        "12px",
        "12px",
        "12px",
        "12px",
        "12px",
        "12px",
        "12px",
        "12px",
        "12px",
        "12px",
        "12px",
        "12px",
        "12px",
        "12px",
        "12px",
        "12px",
        "12px",
        "12px",
        "12px",
        "12px",
      ],
    ]
  `)
})
