import '#components/crit-pane.css'

import { Button } from '@fohte/ui/button'

type CritPaneProps = {
  url: string
  onClose: () => void
}

export function CritPane({ url, onClose }: CritPaneProps) {
  return (
    <aside className="crit-pane" aria-label="crit" id="crit-pane">
      <header className="crit-pane-header">
        <span className="crit-pane-title">crit</span>
        <a href={url} target="_blank" rel="noreferrer">
          {url}
        </a>
        <Button
          type="button"
          variant="plain"
          aria-label="Close crit"
          onClick={onClose}
        >
          ×
        </Button>
      </header>
      <iframe title="Crit review" src={url} />
    </aside>
  )
}
