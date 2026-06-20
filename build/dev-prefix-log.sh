#!/bin/sh
# Prefix each line of stdin for readable multi-process `make dev` output.
label="${1:?usage: dev-prefix-log.sh <label>}"
while IFS= read -r line; do
  printf '[%s] %s\n' "$label" "$line"
done
