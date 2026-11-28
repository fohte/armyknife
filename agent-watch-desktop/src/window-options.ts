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

export const createPopupWindowOptions = (platform: NodeJS.Platform) =>
  withHiddenTitleBar(platform, {
    width: 1100,
    height: 800,
    minWidth: 800,
    webPreferences: {
      nodeIntegration: false,
      contextIsolation: true,
      sandbox: true,
    },
  })
