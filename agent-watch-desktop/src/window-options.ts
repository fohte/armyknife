import type { BrowserWindowConstructorOptions } from 'electron'

import { isWatchAppPageUrl } from './navigation.js'

const popupWindowOptions = {
  width: 1100,
  height: 800,
  webPreferences: {
    nodeIntegration: false,
    contextIsolation: true,
    sandbox: true,
  },
} satisfies BrowserWindowConstructorOptions

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
  withHiddenTitleBar(platform, { ...popupWindowOptions, minWidth: 800 })

export const createContentWindowOptions =
  (): BrowserWindowConstructorOptions => ({ ...popupWindowOptions })

export const createWindowOpenOptions = (
  url: string,
  watchUrl: string,
  platform: NodeJS.Platform,
): BrowserWindowConstructorOptions =>
  isWatchAppPageUrl(url, watchUrl)
    ? createPopupWindowOptions(platform)
    : createContentWindowOptions()
