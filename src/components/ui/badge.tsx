import * as React from "react";
import { cn } from "./cn";

type Variant = "default" | "secondary" | "success" | "warning" | "destructive" | "outline";

const variants: Record<Variant, string> = {
  default: "bg-black/10 dark:bg-white/15 text-foreground",
  secondary: "bg-black/5 dark:bg-white/10 text-muted-foreground",
  success: "bg-green-100 text-green-800 dark:bg-green-900/40 dark:text-green-300",
  warning: "bg-amber-100 text-amber-800 dark:bg-amber-900/40 dark:text-amber-300",
  destructive: "bg-red-100 text-red-800 dark:bg-red-900/40 dark:text-red-300",
  outline: "border border-input text-foreground",
};

export function Badge({
  className,
  variant = "default",
  ...props
}: React.HTMLAttributes<HTMLSpanElement> & { variant?: Variant }) {
  return (
    <span
      className={cn(
        "inline-flex items-center rounded px-1.5 py-0.5 text-xs font-medium",
        variants[variant],
        className,
      )}
      {...props}
    />
  );
}
