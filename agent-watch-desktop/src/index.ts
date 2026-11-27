import { app, BrowserWindow, dialog, Menu, net, session, shell } from 'electron'
import { ResultAsync } from 'neverthrow'

import { resolveWatchTarget } from './config.js'
import type { WatchTarget } from './config.js'
import { createConnectionController } from './connection.js'
import { installApplicationMenu } from './menu.js'
import { classifyNavigation, isAllowedWatchUrl } from './navigation.js'
import {
  createQuitLifecycle,
  handleRendererPreventedUnload,
} from './quit-lifecycle.js'

const targetResult = resolveWatchTarget(process.env.AGENT_WATCH_URL)
const appName = 'Agent Watch'
const retryIntervalMs = 3000
const waitingPage = `data:text/html;charset=utf-8,${encodeURIComponent(`<!doctype html>
<html lang="ja">
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Agent Watch に接続中</title>
  <style>
    body { display: grid; min-height: 100vh; margin: 0; place-items: center; color: #e5e7eb; background: #111827; font: 15px -apple-system, BlinkMacSystemFont, sans-serif; }
    main { max-width: 34rem; padding: 2rem; }
    h1 { font-size: 1.25rem; font-weight: 600; }
    code { border-radius: .35rem; padding: .2rem .4rem; background: #1f2937; }
    p { line-height: 1.7; color: #9ca3af; }
  </style>
  <main>
    <h1>watch-web に接続できません</h1>
    <p>サーバーの起動を確認しています。接続できると自動で表示します。</p>
    <p>サーバーが停止している場合は、ターミナルで <code>a agent watch-web --no-open</code> を実行してください。</p>
  </main>
</html>`)}`

let mainWindow: BrowserWindow | undefined
let watchTarget: WatchTarget | undefined
let connectionController:
  ReturnType<typeof createConnectionController> | undefined
let retryTimer: NodeJS.Timeout | undefined
const quitLifecycle = createQuitLifecycle(() => {
  if (retryTimer !== undefined) clearInterval(retryTimer)
})

const reportPromise = (promise: Promise<unknown>, message: string) => {
  void ResultAsync.fromPromise(promise, (cause) => cause).match(
    () => undefined,
    (cause) => console.error(message, cause),
  )
}

const openExternal = (url: string) => {
  reportPromise(shell.openExternal(url), `failed to open external URL: ${url}`)
}

const refreshConnection = async () => {
  if (
    mainWindow === undefined ||
    mainWindow.isDestroyed() ||
    watchTarget === undefined ||
    connectionController === undefined
  ) {
    return
  }

  const serverResponded = await ResultAsync.fromPromise(
    net.fetch(watchTarget.url, { signal: AbortSignal.timeout(2000) }),
    () => false,
  ).match(
    () => true,
    () => false,
  )

  connectionController.probe(serverResponded)
}

const createMainWindow = () => {
  if (watchTarget === undefined) return
  const target = watchTarget

  connectionController = createConnectionController(target.origin, target.url, {
    showWaitingPage: () => {
      if (mainWindow === undefined || mainWindow.isDestroyed()) return
      reportPromise(
        mainWindow.loadURL(waitingPage),
        'failed to show waiting page',
      )
    },
    replaceWaitingPage: (url) => {
      if (mainWindow === undefined || mainWindow.isDestroyed()) return
      reportPromise(
        mainWindow.webContents.executeJavaScript(
          `window.location.replace(${JSON.stringify(url)})`,
        ),
        'failed to replace waiting page with watch-web',
      )
    },
  })

  mainWindow = new BrowserWindow({
    width: 1280,
    height: 900,
    minWidth: 800,
    minHeight: 600,
    title: appName,
    webPreferences: {
      nodeIntegration: false,
      contextIsolation: true,
      sandbox: true,
    },
  })

  mainWindow.on('close', (event) => {
    if (quitLifecycle.isQuitting()) return
    event.preventDefault()
    mainWindow?.hide()
  })
  mainWindow.on('closed', () => {
    mainWindow = undefined
  })

  installApplicationMenu()
  reportPromise(mainWindow.loadURL(target.url), 'failed to load watch-web')
  retryTimer = setInterval(() => void refreshConnection(), retryIntervalMs)
  void refreshConnection()
}

const configureWebContents = (contents: Electron.WebContents) => {
  if (watchTarget === undefined) return
  const target = watchTarget

  contents.on('will-navigate', (event, navigationUrl) => {
    const action = classifyNavigation(
      contents.getURL(),
      navigationUrl,
      target.origin,
      false,
    )
    if (action === 'allow') return

    event.preventDefault()
    if (action === 'open-external') openExternal(navigationUrl)
  })

  contents.on(
    'will-redirect',
    (event, navigationUrl, _isInPlace, isMainFrame) => {
      if (!isMainFrame) return

      const action = classifyNavigation(
        contents.getURL(),
        navigationUrl,
        target.origin,
        false,
      )
      if (action === 'allow') return

      event.preventDefault()
      if (action === 'open-external') openExternal(navigationUrl)
    },
  )

  contents.setWindowOpenHandler(({ url }) => {
    const action = classifyNavigation(
      contents.getURL(),
      url,
      target.origin,
      true,
    )
    if (action === 'open-window') {
      return {
        action: 'allow',
        overrideBrowserWindowOptions: {
          width: 1100,
          height: 800,
          webPreferences: {
            nodeIntegration: false,
            contextIsolation: true,
            sandbox: true,
          },
        },
      }
    }

    if (action === 'open-external') openExternal(url)
    return { action: 'deny' }
  })

  contents.on(
    'did-fail-load',
    (_event, errorCode, _description, validatedUrl, isMainFrame) => {
      if (
        isMainFrame &&
        errorCode !== -3 &&
        contents === mainWindow?.webContents
      ) {
        connectionController?.mainFrameDidFail(validatedUrl)
      }
    },
  )

  contents.on('did-finish-load', () => {
    if (contents === mainWindow?.webContents) {
      connectionController?.mainFrameDidLoad(contents.getURL())
    }
  })

  contents.on('will-prevent-unload', (event) => {
    handleRendererPreventedUnload(quitLifecycle, event, (isQuitting) => {
      const options = {
        type: 'warning' as const,
        title: appName,
        message: '送信していないメッセージがあります',
        detail: '破棄して続行しますか',
        buttons: ['編集を続ける', isQuitting ? '破棄して終了' : '破棄して続行'],
        defaultId: 0,
        cancelId: 0,
      }
      const window = BrowserWindow.fromWebContents(contents)
      const choice =
        window === null
          ? dialog.showMessageBoxSync(options)
          : dialog.showMessageBoxSync(window, options)
      return choice === 1
    })
  })
}

const hasSingleInstanceLock = app.requestSingleInstanceLock()
if (!hasSingleInstanceLock) {
  app.quit()
} else {
  app.setName(appName)
  app.on('before-quit', () => quitLifecycle.beforeQuit())
  app.on('will-quit', () => quitLifecycle.willQuit())
  app.on('web-contents-created', (_event, contents) =>
    configureWebContents(contents),
  )
  app.on('second-instance', () => {
    if (mainWindow !== undefined && !mainWindow.isDestroyed()) {
      app.show()
      mainWindow.show()
      mainWindow.focus()
    }
  })
  app.on('activate', () => {
    if (mainWindow === undefined || mainWindow.isDestroyed()) {
      createMainWindow()
      return
    }

    app.show()
    mainWindow.show()
  })
  app.whenReady().then(() => {
    targetResult.match(
      (target) => {
        watchTarget = target
        session.defaultSession.setPermissionRequestHandler(
          (_webContents, permission, callback, details) => {
            callback(
              permission === 'clipboard-sanitized-write' &&
                watchTarget !== undefined &&
                isAllowedWatchUrl(details.requestingUrl, watchTarget.origin),
            )
          },
        )
        createMainWindow()
      },
      (error) => {
        dialog.showErrorBox(appName, error)
        app.quit()
      },
    )
  })
}
