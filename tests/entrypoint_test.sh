#!/bin/sh
set -u

here="$(cd "$(dirname "$0")/.." && pwd)"
fail=0

if [ "$(id -u)" = "0" ]; then
    echo "skipping entrypoint tests: running as root bypasses file permissions"
    exit 0
fi

setup() {
    work="$(mktemp -d)"
    mkdir -p "$work/bin" "$work/data"
    cp "$here/entrypoint.sh" "$work/entrypoint.sh"
    printf '#!/bin/sh\necho started "$@"\n' > "$work/bin/route96"
    chmod 755 "$work/bin/route96" "$work/entrypoint.sh"
    printf 'listen: "0.0.0.0:8000"\nstorage_dir: "%s/data" # uploads\n' "$work" > "$work/config.yaml"
}

teardown() {
    chmod -R u+rwx "$work"
    rm -rf "$work"
}

run() {
    out="$(cd "$work" && ./entrypoint.sh "$@" 2>&1)"
    code=$?
}

expect() {
    name="$1"; want_code="$2"; want_text="$3"
    if [ "$code" != "$want_code" ] || ! printf '%s' "$out" | grep -qF -- "$want_text"; then
        echo "FAIL $name: exit $code, output:"
        printf '%s\n' "$out"
        fail=1
    else
        echo "ok   $name"
    fi
}

setup
run --config config.yaml
expect "correct setup starts" 0 "started --config config.yaml"
teardown

setup
chmod 000 "$work/config.yaml"
run --config config.yaml
expect "unreadable config exits 1" 1 "config file config.yaml is not readable"
expect "unreadable config shows chmod 640" 1 "chmod 640"
teardown

setup
chmod 555 "$work/data"
run --config config.yaml
expect "read-only data dir exits 1" 1 "data directory $work/data is not writable"
expect "read-only data dir shows chown" 1 "chown -R $(id -u):$(id -g)"
teardown

setup
chmod 555 "$work/data"
run --config="$work/config.yaml"
expect "--config= form is parsed" 1 "is not writable"
teardown

setup
rmdir "$work/data"
chmod 555 "$work"
run --config config.yaml
expect "missing data dir under read-only parent exits 1" 1 "data directory $work is not writable"
teardown

setup
chmod 555 "$work/data"
APP_STORAGE_DIR="$work/elsewhere" run --config config.yaml
expect "APP_STORAGE_DIR overrides config" 0 "started"
teardown

setup
printf '#!/bin/sh\nexit 3\n' > "$work/bin/route96"
run --config config.yaml
expect "server exit code is passed through" 3 "CRASH: Process exited with code 3"
teardown

exit $fail
