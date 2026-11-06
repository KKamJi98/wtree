#!/usr/bin/env bash
# Download a checksummed release; record only inside a network-isolated container.
set -euo pipefail
cd "$(dirname "$0")/../.."
version=2.0.1
for tool in docker vhs curl shasum tar gifsicle; do
  command -v "$tool" >/dev/null || { printf 'Missing tool: %s\n' "$tool" >&2; exit 1; }
done
case "$(docker info --format '{{.Architecture}}')" in
  aarch64|arm64) target=aarch64-unknown-linux-musl ;;
  x86_64|amd64) target=x86_64-unknown-linux-musl ;;
  *) printf 'Unsupported container architecture\n' >&2; exit 1 ;;
esac
scratch="$(mktemp -d "${TMPDIR:?Set TMPDIR to a scratch directory}/wtree-demo.XXXXXX")"
trap 'rm -rf "$scratch"' EXIT
archive="wt-${version}-${target}.tar.gz"
base="https://github.com/KKamJi98/wtree/releases/download/v${version}"
curl --fail --location --silent --show-error "$base/$archive" -o "$scratch/$archive"
curl --fail --location --silent --show-error "$base/$archive.sha256" -o "$scratch/$archive.sha256"
(cd "$scratch" && shasum -a 256 -c "$archive.sha256")
tar -xzf "$scratch/$archive" -C "$scratch" wt
chmod +x "$scratch/wt"
cp docs/demo/Dockerfile docs/demo/setup.sh docs/demo/start.sh "$scratch/"
docker build --tag wtree-readme-demo:local "$scratch"
vhs docs/demo/wtree.tape
gifsicle -O3 --lossy=40 docs/demo/wtree.gif -o "$scratch/optimized.gif"
cp "$scratch/optimized.gif" docs/demo/wtree.gif
