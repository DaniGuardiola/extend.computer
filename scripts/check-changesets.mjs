import { spawnSync } from "node:child_process";
import { writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const output = process.argv[2];
if (!output) throw new Error("Pass a release-plan output path.");
const cli = fileURLToPath(import.meta.resolve("@changesets/cli/bin.js"));
const result = spawnSync(process.execPath, [cli, "status", "--since", "origin/main", "--output", output], {
  encoding: "utf8",
});
process.stdout.write(result.stdout ?? "");
process.stderr.write(result.stderr ?? "");
if (result.error) throw result.error;
if (result.status === 0) process.exit(0);
// Changesets 3 rejects version-only release PRs and documentation-only changes
// without a pending note. Keep that case advisory, preserving all other errors.
const missingNote = "Some packages have been changed but no changesets were found.";
if (result.status === 1 && `${result.stdout}${result.stderr}`.includes(missingNote)) {
  writeFileSync(output, JSON.stringify({ changesets: [], releases: [] }) + "\n");
  console.log("::notice::No pending changeset. Add one for user-visible changes; release PRs and documentation-only changes may omit it.");
} else {
  process.exit(result.status ?? 1);
}
