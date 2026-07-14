#!/usr/bin/env bash

set -Eeuo pipefail

input_html=${1:-}
input_config=${2:-}
output_config=${3:-}

if [[ -z $input_html || -z $input_config || -z $output_config ]]; then
  printf 'usage: %s INPUT_HTML INPUT_CONFIG OUTPUT_CONFIG\n' "$0" >&2
  exit 1
fi

for path in "$input_html" "$input_config"; do
  [[ -r $path ]] || {
    printf 'render-nginx-csp: input is not readable: %s\n' "$path" >&2
    exit 1
  }
done

inline_hashes=$(perl -MDigest::SHA=sha256_base64 -0777 -ne '
  while (/<script(?:\s[^>]*)?>(.*?)<\/script>/sg) {
    next unless length $1;
    my $hash = sha256_base64($1);
    $hash .= "=" x ((4 - length($hash) % 4) % 4);
    print "\x27sha256-$hash\x27 ";
  }
' "$input_html")

[[ -n $inline_hashes ]] || {
  printf 'render-nginx-csp: no inline bootstrap scripts found\n' >&2
  exit 1
}
[[ $inline_hashes != *'|'* ]] || {
  printf 'render-nginx-csp: generated hash contains an invalid delimiter\n' >&2
  exit 1
}

sed "s|__INLINE_SCRIPT_HASHES__|${inline_hashes% }|" "$input_config" >"$output_config"

grep -q "sha256-" "$output_config"
if grep -q "__INLINE_SCRIPT_HASHES__\|'unsafe-inline'" "$output_config"; then
  printf 'render-nginx-csp: unsafe or unresolved CSP configuration\n' >&2
  exit 1
fi
