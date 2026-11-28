import type { BrowserWindowConstructorOptions } from 'electron'

export const withHiddenTitleBar = (
  platform: NodeJS.Platform,
  options: BrowserWindowConstructorOptions,
): BrowserWindowConstructorOptions =>
  platform === 'darwin'
    ? {
        ...options,
        titleBarStyle: 'hidden',
        trafficLightPosition: { x: 9, y: 13 },
      }
    : options
