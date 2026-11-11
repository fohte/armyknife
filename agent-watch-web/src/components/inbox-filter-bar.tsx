import '#components/inbox-filter.css'

import { Button } from '@fohte/ui/button'
import { Filter } from 'lucide-react'

import {
  formatInboxFilterQuery,
  INBOX_FILTER_FIELDS,
  type InboxFilter,
  type InboxFilterKey,
} from '#inbox-filter'

type InboxFilterBarProps = {
  filters: readonly InboxFilter[]
  matchingNeedsActionCount: number
  totalNeedsActionCount: number
  onFilterQueryChange: (query: string) => void
  onOpenFilterDialog: () => void
  saveViewOpen?: boolean
  saveViewName?: string
  onSaveViewOpenChange?: ((open: boolean) => void) | undefined
  onSaveViewNameChange?: ((name: string) => void) | undefined
  onSaveView?: ((name: string) => void) | undefined
}

export function InboxFilterBar({
  filters,
  matchingNeedsActionCount,
  totalNeedsActionCount,
  onFilterQueryChange,
  onOpenFilterDialog,
  saveViewOpen = false,
  saveViewName = '',
  onSaveViewOpenChange,
  onSaveViewNameChange,
  onSaveView,
}: InboxFilterBarProps) {
  const removeFilter = (key: InboxFilterKey) => {
    onFilterQueryChange(
      formatInboxFilterQuery(filters.filter((filter) => filter.key !== key)),
    )
  }

  return (
    <div className="inbox-filter-bar">
      {filters.length === 0 ? (
        <Button
          type="button"
          variant="plain"
          className="inbox-filter-open cursor-pointer"
          onClick={onOpenFilterDialog}
        >
          <Filter aria-hidden="true" size={14} />
          <span>Filter by project, kind, tq task, repo</span>
          <kbd>/</kbd>
        </Button>
      ) : (
        <>
          <div aria-label="Active inbox filters" className="inbox-filter-chips">
            {filters.map((filter) => {
              const field = INBOX_FILTER_FIELDS.find(
                (candidate) => candidate.key === filter.key,
              )
              const label = field?.label ?? filter.key
              const values = filter.values
                .map((value) =>
                  filter.key === 'task' && !value.startsWith('#')
                    ? `#${value}`
                    : value,
                )
                .join(', ')
              return (
                <span key={filter.key} className="inbox-filter-chip">
                  <span className="inbox-filter-chip-label">{label}</span>
                  <span className="inbox-filter-chip-value">{values}</span>
                  <Button
                    type="button"
                    variant="plain"
                    aria-label={`Remove ${label} filter`}
                    className="inbox-filter-chip-remove"
                    onClick={() => {
                      removeFilter(filter.key)
                    }}
                  >
                    ×
                  </Button>
                </span>
              )
            })}
          </div>
          <Button
            type="button"
            variant="plain"
            className="inbox-filter-add cursor-pointer"
            onClick={onOpenFilterDialog}
          >
            + Filter
          </Button>
          <span className="inbox-filter-result-count">
            {matchingNeedsActionCount} of {totalNeedsActionCount}
          </span>
          {onSaveView !== undefined ? (
            <div>
              {saveViewOpen ? (
                <form
                  className="inbox-save-view-form"
                  onSubmit={(event) => {
                    event.preventDefault()
                    onSaveView(saveViewName)
                  }}
                >
                  <input
                    aria-label="Saved view name"
                    autoFocus
                    maxLength={60}
                    onChange={(event) => {
                      onSaveViewNameChange?.(event.currentTarget.value)
                    }}
                    placeholder="Name this view"
                    value={saveViewName}
                  />
                  <Button
                    type="submit"
                    variant="plain"
                    className="inbox-save-view-submit cursor-pointer"
                    disabled={saveViewName.trim().length === 0}
                  >
                    Save
                  </Button>
                  <Button
                    type="button"
                    variant="plain"
                    className="inbox-save-view-cancel cursor-pointer"
                    onClick={() => {
                      onSaveViewOpenChange?.(false)
                    }}
                  >
                    Cancel
                  </Button>
                </form>
              ) : (
                <Button
                  type="button"
                  variant="plain"
                  className="inbox-save-view aw-button--outline cursor-pointer"
                  onClick={() => {
                    onSaveViewOpenChange?.(true)
                  }}
                >
                  Save view
                </Button>
              )}
            </div>
          ) : null}
        </>
      )}
    </div>
  )
}
