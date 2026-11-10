import { expect, it } from 'vitest'

import {
  readSavedInboxViews,
  type SavedInboxView,
  writeSavedInboxViews,
} from '#saved-inbox-views'

class MemoryStorage implements Storage {
  private readonly values = new Map<string, string>()

  get length() {
    return this.values.size
  }

  clear() {
    this.values.clear()
  }

  getItem(key: string) {
    return this.values.get(key) ?? null
  }

  key(index: number) {
    return [...this.values.keys()][index] ?? null
  }

  removeItem(key: string) {
    this.values.delete(key)
  }

  setItem(key: string, value: string) {
    this.values.set(key, value)
  }
}

it('stores only the name and query and reconstructs view identities when reading', () => {
  const storage = new MemoryStorage()
  const views: SavedInboxView[] = [
    {
      id: 'sample-view',
      name: 'Sample reviews',
      query: 'project:sample-project kind:review',
    },
  ]

  const written = writeSavedInboxViews(views, storage)
  const snapshot = () => ({
    written,
    stored: storage.getItem('agent-watch-web:saved-inbox-views'),
    restored: readSavedInboxViews(storage),
  })

  expect(snapshot()).toEqual({
    written: true,
    stored:
      '[{"name":"Sample reviews","query":"project:sample-project kind:review"}]',
    restored: [
      {
        id: '0',
        name: 'Sample reviews',
        query: 'project:sample-project kind:review',
      },
    ],
  })
})

it('ignores malformed stored view data', () => {
  const storage = new MemoryStorage()
  storage.setItem('agent-watch-web:saved-inbox-views', '[{"name":4}]')

  expect(readSavedInboxViews(storage)).toEqual([])
})
