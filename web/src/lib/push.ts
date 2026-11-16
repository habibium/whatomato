import { queryOptions } from '@tanstack/react-query'

import type { components } from '@/api/schema'

import { api } from '@/api/client'
import { ApiError } from '@/lib/whatsapp'

export type PushStatus = components['schemas']['PushStatus']

export const pushQuery = queryOptions({
  queryKey: ['push'],
  queryFn: async (): Promise<PushStatus> => {
    const { data, response } = await api.GET('/api/push')
    if (!data) throw new ApiError(response.status)
    return data
  },
  retry: (failures, error) =>
    !(error instanceof ApiError && error.status < 500) && failures < 2,
})

/** Whether this browser can show notifications from a website at all. */
export function isPushSupported() {
  return (
    'serviceWorker' in navigator &&
    'PushManager' in window &&
    'Notification' in window
  )
}

function toBytes(base64url: string) {
  const base64 = base64url.replaceAll('-', '+').replaceAll('_', '/')
  return Uint8Array.from(atob(base64), (char) => char.charCodeAt(0))
}

/** What to call this browser in the list, such as "Chrome on Linux". */
function browserLabel() {
  const agent = navigator.userAgent
  const browser = /Edg\//.test(agent)
    ? 'Edge'
    : /OPR\//.test(agent)
      ? 'Opera'
      : /Firefox\//.test(agent)
        ? 'Firefox'
        : /Chrome\//.test(agent)
          ? 'Chrome'
          : /Safari\//.test(agent)
            ? 'Safari'
            : 'A browser'
  const system = /Android/.test(agent)
    ? 'Android'
    : /iPhone|iPad|iPod/.test(agent)
      ? 'iOS'
      : /Windows/.test(agent)
        ? 'Windows'
        : /Mac OS X/.test(agent)
          ? 'macOS'
          : /Linux/.test(agent)
            ? 'Linux'
            : null
  return system ? `${browser} on ${system}` : browser
}

/** This browser's subscription with its push service, if it has one. */
export async function currentSubscription(): Promise<PushSubscription | null> {
  if (!isPushSupported()) return null
  const registration = await navigator.serviceWorker.getRegistration('/')
  return (await registration?.pushManager.getSubscription()) ?? null
}

/**
 * Whether this browser gets this account's notifications. A subscription the server doesn't list for
 * the account is dropped: it was another account's, or made before the server's key changed.
 */
export async function reconcilePush(status: PushStatus): Promise<boolean> {
  const subscription = await currentSubscription().catch(() => null)
  if (!subscription) return false
  if (
    status.subscriptions.some((item) => item.endpoint === subscription.endpoint)
  ) {
    return true
  }
  await subscription.unsubscribe().catch(() => false)
  return false
}

export type TurnOnResult =
  | 'on'
  | 'blocked'
  | 'unsupported'
  | 'refused'
  | 'failed'

/** A browser cut off from its push service can leave a subscription pending for good. */
const SUBSCRIBE_TIMEOUT_MS = 20_000

function orTimeOut<T>(promise: Promise<T>): Promise<T> {
  return Promise.race([
    promise,
    new Promise<never>((_, reject) =>
      setTimeout(() => reject(new Error('timed out')), SUBSCRIBE_TIMEOUT_MS),
    ),
  ])
}

/** Asks for permission, subscribes this browser and tells the server. Call it from a click. */
export async function turnOnPush(publicKey: string): Promise<TurnOnResult> {
  if (!isPushSupported()) return 'unsupported'
  if ((await Notification.requestPermission()) !== 'granted') return 'blocked'
  try {
    await navigator.serviceWorker.register('/sw.js')
    const registration = await navigator.serviceWorker.ready
    // A subscription is tied to the key it was made with
    await (await registration.pushManager.getSubscription())?.unsubscribe()
    const subscription = await orTimeOut(
      registration.pushManager.subscribe({
        userVisibleOnly: true,
        applicationServerKey: toBytes(publicKey),
      }),
    )
    const { endpoint, keys } = subscription.toJSON()
    if (!endpoint || !keys?.p256dh || !keys.auth) {
      await subscription.unsubscribe()
      return 'failed'
    }
    const { response } = await api.POST('/api/push/subscriptions', {
      body: {
        endpoint,
        p256dh: keys.p256dh,
        auth: keys.auth,
        label: browserLabel(),
      },
    })
    if (response.ok) return 'on'
    await subscription.unsubscribe()
    return response.status === 400 ? 'refused' : 'failed'
  } catch {
    return 'failed'
  }
}

/** Stops notifications for a browser of this account; for this one, also at its push service. */
export async function turnOffPush(endpoint?: string): Promise<void> {
  const subscription = await currentSubscription().catch(() => null)
  const target = endpoint ?? subscription?.endpoint
  if (!target) return
  await api
    .POST('/api/push/unsubscribe', { body: { endpoint: target } })
    .catch(() => null)
  if (subscription?.endpoint === target) {
    await subscription.unsubscribe().catch(() => false)
  }
}
