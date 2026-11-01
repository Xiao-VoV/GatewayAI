import { useEffect } from 'react'
import type { ReactNode } from 'react'
import { createPortal } from 'react-dom'

interface ModalProps {
  title: string
  sub?: string
  onClose: () => void
  children: ReactNode
  footer?: ReactNode
  wide?: boolean
}

export function Modal({ title, sub, onClose, children, footer, wide = false }: ModalProps) {
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
          <button className="btn sm ghost" type="button" style={{ marginLeft: 'auto' }} onClick={onClose} aria-label="关闭">
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
  confirmText = '确认',
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
  return (
    <Modal
      title={title}
      onClose={onCancel}
      footer={
        <>
          <button className="btn" type="button" onClick={onCancel} disabled={busy}>
            取消
          </button>
          <button className={`btn ${danger ? 'danger' : 'primary'}`} type="button" onClick={onConfirm} disabled={busy}>
            {busy ? '处理中…' : confirmText}
          </button>
        </>
      }
    >
      <div>{desc}</div>
    </Modal>
  )
}
