import '#components/inbox-save-view-sheet.css'

import { Button } from '@fohte/ui/button'
import { useRef } from 'react'

import { useFocusTrap } from '#components/use-focus-trap'
import { INBOX_FILTER_FIELDS, parseInboxFilterQuery } from '#inbox-filter'

type InboxSaveViewSheetProps = {
  query: string
  name: string
  onNameChange: (name: string) => void
  onSave: (name: string) => void
  onCancel: () => void
}

export function InboxSaveViewSheet({
  query,
  name,
  onNameChange,
  onSave,
  onCancel,
}: InboxSaveViewSheetProps) {
  const sheetRef = useRef<HTMLFormElement>(null)
  const nameRef = useRef<HTMLInputElement>(null)
  useFocusTrap({
    containerRef: sheetRef,
    initialFocusRef: nameRef,
    isActive: true,
  })
  const summary = parseInboxFilterQuery(query)
    .map((filter) => {
      const label =
        INBOX_FILTER_FIELDS.find((field) => field.key === filter.key)?.label ??
        filter.key
      return `${label} ${filter.values.join(', ')}`
    })
    .join(' · ')

  return (
    <div
      className="inbox-save-view-overlay"
      onClick={(event) => {
        if (event.target === event.currentTarget) onCancel()
      }}
    >
      <form
        aria-label="Save view"
        aria-modal="true"
        className="inbox-save-view-sheet"
        ref={sheetRef}
        tabIndex={-1}
        onSubmit={(event) => {
          event.preventDefault()
          onSave(name)
        }}
        role="dialog"
      >
        <h2>Save view</h2>
        <p className="inbox-save-view-summary">{summary}</p>
        <label>
          Name
          <input
            autoFocus
            aria-label="Saved view name"
            ref={nameRef}
            maxLength={60}
            onChange={(event) => {
              onNameChange(event.currentTarget.value)
            }}
            placeholder="Name this view"
            value={name}
          />
        </label>
        <p className="inbox-save-view-hint">
          Saved views appear under Views in the menu.
        </p>
        <div className="inbox-save-view-sheet-actions">
          <Button type="button" variant="plain" onClick={onCancel}>
            Cancel
          </Button>
          <Button type="submit" variant="plain" disabled={name.trim() === ''}>
            Save
          </Button>
        </div>
      </form>
    </div>
  )
}
