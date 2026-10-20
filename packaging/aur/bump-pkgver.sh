#!/usr/bin/env bash
#
# Point a packaging/aur PKGBUILD at a released version.
#
#   bump-pkgver.sh <aur-pkgname> <version> <path/to/PKGBUILD>
#
# Sets pkgver to <version>. pkgrel is reset to 1 for a new version, and bumped
# when the AUR already carries the same version (e.g. re-running the workflow
# after a packaging fix), so users always see an upgrade.
#
# Requires: curl, jq, sed, sort.

set -o errexit -o pipefail -o nounset

pkgname=${1:?usage: bump-pkgver.sh <aur-pkgname> <version> <PKGBUILD>}
version=${2:?usage: bump-pkgver.sh <aur-pkgname> <version> <PKGBUILD>}
pkgbuild=${3:?usage: bump-pkgver.sh <aur-pkgname> <version> <PKGBUILD>}

[[ $version =~ ^[0-9][0-9A-Za-z._+]*$ ]] || {
    echo "error: '$version' does not look like a pkgver" >&2
    exit 1
}

[[ -f $pkgbuild ]] || {
    echo "error: $pkgbuild does not exist" >&2
    exit 1
}

# Current AUR version ("5.7.10-1"), empty when the package does not exist yet.
# A missing package is a normal result; an unreachable AUR aborts the run.
current=$(curl -fsS "https://aur.archlinux.org/rpc/v5/info?arg[]=$pkgname" |
    jq -r '.results[0].Version // empty')

current_pkgver=""
current_pkgrel=""
if [[ -n $current ]]; then
    current_pkgver=${current%-*}
    current_pkgrel=${current##*-}
fi

if [[ -n $current_pkgver && $current_pkgver != "$version" ]]; then
    newest=$(printf '%s\n%s\n' "$current_pkgver" "$version" | sort -V | tail -n 1)
    if [[ $newest != "$version" ]]; then
        echo "error: AUR already carries $current, refusing to push $version" >&2
        exit 1
    fi
fi

if [[ $current_pkgver == "$version" ]]; then
    pkgrel=$((current_pkgrel + 1))
    reason="AUR is already at $current"
else
    pkgrel=1
    reason="AUR has ${current:-<no package>}"
fi

sed -i "s/^pkgver=.*/pkgver=$version/; s/^pkgrel=.*/pkgrel=$pkgrel/" "$pkgbuild"

summary="$pkgname -> $version-$pkgrel ($reason)"
echo "$summary"
if [[ -n ${GITHUB_STEP_SUMMARY:-} ]]; then
    echo "- $summary" >>"$GITHUB_STEP_SUMMARY"
fi
