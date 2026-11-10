export type CommandPaletteShortcut = {
  modifier: 'metaKey' | 'ctrlKey'
  label: '⌘K' | 'Ctrl K'
  ariaLabel: 'Command K' | 'Control K'
}

export function getCommandPaletteShortcut(
  platform: string,
): CommandPaletteShortcut {
  return platform.startsWith('Mac')
    ? { modifier: 'metaKey', label: '⌘K', ariaLabel: 'Command K' }
    : { modifier: 'ctrlKey', label: 'Ctrl K', ariaLabel: 'Control K' }
}
