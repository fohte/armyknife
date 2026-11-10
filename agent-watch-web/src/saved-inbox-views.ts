import { Result } from 'neverthrow'

export type SavedInboxView = {
  id: string
  name: string
  query: string
}

const storageKey = 'agent-watch-web:saved-inbox-views'

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

export function readSavedInboxViews(storage: Storage = window.localStorage) {
  return Result.fromThrowable(
    () => storage.getItem(storageKey),
    (error: unknown) => error,
  )().match(
    (raw) => {
      if (raw === null) return []
      return Result.fromThrowable(
        () => JSON.parse(raw) as unknown,
        (error: unknown) => error,
      )().match(
        (value) =>
          Array.isArray(value)
            ? value.flatMap((entry, index) =>
                isStoredInboxView(entry)
                  ? [{ ...entry, id: String(index) }]
                  : [],
              )
            : [],
        () => [],
      )
    },
    () => [],
  )
}

export function writeSavedInboxViews(
  views: readonly SavedInboxView[],
  storage: Storage = window.localStorage,
) {
  return Result.fromThrowable(
    () => {
      storage.setItem(
        storageKey,
        JSON.stringify(views.map(({ name, query }) => ({ name, query }))),
      )
    },
    (error: unknown) => error,
  )().isOk()
}
