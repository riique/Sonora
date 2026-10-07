import { AlertCircle, Inbox, LoaderCircle } from "lucide-react";
import type { ReactNode } from "react";

export function PageHeader({
  title,
  description,
  action,
}: {
  title: string;
  description: string;
  action?: ReactNode;
}) {
  return (
    <header className="page-header flex max-w-none items-start justify-between gap-6">
      <div className="min-w-0">
        <h1 className="page-title">{title}</h1>
        <p className="page-description">{description}</p>
      </div>
      {action && <div className="shrink-0 pt-0.5">{action}</div>}
    </header>
  );
}

export function PreferenceRow({
  title,
  description,
  children,
}: {
  title: string;
  description: string;
  children: ReactNode;
}) {
  return (
    <div className="flex min-h-[76px] items-center justify-between gap-8 px-1 py-5">
      <div className="min-w-0">
        <h3 className="text-[14px] font-medium text-ink">{title}</h3>
        <p className="mt-1 max-w-[68ch] text-[13px] leading-5 text-muted">{description}</p>
      </div>
      <div className="shrink-0">{children}</div>
    </div>
  );
}

export function EmptyState({ title, description }: { title: string; description: string }) {
  return (
    <div className="flex min-h-52 flex-col items-center justify-center px-8 py-12 text-center">
      <Inbox className="h-6 w-6 text-[#8b8c85]" aria-hidden />
      <h3 className="mt-4 text-[14px] font-medium text-ink">{title}</h3>
      <p className="mt-1 max-w-md text-[13px] leading-5 text-muted">{description}</p>
    </div>
  );
}

export function ErrorState({ children }: { children: ReactNode }) {
  return (
    <div className="flex items-start gap-2.5 rounded-[10px] bg-[#fff1ef] px-4 py-3 text-[13px] leading-5 text-[#9f2720]" role="alert">
      <AlertCircle className="mt-0.5 h-4 w-4 shrink-0" aria-hidden />
      <div>{children}</div>
    </div>
  );
}

export function SkeletonRows({ count = 3 }: { count?: number }) {
  return (
    <div className="divide-y divide-line" aria-label="Carregando" aria-busy="true">
      {Array.from({ length: count }).map((_, index) => (
        <div key={index} className="flex h-20 items-center gap-5 px-4">
          <LoaderCircle className="h-4 w-4 animate-spin text-[#9a9b94]" aria-hidden />
          <div className="flex-1 space-y-2">
            <div className="h-2.5 w-3/5 rounded-sm bg-[#e9e9e4]" />
            <div className="h-2 w-2/5 rounded-sm bg-[#efefeb]" />
          </div>
        </div>
      ))}
    </div>
  );
}
