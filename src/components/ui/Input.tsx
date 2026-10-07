import type { InputHTMLAttributes } from "react";

const base =
  "w-full rounded-[10px] border border-line bg-white px-3.5 text-[13px] text-ink placeholder:text-[#5d5e58] " +
  "transition-[border-color,box-shadow,background-color] duration-150 hover:border-line-strong " +
  "focus:border-[#999a93] focus:outline-none focus:ring-2 focus:ring-[#20201e]/10 disabled:cursor-not-allowed disabled:bg-[#f0f0ec] disabled:text-[#8c8d86]";

export function Input(props: InputHTMLAttributes<HTMLInputElement>) {
  return <input {...props} autoComplete={props.autoComplete ?? "off"} className={base + " h-10 " + (props.className ?? "")} />;
}
