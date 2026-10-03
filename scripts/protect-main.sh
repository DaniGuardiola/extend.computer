#!/bin/bash
# Run after making the repository public or enabling a plan that supports
# private branch protection. Preserves the repository's existing visibility.
set -euo pipefail
repository=${1:-DaniGuardiola/extend.computer}
gh api --method PUT "repos/$repository/branches/main/protection" --input - <<'JSON'
{
  "required_status_checks": {"strict": true, "contexts": ["macOS checks"]},
  "enforce_admins": true,
  "required_pull_request_reviews": {
    "dismiss_stale_reviews": true,
    "require_code_owner_reviews": false,
    "required_approving_review_count": 0
  },
  "restrictions": null,
  "required_linear_history": true,
  "allow_force_pushes": false,
  "allow_deletions": false,
  "required_conversation_resolution": true
}
JSON
