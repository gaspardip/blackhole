#!/bin/sh
# Smoke-test an absolute archive path against the expected Node platform and CLI version.
set -eu
archive=${1:?archive}
platform=${2:?platform}
version=${3:?version}
actual=$(node -p 'process.platform + "-" + process.arch')
if [ "$actual" != "$platform" ]; then
  echo "Expected $platform, running on $actual" >&2
  exit 1
fi
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
cd "$work"
echo '{"private":true}' > package.json
pnpm add --offline --ignore-scripts "$archive"
cli="$PWD/node_modules/.bin/blackhole"
git init -q repo
cd repo
actual=$("$cli" --version)
if [ "$actual" != "blackhole $version" ]; then
  echo "Expected blackhole $version, got $actual" >&2
  exit 1
fi
echo 'export const value = 1' > example.ts
git add example.ts
"$cli" check
echo '// remove this comment' > example.ts
status=0
"$cli" check > findings || status=$?
cat findings
test "$status" = 1
grep -F 'example.ts:1:1 ts/no-comments' findings
