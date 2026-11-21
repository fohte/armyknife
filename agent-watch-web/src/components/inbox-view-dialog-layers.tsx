import { InboxFilterDialog } from '#components/inbox-filter-dialog'
import { SessionRowActionsSheet } from '#components/session-row-actions-sheet'
import type { Session } from '#types'

type InboxViewDialogLayersProps = {
  sessions: readonly Session[]
  mobile: boolean
  filterDialogOpen: boolean
  draftQuery: string
  onDraftQueryChange: (query: string) => void
  onApplyFilter: () => void
  onCloseFilter: () => void
  actionSheetSession: Session | null
  onLater?: ((id: string) => void) | undefined
  onCloseSession?: ((id: string) => void) | undefined
  onDismissActions: () => void
}

export function InboxViewDialogLayers({
  sessions,
  mobile,
  filterDialogOpen,
  draftQuery,
  onDraftQueryChange,
  onApplyFilter,
  onCloseFilter,
  actionSheetSession,
  onLater,
  onCloseSession,
  onDismissActions,
}: InboxViewDialogLayersProps) {
  return (
    <>
      {filterDialogOpen ? (
        <InboxFilterDialog
          sessions={sessions}
          mobile={mobile}
          query={draftQuery}
          onQueryChange={onDraftQueryChange}
          onApply={onApplyFilter}
          onClose={onCloseFilter}
        />
      ) : null}
      {actionSheetSession === null ? null : (
        <SessionRowActionsSheet
          session={actionSheetSession}
          onLater={() => {
            onDismissActions()
            onLater?.(actionSheetSession.id)
          }}
          onCloseSession={
            actionSheetSession.category === 'done' ||
            onCloseSession === undefined
              ? undefined
              : () => {
                  onDismissActions()
                  onCloseSession(actionSheetSession.id)
                }
          }
          onCancel={onDismissActions}
        />
      )}
    </>
  )
}
