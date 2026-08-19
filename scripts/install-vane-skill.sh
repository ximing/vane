#!/usr/bin/env sh
# Install the Vane agent skill into common coding-agent skill directories.
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/ximing/vane/main/scripts/install-vane-skill.sh | sh
# Or from a clone:
#   sh scripts/install-vane-skill.sh
set -eu

REPO="${VANE_REPO:-ximing/vane}"
REF="${VANE_REF:-main}"
URL="https://raw.githubusercontent.com/${REPO}/${REF}/skills/vane/SKILL.md"

if [ -f "$(dirname "$0")/../skills/vane/SKILL.md" ]; then
  SRC="$(cd "$(dirname "$0")/.." && pwd)/skills/vane/SKILL.md"
else
  SRC=""
fi

installed=0
for d in \
  "${HOME}/.claude/skills/vane" \
  "${HOME}/.agents/skills/vane" \
  "${HOME}/.cursor/skills/vane" \
  "${HOME}/.grok/skills/vane"
do
  mkdir -p "$d"
  if [ -n "$SRC" ]; then
    cp "$SRC" "$d/SKILL.md"
  else
    if command -v curl >/dev/null 2>&1; then
      curl -fsSL "$URL" -o "$d/SKILL.md"
    elif command -v wget >/dev/null 2>&1; then
      wget -q -O "$d/SKILL.md" "$URL"
    else
      echo "need curl or wget" >&2
      exit 1
    fi
  fi
  echo "installed ${d}/SKILL.md"
  installed=1
done

if [ "$installed" -eq 0 ]; then
  echo "no skill directories written" >&2
  exit 1
fi
