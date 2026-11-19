import type { PaletteItem } from '#command-palette-model'

export type PaletteAction = { id: string; label: string }

export function getCommandPaletteActions(
  item: PaletteItem | null,
): PaletteAction[] {
  if (item?.type === 'session') {
    return [
      { id: 'open', label: 'Open session' },
      { id: 'open-beside', label: 'Open beside' },
      { id: 'later', label: 'Later' },
      { id: 'copy', label: 'Copy session ID' },
      ...(item.session.canOpenInTerminal === true
        ? [{ id: 'terminal', label: 'Open in terminal' }]
        : []),
      { id: 'close', label: 'Close this session' },
    ]
  }
  if (item?.type === 'command') {
    return [{ id: 'command', label: 'Run command' }]
  }
  return item === null ? [] : [{ id: 'destination', label: 'Show in Board' }]
}
