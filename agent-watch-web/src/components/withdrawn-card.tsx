import '#components/withdrawn-card.css'

type WithdrawnCardProps = {
  kind: 'To-do' | 'Proposal'
  title: string
}

export function WithdrawnCard({ kind, title }: WithdrawnCardProps) {
  return (
    <article
      className="chat-withdrawn-card"
      data-resolution="withdrawn"
      aria-label={
        kind === 'To-do' ? 'User task, withdrawn' : 'Proposal, withdrawn'
      }
    >
      <h2>{kind} · withdrawn</h2>
      <h3>{title}</h3>
    </article>
  )
}
