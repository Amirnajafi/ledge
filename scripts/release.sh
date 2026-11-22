#!/bin/sh
# Bump the patch version, build Ledge for macOS and Windows, publish both to a GitHub release,
# and restart this Mac's Ledge on the new build.
# Every machine running Ledge checks GitHub at launch and twice a day, and installs the new release by itself.
# Needs: a clean git tree, `gh` logged in, and cargo-xwin + Homebrew LLVM for the Windows build.
set -eu
cd "$(dirname "$0")/.."

[ -z "$(git status --porcelain)" ] || { echo "release: commit or stash your changes first"; exit 1; }
gh auth status >/dev/null 2>&1 || { echo "release: run 'gh auth login' first"; exit 1; }

cargo_toml=src-tauri/Cargo.toml
old=$(sed -n 's/^version = "\(.*\)"/\1/p' "$cargo_toml" | head -1)
new=$(echo "$old" | awk -F. '{ printf "%d.%d.%d", $1, $2, $3 + 1 }')
sed -i '' "s/^version = \"$old\"/version = \"$new\"/" "$cargo_toml"
echo "Ledge $old -> $new"

# Asset names must match update::ASSET_SUFFIX in src-tauri/src/update.rs.
mac_zip="release/Ledge-$new-macos-arm64.zip"
win_exe="release/Ledge-$new-windows-x64.exe"
mkdir -p release

npm run tauri build
app=src-tauri/target/release/bundle/macos/Ledge.app
ditto -c -k --keepParent "$app" "$mac_zip"

PATH="/opt/homebrew/opt/llvm/bin:$PATH" npx tauri build --runner cargo-xwin --target x86_64-pc-windows-msvc --no-bundle
cp src-tauri/target/x86_64-pc-windows-msvc/release/edge-dock.exe "$win_exe"

git commit -qam "chore: release $new"
git tag "v$new"
git push -q origin HEAD "v$new"
gh release create "v$new" "$mac_zip" "$win_exe" --title "Ledge $new" --generate-notes

# Run the new build here too — from /Applications when Ledge is installed there (one copy, so macOS permissions stay on it).
pkill -f "Ledge.app/Contents/MacOS/edge-dock" || true
sleep 1
if [ -d /Applications/Ledge.app ]; then
  rm -rf /Applications/Ledge.app.previous
  mv /Applications/Ledge.app /Applications/Ledge.app.previous
  ditto "$app" /Applications/Ledge.app && rm -rf /Applications/Ledge.app.previous || mv /Applications/Ledge.app.previous /Applications/Ledge.app
  app=/Applications/Ledge.app
fi
open "$app"
echo "Ledge $new released on GitHub and running here"
