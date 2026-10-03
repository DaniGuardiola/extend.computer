#!/usr/bin/env python3
"""Write an advisory Changesets release plan to the GitHub Actions summary."""
import json
import os
from pathlib import Path
import subprocess
import sys

plan = json.loads(Path(sys.argv[1]).read_text())
lines = ['## Changesets', '']
base = os.environ.get('PR_BASE_SHA')
if base:
    paths = subprocess.check_output(['git', 'diff', '--name-only', '--diff-filter=AM', base, 'HEAD', '--', '.changeset'], text=True).splitlines()
    changesets = [path for path in paths if path.endswith('.md') and Path(path).name != 'README.md']
    lines.append(f'This pull request adds or updates {len(changesets)} changeset(s).')
    if not changesets:
        lines.append('Add a changeset for user-visible changes. Documentation and test-only changes may omit one.')
    lines.append('')
releases = plan.get('releases', [])
if releases:
    lines.extend(['Pending release plan:', ''])
    for release in releases:
        name = str(release['name']).replace('`', '')
        version = str(release['newVersion']).replace('`', '')
        lines.append(f'- `{name}` → `{version}` ({release["type"]})')
else:
    lines.append('No pending releases.')
output = '\n'.join(lines) + '\n'
summary = os.environ.get('GITHUB_STEP_SUMMARY')
if summary:
    with open(summary, 'a') as file:
        file.write(output)
print(output)
