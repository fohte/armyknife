import { Menu, type BrowserWindow } from 'electron'

export const installApplicationMenu = (
  getWindow: () => BrowserWindow | undefined,
) => {
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
              const history = getWindow()?.webContents.navigationHistory
              if (history?.canGoBack()) history.goBack()
            },
          },
          {
            label: 'Forward',
            accelerator: 'Cmd+]',
            click: () => {
              const history = getWindow()?.webContents.navigationHistory
              if (history?.canGoForward()) history.goForward()
            },
          },
        ],
      },
    ]),
  )
}
