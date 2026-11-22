import { useEffect, useRef, useState, type ReactNode } from "react";

interface SmoothCollapseProps {
  open: boolean;
  children: ReactNode;
  className?: string;
}

/**
 * Layout-smooth grid-row + compositor opacity/transform drawer transition.
 * Defers first mount until opened once, then pauses interaction via `inert` when closed.
 */
export function SmoothCollapse({ open, children, className = "" }: SmoothCollapseProps) {
  const [hasOpened, setHasOpened] = useState(open);
  const innerRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (open && !hasOpened) {
      setHasOpened(true);
    }
  }, [open, hasOpened]);

  useEffect(() => {
    const el = innerRef.current;
    if (!el) return;
    if (open) {
      el.removeAttribute("inert");
    } else {
      el.setAttribute("inert", "");
    }
  }, [open, hasOpened]);

  return (
    <div
      className={`grid transition-[grid-template-rows,opacity] duration-250 ease-[cubic-bezier(0.22,1,0.36,1)] ${
        open ? "grid-rows-[1fr] opacity-100" : "pointer-events-none grid-rows-[0fr] opacity-0"
      } ${className}`}
      aria-hidden={!open}
    >
      <div
        ref={innerRef}
        className={`overflow-hidden transition-transform duration-250 ease-[cubic-bezier(0.22,1,0.36,1)] ${
          open ? "translate-y-0 visible" : "-translate-y-1 invisible"
        }`}
      >
        {hasOpened ? children : null}
      </div>
    </div>
  );
}
