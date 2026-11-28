import { describe, expect, it } from 'vitest'

import { withHiddenTitleBar } from './window-options.js'

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
})
