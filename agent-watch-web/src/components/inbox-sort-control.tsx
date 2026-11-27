import '#components/inbox-sort.css'

import { Button } from '@fohte/ui/button'
import { Check, ChevronDown } from 'lucide-react'
import { useLayoutEffect, useRef } from 'react'

import { MobileSheet } from '#components/mobile-sheet'
import { useDismissableLayer } from '#components/use-dismissable-layer'
import type { InboxSort } from '#inbox-sort'

const OPTIONS: {
  value: InboxSort
  label: string
  hint: string
}[] = [
  { value: 'priority', label: 'Priority', hint: 'tq priority, then newest' },
  { value: 'updated', label: 'Updated', hint: 'Newest first' },
  { value: 'project', label: 'Project', hint: 'A to Z' },
]

type InboxSortControlProps = {
  sort: InboxSort
  storageAvailable: boolean
  isMobile: boolean
  open: boolean
  onOpenChange: (open: boolean) => void
  onSortChange: (sort: InboxSort) => void
}

export function InboxSortControl({
  sort,
  storageAvailable,
  isMobile,
  open,
  onOpenChange,
  onSortChange,
}: InboxSortControlProps) {
  const controlRef = useRef<HTMLDivElement>(null)
  const triggerRef = useRef<HTMLButtonElement>(null)
  const desktopMenuRef = useRef<HTMLElement>(null)
  const selectedLabel =
    OPTIONS.find((option) => option.value === sort)?.label ?? 'Priority'

  useDismissableLayer(!isMobile && open, onOpenChange, [controlRef])

  useLayoutEffect(() => {
    if (isMobile || !open) return

    desktopMenuRef.current
      ?.querySelector<HTMLButtonElement>(
        '[role="menuitemradio"][aria-checked="true"]',
      )
      ?.focus()
  }, [isMobile, open, sort])

  const options = (
    <div className="inbox-sort-options" role="menu" aria-label="Sort inbox">
      {OPTIONS.map((option) => (
        <Button
          key={option.value}
          type="button"
          variant="plain"
          role="menuitemradio"
          aria-checked={option.value === sort}
          className={`inbox-sort-option${option.value === sort ? ' is-selected' : ''}`}
          onClick={() => {
            onSortChange(option.value)
            triggerRef.current?.focus()
            onOpenChange(false)
          }}
        >
          <span className="inbox-sort-check" aria-hidden="true">
            {option.value === sort ? <Check size={14} /> : null}
          </span>
          <span>{option.label}</span>
          <span className="inbox-sort-hint">{option.hint}</span>
        </Button>
      ))}
    </div>
  )

  return (
    <div className="inbox-sort-control" ref={controlRef}>
      <Button
        type="button"
        variant="plain"
        className={`inbox-sort-trigger cursor-pointer${open ? ' is-open' : ''}`}
        aria-haspopup={isMobile ? 'dialog' : 'menu'}
        aria-expanded={open}
        aria-label={`Sort ${selectedLabel}`}
        ref={triggerRef}
        onClick={() => {
          onOpenChange(!open)
        }}
        onKeyDown={(event) => {
          if (
            !isMobile &&
            (event.key === 'ArrowDown' || event.key === 'ArrowUp')
          ) {
            event.preventDefault()
            onOpenChange(true)
          }
        }}
      >
        <span>Sort</span> <strong>{selectedLabel}</strong>
        <ChevronDown aria-hidden="true" size={14} />
      </Button>
      {!isMobile && open ? (
        <section
          className="inbox-sort-menu"
          aria-label="Sort inbox"
          ref={desktopMenuRef}
          onKeyDown={(event) => {
            if (event.key === 'Escape') {
              event.preventDefault()
              event.stopPropagation()
              triggerRef.current?.focus()
              onOpenChange(false)
              return
            }

            const options = Array.from(
              event.currentTarget.querySelectorAll<HTMLButtonElement>(
                '[role="menuitemradio"]',
              ),
            )
            if (options.length === 0) return

            const currentIndex = options.findIndex(
              (option) => option === event.target,
            )
            let nextIndex: number
            if (event.key === 'ArrowDown') {
              event.preventDefault()
              nextIndex =
                currentIndex < 0 ? 0 : (currentIndex + 1) % options.length
            } else if (event.key === 'ArrowUp') {
              event.preventDefault()
              nextIndex =
                currentIndex < 0
                  ? options.length - 1
                  : (currentIndex - 1 + options.length) % options.length
            } else if (event.key === 'Home') {
              event.preventDefault()
              nextIndex = 0
            } else if (event.key === 'End') {
              event.preventDefault()
              nextIndex = options.length - 1
            } else {
              return
            }

            options[nextIndex]?.focus()
          }}
        >
          {options}
          <p className="inbox-sort-note">
            {storageAvailable
              ? 'Sections stay. Saved in this browser.'
              : 'Sections stay. This browser could not save the selection.'}
          </p>
        </section>
      ) : null}
      {isMobile && open ? (
        <MobileSheet
          label="Sort"
          className="inbox-sort-sheet"
          onClose={() => {
            onOpenChange(false)
          }}
        >
          <h2>Sort</h2>
          {options}
          <p className="inbox-sort-note">
            {storageAvailable
              ? 'Sections stay. Saved in this browser.'
              : 'Sections stay. This browser could not save the selection.'}
          </p>
          <Button
            type="button"
            variant="plain"
            className="inbox-sort-cancel"
            onClick={() => {
              onOpenChange(false)
            }}
          >
            Cancel
          </Button>
        </MobileSheet>
      ) : null}
    </div>
  )
}
