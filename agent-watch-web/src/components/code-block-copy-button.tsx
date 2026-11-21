import type { CopyStatus } from '#components/use-copy-to-clipboard'

type CodeBlockCopyButtonProps = {
  status: CopyStatus
  onCopy: () => void
}

export function CodeBlockCopyButton({
  status,
  onCopy,
}: CodeBlockCopyButtonProps) {
  const accessibleStatus =
    status === 'copied'
      ? 'Code copied'
      : status === 'failed'
        ? 'Could not copy code'
        : ''
  const buttonLabel =
    status === 'copied'
      ? '✓ Copied'
      : status === 'failed'
        ? 'Copy failed'
        : 'Copy'

  return (
    <>
      <button
        className="syntax-code-block-copy-button"
        type="button"
        aria-label={status === 'idle' ? 'Copy code' : accessibleStatus}
        title="Copy code"
        onClick={onCopy}
      >
        {buttonLabel}
      </button>
      {accessibleStatus === '' ? null : (
        <span className="sr-only" role="status">
          {accessibleStatus}
        </span>
      )}
    </>
  )
}
