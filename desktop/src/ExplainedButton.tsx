import { Tooltip, TooltipAnchor, TooltipProvider } from "@ariakit/react";
import type { ComponentProps } from "react";

type Props = ComponentProps<"button"> & {
  explanation: string;
};

export function ExplainedButton({ explanation, children, ...props }: Props) {
  return (
    <TooltipProvider timeout={400}>
      <TooltipAnchor
        render={
          <button type="button" aria-description={explanation} {...props} />
        }
      >
        {children}
      </TooltipAnchor>
      <Tooltip className="explanation-tooltip">{explanation}</Tooltip>
    </TooltipProvider>
  );
}
