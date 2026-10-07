import type { ButtonHTMLAttributes, ReactNode } from "react";

type Variant = "primary" | "secondary" | "ghost" | "danger";
type Size = "sm" | "md";

const VARIANTS: Record<Variant, string> = {
  primary: "border border-ink bg-ink text-canvas hover:bg-strong hover:border-strong",
  secondary: "border border-line bg-raised text-ink hover:border-line-strong hover:bg-fill",
  ghost: "border border-transparent bg-transparent text-soft hover:bg-fill hover:text-ink",
  danger: "border border-transparent bg-transparent text-live hover:bg-live-wash",
};

const SIZES: Record<Size, string> = {
  sm: "h-8 rounded-[8px] px-3 text-[12.5px]",
  md: "h-9 rounded-[9px] px-4 text-[13px]",
};

export function Button({
  children,
  variant = "secondary",
  size = "md",
  className = "",
  type = "button",
  ...props
}: {
  children: ReactNode;
  variant?: Variant;
  size?: Size;
} & ButtonHTMLAttributes<HTMLButtonElement>) {
  return (
    <button
      type={type}
      className={
        "inline-flex shrink-0 items-center justify-center gap-2 font-medium transition-[background-color,border-color,color] duration-150 disabled:cursor-not-allowed disabled:opacity-40 " +
        VARIANTS[variant] + " " + SIZES[size] + " " + className
      }
      {...props}
    >
      {children}
    </button>
  );
}
