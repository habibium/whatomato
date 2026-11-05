import { useEffect } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { useNavigate } from '@tanstack/react-router'

import { setSessionUser } from '@/lib/session'
import { ApiError } from '@/lib/whatsapp'

export function useSessionExpiry(error: Error | null, returnTo: string) {
  const queryClient = useQueryClient()
  const navigate = useNavigate()
  const isSignedOut = error instanceof ApiError && error.status === 401

  useEffect(() => {
    if (!isSignedOut) return
    setSessionUser(queryClient, null)
    void navigate({ to: '/signin', search: { redirect: returnTo } })
  }, [isSignedOut, navigate, queryClient, returnTo])

  return isSignedOut
}
