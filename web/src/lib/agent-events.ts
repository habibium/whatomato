import { useEffect } from 'react'
import { useQueryClient } from '@tanstack/react-query'

/**
 * Listens to the server while a signed-in page is open, so a request an agent makes shows at once
 * rather than at the next poll. Where agent access is off the stream answers 404 and the browser
 * gives up on it.
 */
export function useAgentEvents() {
  const queryClient = useQueryClient()

  useEffect(() => {
    if (typeof EventSource === 'undefined') return
    const source = new EventSource('/api/agents/events')
    const refresh = () =>
      void queryClient.invalidateQueries({ queryKey: ['agents'] })
    let hasOpened = false
    source.addEventListener('approvals', refresh)
    source.addEventListener('open', () => {
      // Whatever happened while the connection was down went unheard
      if (hasOpened) refresh()
      hasOpened = true
    })
    return () => source.close()
  }, [queryClient])
}
