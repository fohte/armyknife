import '#components/inbox-save-view-sheet.css'

import { Button } from '@fohte/ui/button'
import { useRef } from 'react'

import { MobileSheet } from '#components/mobile-sheet'
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
  const nameRef = useRef<HTMLInputElement>(null)
  const summary = parseInboxFilterQuery(query)
    .map((filter) => {
      const label =
        INBOX_FILTER_FIELDS.find((field) => field.key === filter.key)?.label ??
        filter.key
      return `${label} ${filter.values.join(', ')}`
    })
    .join(' · ')

  return (
    <MobileSheet label="Save view" initialFocusRef={nameRef} onClose={onCancel}>
      <form
        className="inbox-save-view-sheet"
        onSubmit={(event) => {
          event.preventDefault()
          onSave(name)
        }}
      >
        <h2>Save view</h2>
        <p className="inbox-save-view-summary">{summary}</p>
        <label>
          Name
          <input
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
    </MobileSheet>
  )
}
