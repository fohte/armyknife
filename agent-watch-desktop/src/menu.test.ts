import { beforeEach, describe, expect, it, vi } from 'vitest'

const electronMocks = vi.hoisted(() => ({
  getFocusedWindow: vi.fn(),
  buildFromTemplate: vi.fn(),
  setApplicationMenu: vi.fn(),
}))

vi.mock('electron', () => ({
  BrowserWindow: { getFocusedWindow: electronMocks.getFocusedWindow },
  Menu: {
    buildFromTemplate: electronMocks.buildFromTemplate,
    setApplicationMenu: electronMocks.setApplicationMenu,
  },
}))

import { installApplicationMenu } from './menu.js'

type TestMenuItem = {
  label?: string
  submenu?: TestMenuItem[]
  click?: () => void
}

describe('installApplicationMenu', () => {
  beforeEach(() => {
    electronMocks.getFocusedWindow.mockReset()
    electronMocks.buildFromTemplate.mockReset().mockReturnValue([])
    electronMocks.setApplicationMenu.mockReset()
  })

  it.each([
    { label: 'Back', action: 'back' },
    { label: 'Forward', action: 'forward' },
  ])('uses the focused window for $label', ({ label, action }) => {
    const actions: string[] = []
    electronMocks.getFocusedWindow.mockReturnValue({
      webContents: {
        navigationHistory: {
          canGoBack: () => true,
          goBack: () => actions.push('focused:back'),
          canGoForward: () => true,
          goForward: () => actions.push('focused:forward'),
        },
      },
    })

    installApplicationMenu()

    const template = electronMocks.buildFromTemplate.mock.calls[0]?.[0] as
      TestMenuItem[] | undefined
    const history = template?.find((item) => item.label === 'History')
    history?.submenu?.find((item) => item.label === label)?.click?.()

    expect(actions).toEqual([`focused:${action}`])
  })
})
