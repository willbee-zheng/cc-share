import * as React from "react";
import { cn } from "./cn";

interface CollapsibleProps extends React.HTMLAttributes<HTMLDivElement> {
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
  defaultOpen?: boolean;
}

export const Collapsible: React.FC<CollapsibleProps> = ({
  className,
  children,
  open: controlled,
  onOpenChange,
  defaultOpen = false,
}) => {
  const [uncontrolled, setUncontrolled] = React.useState(defaultOpen);
  const open = controlled ?? uncontrolled;
  return (
    <CollapsibleContext.Provider value={{ open, setOpen: (v) => { setUncontrolled(v); onOpenChange?.(v); } }}>
      <div className={cn(className)}>{children}</div>
    </CollapsibleContext.Provider>
  );
};

const CollapsibleContext = React.createContext<{ open: boolean; setOpen: (v: boolean) => void }>({
  open: false,
  setOpen: () => {},
});

export const CollapsibleTrigger: React.FC<
  React.HTMLAttributes<HTMLButtonElement> & { asChild?: boolean }
> = ({ asChild, children, onClick, ...props }) => {
  const { open, setOpen } = React.useContext(CollapsibleContext);
  const handleClick = (e: React.MouseEvent<HTMLButtonElement>) => {
    onClick?.(e);
    setOpen(!open);
  };
  if (asChild && React.isValidElement(children)) {
    const child = children as React.ReactElement<{ onClick?: (e: React.MouseEvent<HTMLButtonElement>) => void }>;
    return React.cloneElement(child, { ...props, onClick: handleClick });
  }
  return (
    <button {...props} onClick={handleClick}>
      {children}
    </button>
  );
};

export const CollapsibleContent: React.FC<
  React.HTMLAttributes<HTMLDivElement> & { open?: boolean }
> = ({ className, children, open: propOpen, ...props }) => {
  const ctx = React.useContext(CollapsibleContext);
  const open = propOpen ?? ctx.open;
  return (
    <div className={cn("overflow-hidden transition-all", open ? "max-h-[4000px]" : "max-h-0", className)} {...props}>
      {children}
    </div>
  );
};
