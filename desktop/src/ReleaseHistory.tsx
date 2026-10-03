import Markdown from "react-markdown";
import changelog from "../CHANGELOG.md?raw";
import { Dialog } from "./Dialog";

export default function ReleaseHistory({
  version,
  onClose,
}: {
  version?: string;
  onClose: () => void;
}) {
  return (
    <Dialog title="What’s new" onClose={onClose}>
      {version && (
        <p className="mb-4 text-xs text-muted">Installed version: {version}</p>
      )}
      <div className="release-history max-h-[60vh] overflow-y-auto text-sm">
        <Markdown
          components={{
            img: () => null,
            a: ({ children }) => <span>{children}</span>,
          }}
        >
          {changelog}
        </Markdown>
      </div>
    </Dialog>
  );
}
