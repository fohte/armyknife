import '#components/review-document-preview.css'

export function ReviewDocumentIssueReference({
  reference,
  title,
}: {
  reference: string
  title: string | undefined
}) {
  const match = /^([^/]+)\/([^/#]+)#(\d+)$/.exec(reference)
  const owner = match?.[1]
  const repository = match?.[2]
  const issueNumber = match?.[3]
  const href =
    owner === undefined || repository === undefined || issueNumber === undefined
      ? null
      : `https://github.com/${encodeURIComponent(owner)}/${encodeURIComponent(repository)}/issues/${issueNumber}`
  const displayTitle = typeof title === 'string' ? title : undefined

  return (
    <>
      {href === null ? (
        <code>{reference}</code>
      ) : (
        <a
          className="review-document-issue-reference-link"
          href={href}
          rel="noopener noreferrer"
          target="_blank"
        >
          {reference}
        </a>
      )}
      {displayTitle === undefined ? null : (
        <span
          className="review-document-issue-reference-title"
          title={displayTitle}
        >
          {displayTitle}
        </span>
      )}
    </>
  )
}
