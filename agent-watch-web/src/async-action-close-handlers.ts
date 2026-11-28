import type { AsyncAction, StartAsyncActionOptions } from '#async-action-model'

type CloseActionHandlerOptions = Pick<
  StartAsyncActionOptions,
  | 'onCloseKeepWorktree'
  | 'onDiscardChanges'
  | 'onOpenInTerminal'
  | 'onReviewDiscardAgain'
>

export function createCloseActionHandlers() {
  const closeWithWorktreeHandlers = new Map<number, () => void>()
  const discardChangesHandlers = new Map<
    number,
    (action: AsyncAction) => void
  >()
  const reviewDiscardAgainHandlers = new Map<
    number,
    (action: AsyncAction) => void
  >()
  const openInTerminalHandlers = new Map<number, () => void>()

  return {
    register(id: number, options: CloseActionHandlerOptions) {
      if (options.onCloseKeepWorktree !== undefined) {
        closeWithWorktreeHandlers.set(id, options.onCloseKeepWorktree)
      }
      if (options.onDiscardChanges !== undefined) {
        discardChangesHandlers.set(id, options.onDiscardChanges)
      }
      if (options.onReviewDiscardAgain !== undefined) {
        reviewDiscardAgainHandlers.set(id, options.onReviewDiscardAgain)
      }
      if (options.onOpenInTerminal !== undefined) {
        openInTerminalHandlers.set(id, options.onOpenInTerminal)
      }
    },
    delete(id: number) {
      closeWithWorktreeHandlers.delete(id)
      discardChangesHandlers.delete(id)
      reviewDiscardAgainHandlers.delete(id)
      openInTerminalHandlers.delete(id)
    },
    closeWithWorktree(id: number) {
      closeWithWorktreeHandlers.get(id)?.()
    },
    discardChanges(id: number, actions: readonly AsyncAction[]) {
      const action = actions.find((candidate) => candidate.id === id)
      if (action !== undefined) discardChangesHandlers.get(id)?.(action)
    },
    reviewDiscardAgain(id: number, actions: readonly AsyncAction[]) {
      const action = actions.find((candidate) => candidate.id === id)
      if (action !== undefined) reviewDiscardAgainHandlers.get(id)?.(action)
    },
    openInTerminal(id: number) {
      openInTerminalHandlers.get(id)?.()
    },
  }
}
