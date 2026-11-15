#!/usr/bin/env bash
# Publishes muretto-bin VERSION to the AUR: copies the PKGBUILD template into
# a clone of the AUR repository under target/aur, sets the version, fills in
# checksums and .SRCINFO, builds the package once to check it, then commits
# and pushes after asking.
#
#   packaging/aur/publish.sh VERSION [GIT-REF]
#
# GIT-REF is where the icon and licence are taken from (default: vVERSION).
# PKGREL=2 republishes the same version with packaging changes.
# Needs: the GitHub release published, pacman-contrib (updpkgsums), and an
# AUR account with your SSH key.
set -euo pipefail

version=${1:?usage: publish.sh VERSION [GIT-REF]}
ref=${2:-v$version}
pkgrel=${PKGREL:-1}

here=$(cd "$(dirname "$0")" && pwd)
root=$(git -C "$here" rev-parse --show-toplevel)
work="$root/target/aur/muretto-bin"

if [ -d "$work/.git" ]; then
  # A new AUR package starts as an empty repository with nothing to pull.
  if git -C "$work" rev-parse -q --verify HEAD >/dev/null; then
    git -C "$work" pull --ff-only
  fi
else
  mkdir -p "$(dirname "$work")"
  git clone ssh://aur@aur.archlinux.org/muretto-bin.git "$work"
fi

cp "$here/muretto-bin/PKGBUILD" "$work/PKGBUILD"
cd "$work"
sed -i \
  -e "s/^pkgver=.*/pkgver=$version/" \
  -e "s/^pkgrel=.*/pkgrel=$pkgrel/" \
  -e "s|^_ref=.*|_ref=$ref|" \
  PKGBUILD
updpkgsums
makepkg --printsrcinfo > .SRCINFO
makepkg --force --cleanbuild --nodeps

git add PKGBUILD .SRCINFO
git --no-pager diff --cached --stat
if git diff --cached --quiet; then
  echo "Nothing changed; the AUR already has this."
  exit 0
fi
read -r -p "Push muretto-bin $version-$pkgrel to the AUR? [y/N] " answer
[ "$answer" = y ] || { echo "Not pushed. The commit is staged in $work."; exit 1; }
git commit -m "Update to $version-$pkgrel"
git push origin HEAD:master
