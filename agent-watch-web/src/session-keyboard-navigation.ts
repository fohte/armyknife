import { useEffect } from 'react'

import type { HistoryMode, SessionHistoryState } from '#session-history'
import {
  getLaterPresetShortcut,
  getSessionShortcutAction,
  type LaterPreset,
  moveSelection,
  type SessionView,
} from '#session-navigation'
import type { Session } from '#types'

type SessionKeyboardNavigationOptions = {
  isMobile: boolean
  laterSessionId: string | null
  laterSubmissionInFlight: { current: boolean }
  closeLater: () => void
  submitLater: (sessionId: string, preset: LaterPreset) => void
  navigableSessions: readonly Session[]
  openLater: (sessionId: string) => void
  selectedId: string | null
  view: SessionView
  navigate: (
    update: (current: SessionHistoryState) => SessionHistoryState,
    mode?: HistoryMode,
  ) => void
  toggleInbox: () => void
  toggleStatusPanel: () => void
}

export function useSessionKeyboardNavigation({
  isMobile,
  laterSessionId,
  laterSubmissionInFlight,
  closeLater,
  submitLater,
  navigableSessions,
  openLater,
  selectedId,
  view,
  navigate,
  toggleInbox,
  toggleStatusPanel,
}: SessionKeyboardNavigationOptions) {
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (
        event.defaultPrevented ||
        event.altKey ||
        (event.target instanceof HTMLElement &&
          (event.target.isContentEditable ||
            ['INPUT', 'TEXTAREA', 'SELECT'].includes(event.target.tagName)))
      ) {
        return
      }

      if (
        !isMobile &&
        event.metaKey &&
        event.key === '.' &&
        selectedId !== null &&
        view === 'inbox'
      ) {
        event.preventDefault()
        toggleStatusPanel()
        return
      }

      if (laterSessionId !== null) {
        if (event.key === 'Escape') {
          event.preventDefault()
          if (!laterSubmissionInFlight.current) {
            closeLater()
          }
          return
        }
        if (isMobile) {
          return
        }
        if (event.ctrlKey || event.metaKey) return
        const preset = getLaterPresetShortcut(event.key)
        if (preset !== null) {
          event.preventDefault()
          submitLater(laterSessionId, preset)
        }
        return
      }

      if (isMobile) return

      if (event.ctrlKey || event.metaKey) return

      if (event.key === '[') {
        event.preventDefault()
        toggleInbox()
        return
      }

      const action = getSessionShortcutAction(event.key, event.shiftKey, view)
      if (action?.type === 'view') {
        event.preventDefault()
        navigate((current) => ({ ...current, view: action.view }))
      } else if (action?.type === 'move' && view === 'inbox') {
        event.preventDefault()
        navigate(
          (current) => ({
            ...current,
            selectedId: moveSelection(
              navigableSessions,
              current.selectedId,
              action.key,
            ),
          }),
          'replace',
        )
      } else if (
        action?.type === 'later' &&
        selectedId !== null &&
        view === 'inbox'
      ) {
        event.preventDefault()
        openLater(selectedId)
      }
    }

    window.addEventListener('keydown', onKeyDown)
    return () => {
      window.removeEventListener('keydown', onKeyDown)
    }
  }, [
    closeLater,
    isMobile,
    laterSessionId,
    laterSubmissionInFlight,
    navigate,
    navigableSessions,
    openLater,
    selectedId,
    submitLater,
    toggleInbox,
    toggleStatusPanel,
    view,
  ])
}
