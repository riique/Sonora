import { AlertCircle } from "lucide-react";
import type { ReactNode } from "react";

export function PageHeader({
  title,
  description,
  action,
}: {
  title: ReactNode;
  description?: ReactNode;
  action?: ReactNode;
}) {
  return (
    <header className="mb-10 flex items-end justify-between gap-6">
      <div className="min-w-0">
        <h1 className="page-title">{title}</h1>
        {description && <p className="page-description">{description}</p>}
      </div>
      {action && <div className="flex shrink-0 items-center gap-2">{action}</div>}
    </header>
  );
}

/** A titled group of settings or content; separation comes from space, not boxes. */
export function Section({
  title,
  description,
  action,
  children,
  className = "",
}: {
  title: string;
  description?: ReactNode;
  action?: ReactNode;
  children: ReactNode;
  className?: string;
}) {
  return (
    <section className={"mt-12 first:mt-0 " + className}>
      <div className="mb-3 flex items-end justify-between gap-6">
        <div className="min-w-0">
          <h2 className="section-title">{title}</h2>
          {description && <p className="section-description">{description}</p>}
        </div>
        {action && <div className="flex shrink-0 items-center gap-2">{action}</div>}
      </div>
      {children}
    </section>
  );
}

export function PreferenceRow({
  title,
  description,
  children,
  htmlFor,
}: {
  title: string;
  description?: ReactNode;
  children: ReactNode;
  htmlFor?: string;
}) {
  return (
    <div className="flex min-h-16 items-center justify-between gap-10 py-4">
      <div className="min-w-0">
        {htmlFor ? (
          <label htmlFor={htmlFor} className="block text-[13px] font-medium text-ink">{title}</label>
        ) : (
          <h3 className="text-[13px] font-medium text-ink">{title}</h3>
        )}
        {description && <p className="mt-0.5 max-w-[60ch] text-[12.5px] leading-5 text-muted">{description}</p>}
      </div>
      <div className="flex shrink-0 items-center gap-3">{children}</div>
    </div>
  );
}

/** Rows of preferences divided by hairlines. */
export function RowGroup({ children }: { children: ReactNode }) {
  return <div className="hairline-list border-y border-line">{children}</div>;
}

export function EmptyState({ title, description, action }: { title: string; description: ReactNode; action?: ReactNode }) {
  return (
    <div className="flex min-h-48 flex-col items-start justify-center py-12">
      <h3 className="text-[14px] font-medium text-ink">{title}</h3>
      <p className="mt-1 max-w-[52ch] text-[13px] leading-5 text-muted">{description}</p>
      {action && <div className="mt-5">{action}</div>}
    </div>
  );
}

export function ErrorState({ children }: { children: ReactNode }) {
  return (
    <div className="mb-6 flex items-start gap-2.5 rounded-[9px] bg-live-wash px-4 py-3 text-[13px] leading-5 text-live" role="alert">
      <AlertCircle className="mt-0.5 h-4 w-4 shrink-0" aria-hidden />
      <div className="min-w-0 wrap-break-word">{children}</div>
    </div>
  );
}

export function SkeletonRows({ count = 3 }: { count?: number }) {
  return (
    <div className="hairline-list" aria-label="Carregando" aria-busy="true">
      {Array.from({ length: count }).map((_, index) => (
        <div key={index} className="flex h-14 items-center gap-6">
          <div className="h-2.5 w-10 rounded-sm bg-fill" />
          <div className="h-2.5 flex-1 rounded-sm bg-fill" style={{ maxWidth: `${70 - index * 9}%` }} />
        </div>
      ))}
    </div>
  );
}

/** Segmented control for a small set of mutually exclusive views or filters. */
export function Segmented<T extends string>({
  value,
  options,
  onChange,
  label,
}: {
  value: T;
  options: { value: T; label: string }[];
  onChange: (value: T) => void;
  label: string;
}) {
  return (
    <div className="inline-flex rounded-[9px] bg-fill p-[3px]" role="group" aria-label={label}>
      {options.map((option) => (
        <button
          key={option.value}
          type="button"
          aria-pressed={value === option.value}
          onClick={() => onChange(option.value)}
          className={
            "h-7 rounded-[7px] px-3 text-[12px] font-medium transition-colors duration-150 " +
            (value === option.value ? "bg-raised text-ink shadow-[0_1px_2px_rgb(0_0_0/0.08)]" : "text-muted hover:text-ink")
          }
        >
          {option.label}
        </button>
      ))}
    </div>
  );
}
