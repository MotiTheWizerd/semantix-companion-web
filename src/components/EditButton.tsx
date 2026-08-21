interface EditButtonProps {
  label: string;
  onClick: () => void;
}

export function EditButton({ label, onClick }: EditButtonProps) {
  return (
    <button
      className="credential-list__edit"
      type="button"
      aria-label={`Edit ${label}`}
      onClick={onClick}
    >
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <path d="m4 20 4.2-1 10.4-10.4a2.1 2.1 0 0 0-3-3L5.2 16 4 20Z" />
        <path d="m13.8 7.4 2.8 2.8" />
      </svg>
    </button>
  );
}
