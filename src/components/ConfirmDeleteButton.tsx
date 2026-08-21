interface ConfirmDeleteButtonProps {
  label: string;
  isConfirming: boolean;
  isDeleting: boolean;
  onRequestConfirmation: () => void;
  onCancel: () => void;
  onConfirm: () => void;
}

export function ConfirmDeleteButton({
  label,
  isConfirming,
  isDeleting,
  onRequestConfirmation,
  onCancel,
  onConfirm,
}: ConfirmDeleteButtonProps) {
  if (isConfirming) {
    return (
      <div
        className="credential-list__delete-confirmation"
        role="group"
        aria-label={`Confirm removal of ${label}`}
      >
        <button
          className="credential-list__delete-cancel"
          type="button"
          disabled={isDeleting}
          onClick={onCancel}
        >
          Cancel
        </button>
        <button
          className="credential-list__delete-confirm"
          type="button"
          disabled={isDeleting}
          onClick={onConfirm}
        >
          {isDeleting ? "Deleting…" : "Delete"}
        </button>
      </div>
    );
  }

  return (
    <button
      className="credential-list__delete"
      type="button"
      aria-label={`Remove ${label}`}
      onClick={onRequestConfirmation}
    >
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <path d="M4 7h16M9 7V4h6v3M7 7l1 13h8l1-13M10 11v5M14 11v5" />
      </svg>
    </button>
  );
}
