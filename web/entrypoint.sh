#!/bin/sh
set -e

if [ "${1:0:1}" = '-' ]; then
    set -- diving "$@"
fi

exec "$@"