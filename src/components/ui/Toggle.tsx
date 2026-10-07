export function Toggle({
  checked,
  onChange,
  label,
  disabled = false,
}: {
  checked: boolean;
  onChange: (value: boolean) => void;
  label: string;
  disabled?: boolean;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-label={label}
      aria-checked={checked}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      className={
        "relative inline-flex h-6 w-10 shrink-0 items-center rounded-full transition-colors duration-150 disabled:cursor-not-allowed disabled:opacity-40 " +
        (checked ? "bg-[#242422]" : "bg-[#c9c9c2]")
      }
    >
      <span
        className={
          "h-4 w-4 rounded-full bg-white shadow-[0_1px_3px_rgba(0,0,0,.24)] transition-transform duration-150 " +
          (checked ? "translate-x-5" : "translate-x-1")
        }
      />
    </button>
  );
}
