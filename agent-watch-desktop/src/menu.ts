import { BrowserWindow, Menu } from 'electron'

export const installApplicationMenu = () => {
  Menu.setApplicationMenu(
    Menu.buildFromTemplate([
      { role: 'appMenu' },
      { role: 'fileMenu' },
      { role: 'editMenu' },
      { role: 'viewMenu' },
      {
        label: 'History',
        submenu: [
          {
            label: 'Back',
            accelerator: 'Cmd+[',
            click: () => {
              const history =
                BrowserWindow.getFocusedWindow()?.webContents.navigationHistory
              if (history?.canGoBack()) history.goBack()
            },
          },
          {
            label: 'Forward',
            accelerator: 'Cmd+]',
            click: () => {
              const history =
                BrowserWindow.getFocusedWindow()?.webContents.navigationHistory
              if (history?.canGoForward()) history.goForward()
            },
          },
        ],
      },
    ]),
  )
}
