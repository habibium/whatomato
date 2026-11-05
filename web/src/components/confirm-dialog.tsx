import { useState } from 'react'
import { AlertDialog } from '@astryxdesign/core/AlertDialog'

interface ConfirmDeleteDialogProps {
  isOpen: boolean
  onOpenChange: (isOpen: boolean) => void
  title: string
  description: string
  /** The dialog stays open until this settles. */
  onConfirm: () => Promise<void>
}

export function ConfirmDeleteDialog({
  isOpen,
  onOpenChange,
  title,
  description,
  onConfirm,
}: ConfirmDeleteDialogProps) {
  const [isDeleting, setIsDeleting] = useState(false)

  return (
    <AlertDialog
      isOpen={isOpen}
      onOpenChange={onOpenChange}
      title={title}
      description={description}
      actionLabel="Delete"
      isActionLoading={isDeleting}
      onAction={async () => {
        setIsDeleting(true)
        await onConfirm()
        setIsDeleting(false)
      }}
    />
  )
}
