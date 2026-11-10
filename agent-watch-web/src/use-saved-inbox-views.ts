import { useCallback, useEffect, useMemo, useState } from 'react'

import { filterInboxSessions, parseInboxFilterQuery } from '#inbox-filter'
import {
  readSavedInboxViews,
  SAVED_INBOX_VIEWS_STORAGE_KEY,
  type SavedInboxView,
  writeSavedInboxViews,
} from '#saved-inbox-views'
import type { Session } from '#types'

type SavedViewNavItem = SavedInboxView & {
  count: number
  countTone: 'needs-action' | 'error' | null
}

export function useSavedInboxViews(
  sessions: readonly Session[],
  normalizedQuery: string,
  isInbox: boolean,
  setError: (error: string | null) => void,
) {
  const [storageState, setStorageState] = useState(readSavedInboxViews)
  const [selectedViewId, setSelectedViewId] = useState<string | null>(null)
  const [saveViewOpen, setSaveViewOpen] = useState(false)
  const [saveViewName, setSaveViewName] = useState('')
  const savedViews = storageState.views

  useEffect(() => {
    const onStorage = (event: StorageEvent) => {
      if (event.key !== SAVED_INBOX_VIEWS_STORAGE_KEY && event.key !== null) {
        return
      }
      setStorageState(readSavedInboxViews())
      setSelectedViewId(null)
    }
    window.addEventListener('storage', onStorage)
    return () => {
      window.removeEventListener('storage', onStorage)
    }
  }, [])

  const savedViewsForNav = useMemo<SavedViewNavItem[]>(
    () =>
      savedViews.map((savedView) => {
        const matchingSessions = filterInboxSessions(
          sessions,
          parseInboxFilterQuery(savedView.query),
        )
        const hasError = matchingSessions.some(
          (session) => session.kind?.toLocaleLowerCase() === 'error',
        )
        const hasNeedsAction = matchingSessions.some(
          (session) => session.category === 'needs_action',
        )
        return {
          ...savedView,
          count: matchingSessions.length,
          countTone: hasError
            ? 'error'
            : hasNeedsAction
              ? 'needs-action'
              : null,
        }
      }),
    [savedViews, sessions],
  )
  const activeSavedViewId = isInbox
    ? (savedViews.find(
        (savedView) =>
          savedView.id === selectedViewId &&
          savedView.query === normalizedQuery,
      )?.id ??
      savedViews.find((savedView) => savedView.query === normalizedQuery)?.id ??
      null)
    : null

  const saveView = useCallback(
    (name: string) => {
      const trimmedName = name.trim()
      if (trimmedName.length === 0 || normalizedQuery.length === 0) return

      const current = readSavedInboxViews()
      if (!current.storageAvailable) {
        setError('Could not save the view.')
        return
      }
      const savedView = {
        id: String(current.views.length),
        name: trimmedName,
        query: normalizedQuery,
      }
      const nextViews = [...current.views, savedView]
      if (!writeSavedInboxViews(nextViews)) {
        setError('Could not save the view.')
        return
      }
      setStorageState({ views: nextViews, storageAvailable: true })
      setSelectedViewId(savedView.id)
      setSaveViewName('')
      setSaveViewOpen(false)
      setError(null)
    },
    [normalizedQuery, setError],
  )

  const deleteView = useCallback(
    (id: string) => {
      const targetIndex = savedViews.findIndex((view) => view.id === id)
      const target = savedViews[targetIndex]
      if (target === undefined) return

      const current = readSavedInboxViews()
      if (!current.storageAvailable) {
        setError('Could not delete the view.')
        return
      }
      const occurrence = savedViews
        .slice(0, targetIndex)
        .filter(
          (view) => view.name === target.name && view.query === target.query,
        ).length
      const matchingIndexes = current.views.flatMap((view, index) =>
        view.name === target.name && view.query === target.query ? [index] : [],
      )
      const indexToDelete = matchingIndexes[occurrence]
      if (indexToDelete === undefined) {
        setStorageState(current)
        return
      }
      const nextViews = current.views
        .filter((_, index) => index !== indexToDelete)
        .map((view, index) => ({ ...view, id: String(index) }))
      if (!writeSavedInboxViews(nextViews)) {
        setError('Could not delete the view.')
        return
      }
      setStorageState({ views: nextViews, storageAvailable: true })
      setSelectedViewId(null)
      setError(null)
    },
    [savedViews, setError],
  )

  const selectView = useCallback(
    (id: string) => {
      const savedView = savedViews.find((view) => view.id === id)
      if (savedView === undefined) return null
      setSelectedViewId(id)
      setSaveViewOpen(false)
      return savedView.query
    },
    [savedViews],
  )

  return {
    activeSavedViewId,
    deleteView,
    saveView,
    saveViewName,
    saveViewOpen,
    savedViews,
    savedViewsForNav,
    selectView,
    setSaveViewName,
    setSaveViewOpen,
    storageError: storageState.storageAvailable
      ? null
      : 'Could not load saved views from browser storage.',
  }
}
