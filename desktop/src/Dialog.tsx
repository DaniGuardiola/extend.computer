import type { ReactNode } from "react";
import * as Ariakit from "@ariakit/react";
import { X } from "lucide-react";
export function Dialog({
  title,
  children,
  onClose,
}: {
  title: string;
  children: ReactNode;
  onClose: () => void;
}) {
  return (
    <Ariakit.Dialog
      open
      onClose={onClose}
      className="dialog"
      hideOnInteractOutside={
        import.meta.env.MODE === "ui-review"
          ? (event) => (event.target as Node | null)?.ownerDocument === document
          : undefined
      }
      backdrop={
        <div className="fixed inset-0 bg-black/20 backdrop-blur-[3px]" />
      }
    >
      <div className="mb-5 flex items-center justify-between gap-4">
        <Ariakit.DialogHeading className="min-w-0 break-words text-lg font-medium tracking-[-.3px]">
          {title}
        </Ariakit.DialogHeading>
        <button
          className="icon-button -me-2 shrink-0"
          aria-label="Close"
          onClick={onClose}
        >
          <X size={16} />
        </button>
      </div>
      {children}
    </Ariakit.Dialog>
  );
}
