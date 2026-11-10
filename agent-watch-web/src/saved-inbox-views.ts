import { Result } from 'neverthrow'

export type SavedInboxView = {
  id: string
  name: string
  query: string
}

export const SAVED_INBOX_VIEWS_STORAGE_KEY = 'agent-watch-web:saved-inbox-views'

export type SavedInboxViewsRead = {
  views: SavedInboxView[]
  storageAvailable: boolean
}

function isStoredInboxView(
  value: unknown,
): value is Omit<SavedInboxView, 'id'> {
  return (
    typeof value === 'object' &&
    value !== null &&
    'name' in value &&
    typeof value.name === 'string' &&
    'query' in value &&
    typeof value.query === 'string'
  )
}

export function readSavedInboxViews(storage?: Storage): SavedInboxViewsRead {
  return Result.fromThrowable(
    () =>
      (storage ?? window.localStorage).getItem(SAVED_INBOX_VIEWS_STORAGE_KEY),
    (error: unknown) => error,
  )().match(
    (raw) => {
      if (raw === null) return { views: [], storageAvailable: true }
      return Result.fromThrowable(
        () => JSON.parse(raw) as unknown,
        (error: unknown) => error,
      )().match(
        (value) =>
          Array.isArray(value)
            ? {
                views: value.flatMap((entry, index) =>
                  isStoredInboxView(entry)
                    ? [{ ...entry, id: String(index) }]
                    : [],
                ),
                storageAvailable: true,
              }
            : { views: [], storageAvailable: false },
        () => ({ views: [], storageAvailable: false }),
      )
    },
    () => ({ views: [], storageAvailable: false }),
  )
}

export function writeSavedInboxViews(
  views: readonly SavedInboxView[],
  storage?: Storage,
) {
  return Result.fromThrowable(
    () => {
      const targetStorage = storage ?? window.localStorage
      targetStorage.setItem(
        SAVED_INBOX_VIEWS_STORAGE_KEY,
        JSON.stringify(views.map(({ name, query }) => ({ name, query }))),
      )
    },
    (error: unknown) => error,
  )().isOk()
}
