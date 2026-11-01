import { useEffect } from 'react'
import type { ReactNode } from 'react'
import { createPortal } from 'react-dom'
import { useI18n } from '../i18n/I18nProvider'

interface ModalProps {
  title: string
  sub?: string
  onClose: () => void
  children: ReactNode
  footer?: ReactNode
  wide?: boolean
}

export function Modal({ title, sub, onClose, children, footer, wide = false }: ModalProps) {
  const { tr } = useI18n()
  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      if (event.key === 'Escape') onClose()
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [onClose])

  return createPortal(
    <div
      className="modal-mask"
      role="presentation"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) onClose()
      }}
    >
      <div className={`modal ${wide ? 'wide' : ''}`} role="dialog" aria-modal="true" aria-label={title}>
        <div className="modal-head">
          <div>
            <h3>{title}</h3>
            {sub && <div className="sub">{sub}</div>}
          </div>
          <button className="btn sm ghost" type="button" style={{ marginLeft: 'auto' }} onClick={onClose} aria-label={tr('关闭', 'Close')}>
            ×
          </button>
        </div>
        <div className="modal-body">{children}</div>
        {footer && <div className="modal-foot">{footer}</div>}
      </div>
    </div>,
    document.body,
  )
}

export function ConfirmDialog({
  title,
  desc,
  confirmText,
  danger = false,
  busy = false,
  onCancel,
  onConfirm,
}: {
  title: string
  desc: ReactNode
  confirmText?: string
  danger?: boolean
  busy?: boolean
  onCancel: () => void
  onConfirm: () => void
}) {
  const { tr } = useI18n()
  return (
    <Modal
      title={title}
      onClose={onCancel}
      footer={
        <>
          <button className="btn" type="button" onClick={onCancel} disabled={busy}>
            {tr('取消', 'Cancel')}
          </button>
          <button className={`btn ${danger ? 'danger' : 'primary'}`} type="button" onClick={onConfirm} disabled={busy}>
            {busy ? tr('处理中…', 'Processing…') : (confirmText ?? tr('确认', 'Confirm'))}
          </button>
        </>
      }
    >
      <div>{desc}</div>
    </Modal>
  )
}
