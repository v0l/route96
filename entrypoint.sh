#!/bin/sh

config="config.yaml"
prev=""
for arg in "$@"; do
    case "$arg" in
        --config=*) config="${arg#--config=}" ;;
    esac
    if [ "$prev" = "--config" ]; then
        config="$arg"
    fi
    prev="$arg"
done
case "$config" in
    /*) ;;
    *) config="$(pwd)/$config" ;;
esac

uid="$(id -u)"
gid="$(id -g)"
problems=""
fixes=""

if [ -e "$config" ] && [ ! -r "$config" ]; then
    owner="$(stat -c '%u:%g mode %a' "$config" 2>/dev/null)"
    problems="${problems}  config file $config is not readable (owner $owner, process uid $uid)
"
    fixes="${fixes}  chown $uid:$gid <host path of $config> && chmod 640 <host path of $config>
"
fi

data_dir="${APP_STORAGE_DIR:-}"
if [ -z "$data_dir" ] && [ -r "$config" ]; then
    data_dir="$(sed -n "s/^storage_dir:[[:space:]]*[\"']\{0,1\}\([^\"'#]*\).*/\1/p" "$config" | head -n 1 | sed 's/[[:space:]]*$//')"
fi
if [ -z "$data_dir" ] && [ ! -r "$config" ] && [ -d /app/data ]; then
    data_dir="/app/data"
fi

if [ -n "$data_dir" ]; then
    probe="$data_dir"
    while [ ! -e "$probe" ]; do
        probe="$(dirname "$probe")"
    done
    if [ ! -d "$probe" ] || [ ! -w "$probe" ] || [ ! -x "$probe" ]; then
        owner="$(stat -c '%u:%g mode %a' "$probe" 2>/dev/null)"
        problems="${problems}  data directory $probe is not writable (owner $owner, process uid $uid)
"
        fixes="${fixes}  chown -R $uid:$gid <host path of $probe>
  or for a named volume: docker run --rm -u 0 -v <volume>:$probe --entrypoint chown voidic/route96 -R $uid:$gid $probe
"
    fi
fi

if [ -n "$problems" ]; then
    printf 'route96 cannot start as uid %s:\n%s' "$uid" "$problems" >&2
    printf 'Images since 17 Nov run as uid 10001 instead of root. Fix the ownership on the host:\n%s' "$fixes" >&2
    printf 'See "Upgrading from images before 17 Nov" in the README.\n' >&2
    exit 1
fi

EXIT_CODE=0
./bin/route96 "$@" || EXIT_CODE=$?

if [ $EXIT_CODE -ne 0 ]; then
    case $EXIT_CODE in
        139) echo "CRASH: Process killed by SIGSEGV (segmentation fault)" >&2 ;;
        134) echo "CRASH: Process killed by SIGABRT (abort)" >&2 ;;
        136) echo "CRASH: Process killed by SIGFPE (floating point exception)" >&2 ;;
        137) echo "CRASH: Process killed by SIGKILL (OOM or external kill)" >&2 ;;
        *)   echo "CRASH: Process exited with code $EXIT_CODE" >&2 ;;
    esac
fi

exit $EXIT_CODE
