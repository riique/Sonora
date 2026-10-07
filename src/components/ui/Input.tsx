import type { InputHTMLAttributes, SelectHTMLAttributes, TextareaHTMLAttributes } from "react";
import { ChevronDown } from "lucide-react";

export const fieldBase =
  "w-full rounded-[9px] border border-line bg-raised text-[13px] text-ink placeholder:text-muted " +
  "transition-[border-color,box-shadow] duration-150 hover:border-line-strong " +
  "focus:border-faint focus:outline-hidden focus:ring-[3px] focus:ring-ink/8 disabled:cursor-not-allowed disabled:bg-fill disabled:text-muted";

export function Input(props: InputHTMLAttributes<HTMLInputElement>) {
  return <input {...props} autoComplete={props.autoComplete ?? "off"} className={fieldBase + " h-9 px-3 " + (props.className ?? "")} />;
}

export function Textarea(props: TextareaHTMLAttributes<HTMLTextAreaElement>) {
  return <textarea {...props} className={fieldBase + " px-3 py-2.5 leading-[1.6] " + (props.className ?? "")} />;
}

/** Native select with a themed chevron; keeps platform keyboard behaviour. */
export function Select({ className = "", wrapperClassName = "", children, ...props }: SelectHTMLAttributes<HTMLSelectElement> & { wrapperClassName?: string }) {
  return (
    <span className={"relative inline-flex min-w-0 " + wrapperClassName}>
      <select {...props} className={fieldBase + " h-9 appearance-none pl-3 pr-8 " + className}>
        {children}
      </select>
      <ChevronDown className="pointer-events-none absolute right-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted" aria-hidden />
    </span>
  );
}
