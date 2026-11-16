import { Outlet, createFileRoute, redirect } from '@tanstack/react-router'

import { useAgentEvents } from '@/lib/agent-events'
import { currentUser } from '@/lib/session'

/** Pathless layout: every route under `_authenticated/` requires a session. */
export const Route = createFileRoute('/_authenticated')({
  beforeLoad: async ({ context, location }) => {
    const user = await currentUser(context.queryClient)
    if (!user) {
      throw redirect({ to: '/signin', search: { redirect: location.href } })
    }
    return { user }
  },
  component: Authenticated,
})

function Authenticated() {
  useAgentEvents()
  return <Outlet />
}
