#!/bin/sh
set -u
apt_opts="-o Acquire::Retries=3 -o Acquire::http::Timeout=30 -o Acquire::https::Timeout=30"
for attempt in 1 2 3; do
  if sudo timeout 180 apt-get $apt_opts update && sudo timeout 240 apt-get $apt_opts install -y --no-install-recommends "$@"; then
    exit 0
  fi
  echo "apt attempt $attempt failed, retrying" >&2
  sleep $((attempt * 10))
done
exit 1
