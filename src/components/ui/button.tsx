import * as React from "react";
import { cn } from "./cn";

type Variant = "default" | "ghost" | "outline" | "secondary" | "destructive";
type Size = "sm" | "md" | "lg" | "icon";

const variants: Record<Variant, string> = {
  default: "bg-black text-white hover:bg-black/90 border border-transparent dark:bg-white dark:text-black dark:hover:bg-black dark:hover:text-white dark:hover:border-white",
  ghost: "hover:bg-black/5 dark:hover:bg-white/5 text-muted-foreground hover:text-foreground",
  outline: "border border-input bg-transparent hover:bg-black/5 dark:hover:bg-white/5",
  secondary: "bg-black/5 dark:bg-white/10 text-foreground hover:bg-black/10",
  destructive: "bg-red-600 text-white hover:bg-red-700",
};

const sizes: Record<Size, string> = {
  sm: "h-8 px-3 text-xs",
  md: "h-9 px-4 text-sm",
  lg: "h-10 px-6 text-base",
  icon: "h-9 w-9",
};

export interface ButtonProps
  extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: Variant;
  size?: Size;
}

export const Button = React.forwardRef<HTMLButtonElement, ButtonProps>(
  ({ className, variant = "default", size = "md", ...props }, ref) => (
    <button
      ref={ref}
      className={cn(
        "inline-flex items-center justify-center gap-1.5 rounded-md font-medium transition-colors disabled:opacity-50 disabled:pointer-events-none",
        variants[variant],
        sizes[size],
        className,
      )}
      {...props}
    />
  ),
);
Button.displayName = "Button";
