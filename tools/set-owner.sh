#!/usr/bin/env bash
set -euo pipefail
owner="${1:?usage: tools/set-owner.sh GITHUB_USER [REPOSITORY]}"
repo="${2:-camd}"
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"
git ls-files -z -- ':!*.png' ':!*.jpg' ':!*.woff2' ':!tools/set-owner.sh' \
  | xargs -0 grep -lE 'OWNER|YOUR-USER' \
  | while read -r file; do
      sed -i -e "s#OWNER\.github\.io/camd#${owner}.github.io/${repo}#g" -e "s#OWNER/camd#${owner}/${repo}#g" -e "s#YOUR-USER/camd#${owner}/${repo}#g" -e "s#YOUR-USER\.github\.io/camd#${owner}.github.io/${repo}#g" "$file"
    done
