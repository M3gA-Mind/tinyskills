#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

engine_pattern='^(rusqlite|libsqlite3-sys|mongodb|bson|aws-sdk-s3|lapin|amq-protocol|sqlx|redb|sled|rocksdb|tinystoragedrivers-(document|kv|blob|queue|secrets|connect)|tinystoragedrivers)$'

strip_carriage_return() {
  printf '%s' "${1%$'\r'}"
}

tree_names() {
  printf '%s\n' "$1" | tr -d '\r' | awk '{print $1}'
}

# Keep the metadata parser compatible with Bash 3 on macOS.
crlf_packages=$'tinyskills\r\ntinyskills-drivers\r\n'
normalized_packages=''
while IFS= read -r package; do
  [[ -n "$package" ]] || continue
  package="$(strip_carriage_return "$package")"
  normalized_packages="${normalized_packages:+${normalized_packages}$'\n'}${package}"
done <<< "$crlf_packages"
if [[ "$normalized_packages" != $'tinyskills\ntinyskills-drivers' ]]; then
  echo "engine gate failed its CRLF package-list self-test" >&2
  exit 1
fi

if [[ "$(tree_names $'tinyskills\r\ntinystoragedrivers-api\r\n')" != $'tinyskills\ntinystoragedrivers-api' ]]; then
  echo "engine gate failed its CRLF cargo-tree self-test" >&2
  exit 1
fi

# The matcher must catch a real tree row, without rejecting public contracts
# or names that merely contain an engine package name.
sample_tree=$'tinyskills v0.5.1\n tinystoragedrivers-document v2.0.2\n'
if ! printf '%s\n' "$(tree_names "$sample_tree")" | grep -xE "$engine_pattern" >/dev/null; then
  echo "engine gate failed to detect a formatted engine dependency" >&2
  exit 1
fi
for name in tinystoragedrivers-document tinystoragedrivers-kv tinystoragedrivers-blob tinystoragedrivers-queue tinystoragedrivers-secrets tinystoragedrivers-connect tinystoragedrivers; do
  if ! printf '%s\n' "$name" | grep -xE "$engine_pattern" >/dev/null; then
    echo "engine matcher missed package: $name" >&2
    exit 1
  fi
done
for name in tinyskills tinyskills-drivers tinystoragedrivers-api tinystoragedrivers-types not-tinystoragedrivers-document tinystoragedrivers-types-extra; do
  if printf '%s\n' "$name" | grep -xE "$engine_pattern" >/dev/null; then
    echo "engine matcher rejected allowed package name: $name" >&2
    exit 1
  fi
done

# Preserve the published parser library's stronger host-independence contract:
# it must not acquire transports, HTTP clients, or repository integrations.
host_pattern='^(tinybus.*|tauri|reqwest|ureq|hyper|git2)$'
for features in default all; do
  args=(tree --locked -p tinyskills -e normal,build --prefix none)
  if [[ "$features" == all ]]; then
    args+=(--all-features)
  fi
  tree="$(cargo "${args[@]}")"
  names="$(tree_names "$tree")"
  if ! printf '%s\n' "$names" | grep -Fx tinyskills >/dev/null; then
    echo "cargo tree for tinyskills ($features features) omitted its root package" >&2
    exit 1
  fi
  forbidden="$(printf '%s\n' "$names" | grep -xE "$host_pattern" || true)"
  if [[ -n "$forbidden" ]]; then
    printf 'host integrations found in tinyskills (%s features):\n%s\n' "$features" "$forbidden" >&2
    exit 1
  fi
done

libraries="$(cargo metadata --locked --format-version 1 --no-deps | python3 -c '
import json, sys
metadata = json.load(sys.stdin)
members = set(metadata["workspace_members"])
library_kinds = {"lib", "rlib", "cdylib", "dylib", "staticlib", "proc-macro"}
names = sorted({
    package["name"]
    for package in metadata["packages"]
    if package["id"] in members
    and any(library_kinds.intersection(target["kind"]) for target in package["targets"])
})
print("\n".join(names))
')"
if [[ -z "$libraries" ]]; then
  echo "Cargo metadata found no workspace library targets" >&2
  exit 1
fi

while IFS= read -r package; do
  package="$(strip_carriage_return "$package")"
  [[ -n "$package" ]] || continue
  for features in default all; do
    args=(tree --locked -p "$package" -e normal,build --prefix none)
    if [[ "$features" == all ]]; then
      args+=(--all-features)
    fi
    tree="$(cargo "${args[@]}")"
    names="$(tree_names "$tree")"
    if ! printf '%s\n' "$names" | grep -Fx "$package" >/dev/null; then
      echo "cargo tree for $package ($features features) omitted its root package" >&2
      exit 1
    fi
    forbidden="$(printf '%s\n' "$names" | grep -xE "$engine_pattern" || true)"
    if [[ -n "$forbidden" ]]; then
      printf 'storage engines found in %s (%s features):\n%s\n' "$package" "$features" "$forbidden" >&2
      exit 1
    fi
  done
done <<< "$libraries"
