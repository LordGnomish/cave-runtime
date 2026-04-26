#!/bin/bash
set -e
cd "$(dirname "$0")"

echo "=== CAVE Git Cleanup Script ==="

# Step 1: Remove stale lock
echo "1. Removing index.lock..."
rm -f .git/index.lock

# Step 2: Commit config.rs fix
echo "2. Committing config.rs fix..."
git add crates/cave-core/src/config.rs
git commit -m "chore: remove employer reference from config example" 2>/dev/null || echo "   (already committed or no changes)"

# Step 3: Remove old filter-branch backups
echo "3. Removing old refs/original..."
git for-each-ref --format='%(refname)' refs/original/ 2>/dev/null | xargs -r git update-ref -d

# Step 4: Rewrite ALL history
echo "4. Rewriting git history (all branches)..."
FILTER_BRANCH_SQUELCH_WARNING=1 git filter-branch -f --env-filter '
  export GIT_AUTHOR_NAME="CAVE Contributors"
  export GIT_AUTHOR_EMAIL="contributors@caveplatform.dev"
  export GIT_COMMITTER_NAME="CAVE Contributors"
  export GIT_COMMITTER_EMAIL="contributors@caveplatform.dev"
' -- --all

# Step 5: Verify
echo "5. Verifying..."
echo "   Authors found:"
git log --all --format='%aN <%aE>' | sort | uniq
echo "   Committers found:"
git log --all --format='%cN <%cE>' | sort | uniq

# Step 6: Check for remaining references in code
echo "6. Checking code for remaining references..."
FOUND=$(grep -ri "cave-corp" --include="*.rs" --include="*.toml" --include="*.yaml" --include="*.md" --include="*.yml" . 2>/dev/null | grep -v ".git/" | grep -v ".claude/worktrees/" | grep -v "cleanup-git.sh" || true)
if [ -z "$FOUND" ]; then
  echo "   CLEAN - no references found"
else
  echo "   WARNING - found references:"
  echo "$FOUND"
fi

# Step 7: Force push
echo "7. Force pushing to GitHub..."
git push origin main --force

echo ""
echo "=== DONE ==="
echo "Run 'rm cleanup-git.sh' to remove this script"
