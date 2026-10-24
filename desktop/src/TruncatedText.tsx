import { useLayoutEffect, useRef, useState } from "react";
import { Tooltip, TooltipAnchor, useTooltipStore } from "@ariakit/react";

export function TruncatedText({
  children,
  className,
}: {
  children: string;
  className?: string;
}) {
  const elementRef = useRef<HTMLParagraphElement>(null);
  const [clipped, setClipped] = useState(false);
  const tooltip = useTooltipStore({ timeout: 400 });

  useLayoutEffect(() => {
    const element = elementRef.current;
    if (!element) return;
    let active = true;
    const measure = () => {
      if (!active) return;
      const next = element.scrollWidth > element.clientWidth;
      setClipped(next);
      if (!next) tooltip.hide();
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    const fonts = element.ownerDocument.fonts;
    void fonts.ready.then(measure);
    fonts.addEventListener("loadingdone", measure);
    fonts.addEventListener("loadingerror", measure);
    return () => {
      active = false;
      observer.disconnect();
      fonts.removeEventListener("loadingdone", measure);
      fonts.removeEventListener("loadingerror", measure);
    };
  }, [children, tooltip]);

  return (
    <>
      <TooltipAnchor
        store={tooltip}
        ref={elementRef}
        render={<p />}
        className={`truncate ${clipped ? "cursor-help " : ""}${className ?? ""}`}
        focusable={clipped}
        tabIndex={clipped ? 0 : undefined}
        showOnHover={clipped}
        onFocusVisible={(event) => {
          if (!clipped) event.preventDefault();
        }}
      >
        {children}
      </TooltipAnchor>
      {clipped && (
        <Tooltip store={tooltip} className="explanation-tooltip name-tooltip">
          {children}
        </Tooltip>
      )}
    </>
  );
}
