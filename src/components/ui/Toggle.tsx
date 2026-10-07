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
        "relative inline-flex h-[22px] w-[38px] shrink-0 items-center rounded-full transition-colors duration-150 disabled:cursor-not-allowed disabled:opacity-40 " +
        (checked ? "bg-ink" : "bg-line-strong")
      }
    >
      <span
        className={
          "h-4 w-4 rounded-full bg-raised shadow-[0_1px_2px_rgb(0_0_0/0.25)] transition-transform duration-150 ease-out " +
          (checked ? "translate-x-[19px]" : "translate-x-[3px]")
        }
      />
    </button>
  );
}
