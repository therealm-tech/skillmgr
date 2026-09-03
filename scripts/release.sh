#!/usr/bin/env bash
#
# Cut a skillmgr release: bump the version, commit, tag, and offer to push.
# CI publishes from the tag and refuses when Cargo.toml disagrees with it, so
# this script is the only supported way to release.

set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

DEFAULT_BRANCH="main"

usage() {
  cat <<'EOF'
Usage: scripts/release.sh [--dry-run] <version>

Bump the version in Cargo.toml and Cargo.lock, commit, tag `v<version>`, and
ask before pushing. Pushing the tag is what publishes the release.

Arguments:
  <version>    the new version, in semver form (e.g. 0.2.0)

Options:
  --dry-run    report what would change, then stop
  -h, --help   show this help

Examples:
  scripts/release.sh --dry-run 0.2.0
  scripts/release.sh 0.2.0
EOF
}

die() {
  echo "error: $*" >&2
  exit 1
}

dry_run=false
version=""

while [ $# -gt 0 ]; do
  case "$1" in
    --dry-run) dry_run=true ;;
    -h | --help)
      usage
      exit 0
      ;;
    -*) die "unknown option $1 (see --help)" ;;
    *)
      [ -z "$version" ] || die "only one version may be given"
      version="$1"
      ;;
  esac
  shift
done

[ -n "$version" ] || {
  usage >&2
  die "a version is required"
}

echo "$version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$' ||
  die "\`$version\` is not a semver version"

command -v cargo >/dev/null || die "cargo is not on PATH"
cargo set-version --help >/dev/null 2>&1 ||
  die "cargo-set-version is missing; install it with \`cargo install cargo-edit\`"

current="$(python3 -c 'import tomllib; print(tomllib.load(open("Cargo.toml", "rb"))["package"]["version"])')"
tag="v${version}"
branch="$(git rev-parse --abbrev-ref HEAD)"

[ "$branch" = "$DEFAULT_BRANCH" ] ||
  die "releases are cut from ${DEFAULT_BRANCH}, not ${branch}"
[ -z "$(git status --porcelain)" ] ||
  die "the working tree is dirty; commit or stash first"
git rev-parse --verify --quiet "refs/tags/${tag}" >/dev/null &&
  die "the tag ${tag} already exists"

git fetch --quiet origin "$branch"
behind="$(git rev-list --count "HEAD..origin/${branch}")"
[ "$behind" -eq 0 ] ||
  die "${branch} is ${behind} commit(s) behind origin; pull first"

if [ "$dry_run" = true ]; then
  cat <<EOF
version: ${current} -> ${version}
commit:  release ${version}
tag:     ${tag}
nothing was changed (dry run)
EOF
  exit 0
fi

cargo set-version "$version"
cargo check --quiet --locked >/dev/null

git add Cargo.toml Cargo.lock
git commit --quiet --message "release ${version}"
git tag --annotate "$tag" --message "skillmgr ${version}"

echo "committed and tagged ${tag} on ${branch}."

if [ ! -t 0 ]; then
  echo "not a terminal, so nothing was pushed. To publish:" >&2
  echo "  git push origin ${branch} ${tag}" >&2
  exit 0
fi

printf 'push %s and %s to origin? this publishes the release. [y/N] ' "$branch" "$tag"
read -r reply
case "$reply" in
  [yY] | [yY][eE][sS])
    git push origin "$branch" "$tag"
    echo "pushed. CI publishes the release from ${tag}."
    ;;
  *)
    echo "nothing was pushed. To publish:"
    echo "  git push origin ${branch} ${tag}"
    ;;
esac
