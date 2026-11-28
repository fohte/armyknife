import { describe, expect, it } from 'vitest'

import {
  createContentWindowOptions,
  createPopupWindowOptions,
  createWindowOpenOptions,
  withHiddenTitleBar,
} from './window-options.js'

describe('withHiddenTitleBar', () => {
  it('positions macOS traffic lights inside the content area', () => {
    expect(
      withHiddenTitleBar('darwin', {
        width: 1280,
        height: 900,
        title: 'Agent Watch',
      }),
    ).toEqual({
      width: 1280,
      height: 900,
      title: 'Agent Watch',
      titleBarStyle: 'hidden',
      trafficLightPosition: { x: 9, y: 13 },
    })
  })

  it('keeps the platform default title bar on other platforms', () => {
    expect(
      withHiddenTitleBar('linux', {
        width: 1100,
        height: 800,
      }),
    ).toEqual({
      width: 1100,
      height: 800,
    })
  })

  it('keeps popup windows above the mobile layout breakpoint', () => {
    expect(createPopupWindowOptions('darwin')).toEqual({
      width: 1100,
      height: 800,
      minWidth: 800,
      webPreferences: {
        nodeIntegration: false,
        contextIsolation: true,
        sandbox: true,
      },
      titleBarStyle: 'hidden',
      trafficLightPosition: { x: 9, y: 13 },
    })
  })

  it('keeps standalone content windows with the native title bar', () => {
    expect(createContentWindowOptions()).toEqual({
      width: 1100,
      height: 800,
      webPreferences: {
        nodeIntegration: false,
        contextIsolation: true,
        sandbox: true,
      },
    })
  })

  it.each([
    {
      name: 'watch app pages',
      url: 'http://127.0.0.1:1234/?session=example',
      expected: {
        width: 1100,
        height: 800,
        minWidth: 800,
        webPreferences: {
          nodeIntegration: false,
          contextIsolation: true,
          sandbox: true,
        },
        titleBarStyle: 'hidden',
        trafficLightPosition: { x: 9, y: 13 },
      },
    },
    {
      name: 'standalone file content',
      url: 'http://127.0.0.1:1234/api/sessions/example/files/example.pdf',
      expected: {
        width: 1100,
        height: 800,
        webPreferences: {
          nodeIntegration: false,
          contextIsolation: true,
          sandbox: true,
        },
      },
    },
  ])('selects complete options for $name', ({ url, expected }) => {
    expect(
      createWindowOpenOptions(url, 'http://127.0.0.1:1234/', 'darwin'),
    ).toEqual(expected)
  })
})
