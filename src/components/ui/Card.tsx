import type { HTMLAttributes, ReactNode } from "react";

export function Card({
  children,
  className = "",
  onClick,
  ...props
}: {
  children: ReactNode;
  className?: string;
  onClick?: () => void;
} & Omit<HTMLAttributes<HTMLDivElement>, "onClick">) {
  return (
    <div onClick={onClick} className={"surface " + className} {...props}>
      {children}
    </div>
  );
}
