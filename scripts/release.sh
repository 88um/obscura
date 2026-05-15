#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage:
  scripts/release.sh patch [options]
  scripts/release.sh minor [options]
  scripts/release.sh major [options]
  scripts/release.sh 1.2.3 [options]
  scripts/release.sh release [options]

Examples:
  scripts/release.sh patch
  scripts/release.sh minor --skip-tests
  scripts/release.sh 0.3.0 --commit-all
  scripts/release.sh release --no-push

Commands:
  patch          Bump X.Y.Z -> X.Y.(Z+1), commit, tag, and push.
  minor          Bump X.Y.Z -> X.(Y+1).0, commit, tag, and push.
  major          Bump X.Y.Z -> (X+1).0.0, commit, tag, and push.
  1.2.3          Set an explicit version, commit, tag, and push.
  release        Do not bump; tag and push the current Cargo.toml version.

Options:
  --remote NAME  Git remote to push to. Default: origin.
  --branch NAME  Branch to push. Default: current branch.
  --skip-tests   Skip cargo test --workspace.
  --no-push      Create the commit/tag locally, but do not push.
  --commit-all   Include all current working-tree changes in the release commit.
  --dry-run      Print what would happen without changing files.
  -y, --yes      Do not prompt before pushing.
  -h, --help     Show this help.
USAGE
}

die() {
  echo "error: $*" >&2
  exit 1
}

is_dirty() {
  ! git diff --quiet ||
    ! git diff --cached --quiet ||
    [ -n "$(git ls-files --others --exclude-standard)" ]
}

current_workspace_version() {
  awk '
    /^\[workspace\.package\]/ { in_section = 1; next }
    /^\[/ && in_section { exit }
    in_section && $1 == "version" {
      gsub(/"/, "", $3)
      print $3
      exit
    }
  ' Cargo.toml
}

validate_version() {
  case "$1" in
    v*) return 1 ;;
  esac
  [[ "$1" =~ ^[0-9]+\.[0-9]+\.[0-9]+([-+][0-9A-Za-z.-]+)?$ ]]
}

bump_version() {
  local current="$1"
  local bump="$2"

  [[ "$current" =~ ^([0-9]+)\.([0-9]+)\.([0-9]+)$ ]] ||
    die "automatic bumps require a plain X.Y.Z version, got '$current'"

  local major="${BASH_REMATCH[1]}"
  local minor="${BASH_REMATCH[2]}"
  local patch="${BASH_REMATCH[3]}"

  case "$bump" in
    patch) patch=$((patch + 1)) ;;
    minor) minor=$((minor + 1)); patch=0 ;;
    major) major=$((major + 1)); minor=0; patch=0 ;;
    *) die "unknown bump '$bump'" ;;
  esac

  echo "${major}.${minor}.${patch}"
}

set_workspace_version() {
  local next_version="$1"
  NEXT_VERSION="$next_version" perl -0pi -e '
    my $next = $ENV{"NEXT_VERSION"};
    s/(\[workspace\.package\]\s+version\s*=\s*")[^"]+(")/$1$next$2/s
      or die "workspace.package version not found\n";
  ' Cargo.toml
}

command="${1:-}"
[ -n "$command" ] || { usage; exit 1; }
case "$command" in
  -h|--help)
    usage
    exit 0
    ;;
esac
shift || true

remote="origin"
branch=""
run_tests=1
push=1
commit_all=0
dry_run=0
assume_yes=0

while [ "$#" -gt 0 ]; do
  case "$1" in
    --remote)
      [ "$#" -ge 2 ] || die "--remote requires a value"
      remote="$2"
      shift 2
      ;;
    --branch)
      [ "$#" -ge 2 ] || die "--branch requires a value"
      branch="$2"
      shift 2
      ;;
    --skip-tests)
      run_tests=0
      shift
      ;;
    --no-push)
      push=0
      shift
      ;;
    --commit-all)
      commit_all=1
      shift
      ;;
    --dry-run)
      dry_run=1
      shift
      ;;
    -y|--yes)
      assume_yes=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      die "unknown option '$1'"
      ;;
  esac
done

git_root="$(git rev-parse --show-toplevel)"
cd "$git_root"

[ -f Cargo.toml ] || die "Cargo.toml not found at repo root"

current_version="$(current_workspace_version)"
[ -n "$current_version" ] || die "could not read [workspace.package] version from Cargo.toml"

case "$command" in
  patch|minor|major)
    next_version="$(bump_version "$current_version" "$command")"
    bumping=1
    ;;
  release)
    next_version="$current_version"
    bumping=0
    ;;
  v*)
    die "use explicit versions without a leading v, e.g. '1.2.3'"
    ;;
  *)
    validate_version "$command" || die "expected patch, minor, major, release, or X.Y.Z"
    next_version="$command"
    bumping=1
    ;;
esac

tag="v${next_version}"

if [ -z "$branch" ]; then
  branch="$(git branch --show-current)"
fi
[ -n "$branch" ] || die "detached HEAD; pass --branch NAME"

if git rev-parse -q --verify "refs/tags/${tag}" >/dev/null; then
  die "local tag ${tag} already exists"
fi

if git ls-remote --exit-code --tags "$remote" "refs/tags/${tag}" >/dev/null 2>&1; then
  die "remote tag ${tag} already exists on ${remote}"
fi

if [ "$dry_run" -eq 1 ]; then
  cat <<DRYRUN
Release dry run
  repo:          $git_root
  current:       $current_version
  next:          $next_version
  tag:           $tag
  branch:        $branch
  remote:        $remote
  bump version:  $bumping
  run tests:     $run_tests
  push:          $push
  commit all:    $commit_all
DRYRUN
  exit 0
fi

if is_dirty && [ "$commit_all" -eq 0 ]; then
  die "working tree is dirty; commit first or rerun with --commit-all"
fi

if [ "$bumping" -eq 1 ]; then
  echo "Bumping ${current_version} -> ${next_version}"
  set_workspace_version "$next_version"
  cargo generate-lockfile
fi

if [ "$run_tests" -eq 1 ]; then
  cargo test --workspace
fi

if [ "$commit_all" -eq 1 ]; then
  git add -A
else
  git add Cargo.toml Cargo.lock
fi

if ! git diff --cached --quiet; then
  git commit -m "Release ${tag}"
fi

git tag -a "$tag" -m "$tag"

if [ "$push" -eq 0 ]; then
  echo "Created ${tag} locally. Push later with:"
  echo "  git push ${remote} ${branch}"
  echo "  git push ${remote} ${tag}"
  exit 0
fi

if [ "$assume_yes" -eq 0 ]; then
  printf "Push %s and %s to %s? [y/N] " "$branch" "$tag" "$remote"
  read -r answer
  case "$answer" in
    y|Y|yes|YES) ;;
    *) die "push cancelled; commit/tag were created locally" ;;
  esac
fi

git push "$remote" "$branch"
git push "$remote" "$tag"

echo "Pushed ${tag}. The GitHub release workflow should start from the tag push."
