import type { ReactNode } from 'react'

import { formatLastUsed } from '#components/format-last-used'
import type { SessionDirectory } from '#new-session-api'

type SessionDirectoryOptionsProps = {
  directories: readonly SessionDirectory[]
  error?: string | null
  isLoading: boolean
  query: string
  autoFocus?: boolean
  activeIndex?: number
  now?: Date
  afterSearchInput?: ReactNode
  onQueryChange: (query: string) => void
  onSelect: (directory: SessionDirectory) => void
  onActiveIndexChange?: (index: number) => void
}

const SOURCES: ReadonlyArray<{
  source: SessionDirectory['source']
  label: string
}> = [
  { source: 'recent', label: 'RECENT' },
  { source: 'repo', label: 'PROJECTS' },
  { source: 'path', label: 'PATHS' },
]

export function SessionDirectoryOptions({
  directories,
  error = null,
  isLoading,
  query,
  autoFocus = false,
  activeIndex = 0,
  now,
  afterSearchInput,
  onQueryChange,
  onSelect,
  onActiveIndexChange = () => {},
}: SessionDirectoryOptionsProps) {
  const currentTime = now ?? new Date()

  return (
    <div className="session-directory-options">
      <label className="session-directory-search">
        <span aria-hidden="true">⌕</span>
        <input
          aria-label="Search directories"
          autoFocus={autoFocus}
          autoComplete="off"
          onChange={(event) => {
            onQueryChange(event.currentTarget.value)
            onActiveIndexChange(0)
          }}
          placeholder="Search directories or enter a path"
          value={query}
        />
      </label>
      {afterSearchInput}
      <div
        aria-label="Directories"
        className="session-directory-results"
        role="listbox"
      >
        {error !== null && !isLoading ? (
          <p className="session-directory-feedback" role="alert">
            {error}
          </p>
        ) : isLoading ? (
          <p className="session-directory-feedback" role="status">
            Loading directories…
          </p>
        ) : directories.length === 0 ? (
          <p className="session-directory-feedback">No matching directories</p>
        ) : (
          SOURCES.map(({ source, label }) => {
            const options = directories.filter(
              (directory) => directory.source === source,
            )
            if (options.length === 0) return null
            return (
              <section className="session-directory-group" key={source}>
                <h2>{label}</h2>
                {options.map((directory) => {
                  const index = directories.indexOf(directory)
                  const title = directory.path.split('/').filter(Boolean).at(-1)
                  const lastUsed =
                    directory.source === 'recent' &&
                    directory.lastUsedAt !== undefined
                      ? formatLastUsed(directory.lastUsedAt, currentTime)
                      : null
                  return (
                    <button
                      key={`${directory.source}:${directory.path}`}
                      aria-label={`Select directory ${directory.path}`}
                      aria-selected={activeIndex === index}
                      className="session-directory-option"
                      onClick={() => {
                        onSelect(directory)
                      }}
                      onMouseEnter={() => {
                        onActiveIndexChange(index)
                      }}
                      role="option"
                      type="button"
                    >
                      <span className="session-directory-option-title-row">
                        <span className="session-directory-option-title">
                          {title ?? directory.path}
                        </span>
                        {lastUsed === null ? null : (
                          <span className="session-directory-option-when">
                            {lastUsed}
                          </span>
                        )}
                      </span>
                      <span className="session-directory-option-path is-monospace">
                        {directory.path}
                      </span>
                    </button>
                  )
                })}
              </section>
            )
          })
        )}
      </div>
    </div>
  )
}
