import { Button } from '@fohte/ui/button'
import { useEffect, useRef, useState } from 'react'

import {
  formatInboxFilterQuery,
  getInboxFilterOptions,
  INBOX_FILTER_FIELDS,
  type InboxFilterKey,
  normalizeInboxFilterValue,
  parseInboxFilterQuery,
} from '#inbox-filter'
import type { Session } from '#types'

type InboxFilterDialogProps = {
  sessions: readonly Session[]
  query: string
  initialSelectedKey?: InboxFilterKey
  onQueryChange: (query: string) => void
  onApply: () => void
  onClose: () => void
}

export function InboxFilterDialog({
  sessions,
  query,
  initialSelectedKey = 'project',
  onQueryChange,
  onApply,
  onClose,
}: InboxFilterDialogProps) {
  const [selectedKey, setSelectedKey] =
    useState<InboxFilterKey>(initialSelectedKey)
  const inputRef = useRef<HTMLInputElement>(null)
  const selectedField = INBOX_FILTER_FIELDS.find(
    (field) => field.key === selectedKey,
  )
  const draftFilters = parseInboxFilterQuery(query)
  const selectedValues =
    draftFilters.find((filter) => filter.key === selectedKey)?.values ?? []
  const options = getInboxFilterOptions(sessions, draftFilters, selectedKey)
  useEffect(() => {
    inputRef.current?.focus()
  }, [])

  function toggleValue(value: string) {
    const filters = parseInboxFilterQuery(query)
    const current = filters.find((filter) => filter.key === selectedKey)
    const nextValues = (current?.values ?? []).filter(
      (currentValue) =>
        normalizeInboxFilterValue(selectedKey, currentValue) !==
        normalizeInboxFilterValue(selectedKey, value),
    )
    if (nextValues.length === (current?.values.length ?? 0)) {
      nextValues.push(value)
    }
    const nextFilters = filters.filter((filter) => filter.key !== selectedKey)
    if (nextValues.length > 0) {
      nextFilters.push({ key: selectedKey, values: nextValues })
    }
    onQueryChange(formatInboxFilterQuery(nextFilters))
  }

  return (
    <div
      className="inbox-filter-overlay"
      onClick={onClose}
      onKeyDown={(event) => {
        if (event.key === 'Escape') {
          event.preventDefault()
          onClose()
        }
      }}
    >
      <section
        aria-label="Filter inbox"
        aria-modal="true"
        className="inbox-filter-dialog"
        role="dialog"
        onClick={(event) => {
          event.stopPropagation()
        }}
      >
        <form
          className="inbox-filter-search-row"
          onSubmit={(event) => {
            event.preventDefault()
            onApply()
          }}
        >
          <input
            ref={inputRef}
            aria-label="Inbox filter query"
            className="inbox-filter-query"
            onChange={(event) => {
              onQueryChange(event.currentTarget.value)
            }}
            placeholder="kind:"
            value={query}
          />
          <span className="inbox-filter-key-help">
            <kbd>↵</kbd> apply · <kbd>esc</kbd> close
          </span>
        </form>
        <div className="inbox-filter-picker">
          <nav aria-label="Filter fields" className="inbox-filter-fields">
            {INBOX_FILTER_FIELDS.map((field) => (
              <Button
                key={field.key}
                type="button"
                variant="plain"
                aria-current={selectedKey === field.key ? 'true' : undefined}
                className={`inbox-filter-field${selectedKey === field.key ? ' is-selected aw-button--pressed' : ''}`}
                onClick={() => {
                  setSelectedKey(field.key)
                }}
              >
                <span>{field.label}</span>
                <code>{field.prefix}</code>
              </Button>
            ))}
          </nav>
          <div
            aria-label={`${selectedField?.label ?? ''} values`}
            className="inbox-filter-options"
            role="group"
          >
            {options.length > 0 ? (
              options.map((option) => {
                const checked = selectedValues.some(
                  (value) =>
                    normalizeInboxFilterValue(selectedKey, value) ===
                    normalizeInboxFilterValue(selectedKey, option.value),
                )
                return (
                  <label key={option.value} className="inbox-filter-option">
                    <input
                      checked={checked}
                      onChange={() => {
                        toggleValue(option.value)
                      }}
                      type="checkbox"
                    />
                    <span
                      className="inbox-filter-option-label"
                      data-tone={
                        selectedKey === 'kind'
                          ? option.value.toLowerCase()
                          : undefined
                      }
                    >
                      {option.value}
                    </span>
                    <span className="inbox-filter-option-count">
                      {option.count}
                    </span>
                  </label>
                )
              })
            ) : (
              <p className="inbox-filter-no-values">No values available</p>
            )}
          </div>
        </div>
        <p className="inbox-filter-example">
          Type to filter:{' '}
          <code>
            project:web-app kind:review,question task:#123 has:decision
          </code>
        </p>
      </section>
    </div>
  )
}
