import { useState } from 'react'
import { Check, Download } from 'lucide-react'

export function DownloadButton({
  value,
  filename,
}: {
  value: string
  filename: string
}) {
  const [downloads, setDownloads] = useState(0)
  return (
    <button
      type="button"
      className="text-button link-button"
      aria-label={downloads ? 'Downloaded!' : 'Download codes'}
      onClick={() => {
        const url = URL.createObjectURL(
          new Blob([value], { type: 'text/plain' }),
        )
        const link = document.createElement('a')
        link.href = url
        link.download = filename
        link.click()
        URL.revokeObjectURL(url)
        setDownloads((count) => count + 1)
      }}
    >
      {downloads ? (
        <Check key={`check-${downloads}`} size={16} aria-hidden="true" />
      ) : (
        <Download size={16} aria-hidden="true" />
      )}
      <span role="status" key={`label-${downloads}`}>
        {downloads ? 'Downloaded!' : 'Download codes'}
      </span>
    </button>
  )
}
