export function parseEntryDate(value: string): Date | null {
  const normalized = value.includes("T") ? value : value.replace(" ", "T");
  const parsed = new Date(normalized);
  return Number.isNaN(parsed.getTime()) ? null : parsed;
}

function startOfDay(date: Date): number {
  return new Date(date.getFullYear(), date.getMonth(), date.getDate()).getTime();
}

const capitalize = (value: string) => value.charAt(0).toUpperCase() + value.slice(1);

/** "Hoje", "Ontem", "Segunda-feira, 6 de outubro" or "6 de outubro de 2024". */
export function dayLabel(date: Date, now = new Date()): string {
  const days = Math.round((startOfDay(now) - startOfDay(date)) / 86_400_000);
  if (days === 0) return "Hoje";
  if (days === 1) return "Ontem";
  const sameYear = date.getFullYear() === now.getFullYear();
  if (days > 1 && days < 7) {
    return capitalize(new Intl.DateTimeFormat("pt-BR", { weekday: "long", day: "numeric", month: "long" }).format(date));
  }
  return new Intl.DateTimeFormat("pt-BR", sameYear ? { day: "numeric", month: "long" } : { day: "numeric", month: "long", year: "numeric" }).format(date);
}

export function dayKey(date: Date): string {
  return `${date.getFullYear()}-${date.getMonth()}-${date.getDate()}`;
}

export function clockTime(date: Date): string {
  return new Intl.DateTimeFormat("pt-BR", { hour: "2-digit", minute: "2-digit" }).format(date);
}

/** Short stamp for compact lists: time today, otherwise day and month. */
export function shortStamp(value: string, now = new Date()): string {
  const date = parseEntryDate(value);
  if (!date) return value;
  if (startOfDay(date) === startOfDay(now)) return clockTime(date);
  return new Intl.DateTimeFormat("pt-BR", { day: "2-digit", month: "2-digit" }).format(date);
}

export function formatDuration(milliseconds: number | null | undefined): string {
  if (!milliseconds) return "";
  const seconds = Math.max(1, Math.round(milliseconds / 1000));
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
}
