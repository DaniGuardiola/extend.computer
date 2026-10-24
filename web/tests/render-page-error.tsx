// Browser-test fixture. No production route or runtime preview branch.
import { createRoot } from 'react-dom/client'
import { PageError } from '../src/components/PageError'
export function renderPageError() {
  const main = document.querySelector('main')!
  const container = document.createElement('div')
  main.replaceWith(container)
  createRoot(container).render(<PageError />)
}
